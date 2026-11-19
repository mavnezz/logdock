use async_trait::async_trait;
use logdock_kernel::redaction::RedactionRules;
use logdock_kernel::{EnvironmentId, PortError, ProjectId, Timestamp};
use logdock_projects::application::ports::ProjectRepository;
use logdock_projects::domain::{
    Environment, EnvironmentName, NotifyAddress, Project, ProjectName, ProjectSettings,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::Database;

/// [`ProjectRepository`] on the `projects` and `environments` tables.
#[derive(Debug, Clone)]
pub struct SqliteProjects {
    db: Database,
}

impl SqliteProjects {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("projects", error)
}

/// [`ProjectSettings`] as stored; every field defaults, so settings written
/// by an older release still read.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct SettingsJson {
    notify: Vec<String>,
    notify_new_issues: bool,
    notify_regressions: bool,
    notify_monitors: bool,
    redaction_extra_keys: Vec<String>,
    redaction_allowed_keys: Option<Vec<String>>,
    scrub_emails: bool,
    scrub_ip_addresses: bool,
}

impl Default for SettingsJson {
    fn default() -> Self {
        Self::from(&ProjectSettings::default())
    }
}

impl From<&ProjectSettings> for SettingsJson {
    fn from(settings: &ProjectSettings) -> Self {
        Self {
            notify: settings.notify.iter().map(|a| a.as_str().to_owned()).collect(),
            notify_new_issues: settings.notify_new_issues,
            notify_regressions: settings.notify_regressions,
            notify_monitors: settings.notify_monitors,
            redaction_extra_keys: settings.redaction.extra_keys.clone(),
            redaction_allowed_keys: settings.redaction.allowed_keys.clone(),
            scrub_emails: settings.redaction.scrub_emails,
            scrub_ip_addresses: settings.redaction.scrub_ip_addresses,
        }
    }
}

impl SettingsJson {
    fn into_settings(self) -> ProjectSettings {
        ProjectSettings {
            notify: self.notify.iter().filter_map(|a| NotifyAddress::parse(a).ok()).collect(),
            notify_new_issues: self.notify_new_issues,
            notify_regressions: self.notify_regressions,
            notify_monitors: self.notify_monitors,
            redaction: RedactionRules {
                extra_keys: self.redaction_extra_keys,
                allowed_keys: self.redaction_allowed_keys,
                scrub_emails: self.scrub_emails,
                scrub_ip_addresses: self.scrub_ip_addresses,
            },
        }
    }
}

impl SqliteProjects {
    async fn load(&self, filter: Option<ProjectId>) -> Result<Vec<Project>, PortError> {
        let rows = sqlx::query(
            "SELECT id, name, settings, created_at FROM projects WHERE ?1 IS NULL OR id = ?1 ORDER BY name COLLATE NOCASE",
        )
        .bind(filter.map(ProjectId::get))
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        let environments = sqlx::query(
            "SELECT id, project_id, name, created_at FROM environments WHERE ?1 IS NULL OR project_id = ?1 \
             ORDER BY project_id, id",
        )
        .bind(filter.map(ProjectId::get))
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        rows.iter()
            .map(|row| {
                let id: i64 = row.get("id");
                let settings: String = row.get("settings");
                let settings: SettingsJson = serde_json::from_str(&settings).unwrap_or_default();
                Ok(Project {
                    id: ProjectId::new(id),
                    name: ProjectName::parse(row.get("name")).map_err(port)?,
                    environments: environments
                        .iter()
                        .filter(|e| e.get::<i64, _>("project_id") == id)
                        .filter_map(|e| {
                            Some(Environment {
                                id: EnvironmentId::new(e.get("id")),
                                name: EnvironmentName::parse(e.get("name")).ok()?,
                                created_at: Timestamp::from_unix_micros(e.get("created_at")),
                            })
                        })
                        .collect(),
                    settings: settings.into_settings(),
                    created_at: Timestamp::from_unix_micros(row.get("created_at")),
                })
            })
            .collect()
    }
}

#[async_trait]
impl ProjectRepository for SqliteProjects {
    async fn list(&self) -> Result<Vec<Project>, PortError> {
        self.load(None).await
    }

    async fn find(&self, id: ProjectId) -> Result<Option<Project>, PortError> {
        Ok(self.load(Some(id)).await?.pop())
    }

    async fn create(
        &self,
        name: &ProjectName,
        environments: &[EnvironmentName],
        at: Timestamp,
    ) -> Result<Option<Project>, PortError> {
        let mut tx = self.db.write().await.map_err(port)?;
        let settings = serde_json::to_string(&SettingsJson::default()).map_err(port)?;
        let id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO projects (name, settings, created_at) VALUES (?, ?, ?) ON CONFLICT (name) DO NOTHING RETURNING id",
        )
        .bind(name.as_str())
        .bind(settings)
        .bind(at.unix_micros())
        .fetch_optional(&mut *tx)
        .await
        .map_err(port)?;
        let Some(id) = id else { return Ok(None) };
        for environment in environments {
            sqlx::query("INSERT INTO environments (project_id, name, created_at) VALUES (?, ?, ?)")
                .bind(id)
                .bind(environment.as_str())
                .bind(at.unix_micros())
                .execute(&mut *tx)
                .await
                .map_err(port)?;
        }
        tx.commit().await.map_err(port)?;
        self.find(ProjectId::new(id)).await
    }

    async fn rename(&self, id: ProjectId, name: &ProjectName) -> Result<bool, PortError> {
        let result = sqlx::query("UPDATE projects SET name = ? WHERE id = ?")
            .bind(name.as_str())
            .bind(id.get())
            .execute(self.db.pool())
            .await;
        match result {
            Ok(_) => Ok(true),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => Ok(false),
            Err(error) => Err(port(error)),
        }
    }

    async fn update_settings(&self, id: ProjectId, settings: &ProjectSettings) -> Result<(), PortError> {
        let json = serde_json::to_string(&SettingsJson::from(settings)).map_err(port)?;
        sqlx::query("UPDATE projects SET settings = ? WHERE id = ?")
            .bind(json)
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn delete(&self, id: ProjectId) -> Result<(), PortError> {
        let mut tx = self.db.write().await.map_err(port)?;
        // What the segments' writers count is kept beside them, without a
        // foreign key: it goes with the project here.
        for statement in [
            "DELETE FROM log_rollups WHERE project_id = ?",
            "DELETE FROM log_sources WHERE project_id = ?",
            "DELETE FROM projects WHERE id = ?",
        ] {
            sqlx::query(statement).bind(id.get()).execute(&mut *tx).await.map_err(port)?;
        }
        tx.commit().await.map_err(port)
    }

    async fn add_environment(
        &self,
        id: ProjectId,
        name: &EnvironmentName,
        at: Timestamp,
    ) -> Result<Option<Environment>, PortError> {
        let env_id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO environments (project_id, name, created_at) VALUES (?, ?, ?) \
             ON CONFLICT (project_id, name) DO NOTHING RETURNING id",
        )
        .bind(id.get())
        .bind(name.as_str())
        .bind(at.unix_micros())
        .fetch_optional(self.db.pool())
        .await
        .map_err(port)?;
        Ok(env_id.map(|env_id| Environment {
            id: EnvironmentId::new(env_id),
            name: name.clone(),
            created_at: at,
        }))
    }

    async fn remove_environment(&self, id: ProjectId, environment: EnvironmentId) -> Result<(), PortError> {
        sqlx::query("DELETE FROM environments WHERE id = ? AND project_id = ?")
            .bind(environment.get())
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }
}
