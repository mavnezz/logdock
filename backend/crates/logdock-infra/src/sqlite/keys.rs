use async_trait::async_trait;
use logdock_kernel::{EnvironmentId, PortError, ProjectId, Secret, Timestamp};
use logdock_projects::application::ports::{KeyRepository, KeyTokens};
use logdock_projects::domain::{IngestKey, KeyHash, KeyId, KeyLabel};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::Database;
use crate::random::{hex, random_bytes, sha256_hex};

/// [`KeyRepository`] on the `ingest_keys` table.
#[derive(Debug, Clone)]
pub struct SqliteKeys {
    db: Database,
}

impl SqliteKeys {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn port(error: sqlx::Error) -> PortError {
    PortError::new("ingest keys", error)
}

fn key(row: &SqliteRow) -> IngestKey {
    let label: Option<String> = row.get("label");
    IngestKey {
        id: KeyId::new(row.get("id")),
        project: ProjectId::new(row.get("project_id")),
        environment: row.get::<Option<i64>, _>("environment_id").map(EnvironmentId::new),
        label: label.and_then(|l| KeyLabel::parse(&l).ok().flatten()),
        prefix: row.get("prefix"),
        created_at: Timestamp::from_unix_micros(row.get("created_at")),
        last_used_at: row.get::<Option<i64>, _>("last_used_at").map(Timestamp::from_unix_micros),
        revoked_at: row.get::<Option<i64>, _>("revoked_at").map(Timestamp::from_unix_micros),
    }
}

#[async_trait]
impl KeyRepository for SqliteKeys {
    async fn list(&self, project: ProjectId) -> Result<Vec<IngestKey>, PortError> {
        let rows = sqlx::query("SELECT * FROM ingest_keys WHERE project_id = ? ORDER BY id DESC")
            .bind(project.get())
            .fetch_all(self.db.pool())
            .await
            .map_err(port)?;
        Ok(rows.iter().map(key).collect())
    }

    async fn create(
        &self,
        project: ProjectId,
        environment: Option<EnvironmentId>,
        label: Option<&KeyLabel>,
        prefix: &str,
        hash: &KeyHash,
        at: Timestamp,
    ) -> Result<IngestKey, PortError> {
        let row = sqlx::query(
            "INSERT INTO ingest_keys (project_id, environment_id, label, prefix, hash, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(project.get())
        .bind(environment.map(EnvironmentId::get))
        .bind(label.map(KeyLabel::as_str))
        .bind(prefix)
        .bind(hash.as_str())
        .bind(at.unix_micros())
        .fetch_one(self.db.pool())
        .await
        .map_err(port)?;
        Ok(key(&row))
    }

    async fn find_by_hash(&self, hash: &KeyHash) -> Result<Option<IngestKey>, PortError> {
        let row = sqlx::query("SELECT * FROM ingest_keys WHERE hash = ?")
            .bind(hash.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(port)?;
        Ok(row.as_ref().map(key))
    }

    async fn revoke(&self, id: KeyId, at: Timestamp) -> Result<(), PortError> {
        sqlx::query("UPDATE ingest_keys SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(at.unix_micros())
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn touch(&self, id: KeyId, at: Timestamp) -> Result<(), PortError> {
        sqlx::query("UPDATE ingest_keys SET last_used_at = ? WHERE id = ?")
            .bind(at.unix_micros())
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }
}

/// Keys of 32 hex characters (128 bits), as Sentry's DSN keys look, so
/// every SDK takes them; stored as their SHA-256.
#[derive(Debug, Default)]
pub struct RandomKeys;

impl KeyTokens for RandomKeys {
    fn new_key(&self) -> Secret {
        Secret::new(hex(&random_bytes::<16>()))
    }

    fn hash(&self, key: &str) -> KeyHash {
        KeyHash::new(sha256_hex(key))
    }
}
