use std::sync::Arc;

use logdock_kernel::{Actor, AuditEntry, Day, EnvironmentId, Outcome, ProjectId, RequestContext};

use super::auth::KeyCache;
use super::{ProjectError, ProjectPorts};
use crate::domain::{
    AuditAction, Environment, EnvironmentName, IngestCount, Project, ProjectName, ProjectSettings,
};

async fn audit(
    ports: &ProjectPorts,
    action: AuditAction,
    actor: &Actor,
    target: &str,
    request: &RequestContext,
) -> Result<(), ProjectError> {
    let entry = AuditEntry::new(ports.clock.now(), action.as_str(), Outcome::Success, request)
        .by(actor)
        .target(target);
    ports.audit.record(entry).await?;
    Ok(())
}

async fn find(ports: &ProjectPorts, id: ProjectId) -> Result<Project, ProjectError> {
    ports.projects.find(id).await?.ok_or(ProjectError::NotFound)
}

/// Query: all projects with their stages.
#[derive(Clone)]
pub struct ListProjects {
    ports: Arc<ProjectPorts>,
}

impl ListProjects {
    pub(crate) fn new(ports: Arc<ProjectPorts>) -> Self {
        Self { ports }
    }

    pub async fn execute(&self) -> Result<Vec<Project>, ProjectError> {
        Ok(self.ports.projects.list().await?)
    }
}

/// Query: one project.
#[derive(Clone)]
pub struct GetProject {
    ports: Arc<ProjectPorts>,
}

impl GetProject {
    pub(crate) fn new(ports: Arc<ProjectPorts>) -> Self {
        Self { ports }
    }

    pub async fn execute(&self, id: ProjectId) -> Result<Project, ProjectError> {
        find(&self.ports, id).await
    }
}

/// Command: a new project with its first stages.
#[derive(Clone)]
pub struct CreateProject {
    ports: Arc<ProjectPorts>,
}

impl CreateProject {
    pub(crate) fn new(ports: Arc<ProjectPorts>) -> Self {
        Self { ports }
    }

    pub async fn execute(
        &self,
        actor: &Actor,
        name: &str,
        environments: &[String],
        request: &RequestContext,
    ) -> Result<Project, ProjectError> {
        let name = ProjectName::parse(name)?;
        let mut names: Vec<EnvironmentName> = Vec::new();
        for environment in environments {
            let environment = EnvironmentName::parse(environment)?;
            if !names.contains(&environment) {
                names.push(environment);
            }
        }
        let p = &self.ports;
        let project =
            p.projects.create(&name, &names, p.clock.now()).await?.ok_or(ProjectError::NameTaken)?;
        audit(p, AuditAction::ProjectCreate, actor, name.as_str(), request).await?;
        Ok(project)
    }
}

#[derive(Debug, Default)]
pub struct ProjectChanges {
    pub name: Option<String>,
    pub settings: Option<ProjectSettings>,
}

/// Command: renames a project or changes its settings.
#[derive(Clone)]
pub struct UpdateProject {
    ports: Arc<ProjectPorts>,
    cache: KeyCache,
}

impl UpdateProject {
    pub(crate) fn new(ports: Arc<ProjectPorts>, cache: KeyCache) -> Self {
        Self { ports, cache }
    }

    pub async fn execute(
        &self,
        actor: &Actor,
        id: ProjectId,
        changes: ProjectChanges,
        request: &RequestContext,
    ) -> Result<Project, ProjectError> {
        let p = &self.ports;
        let project = find(p, id).await?;
        if let Some(name) = changes.name {
            let name = ProjectName::parse(&name)?;
            if name != project.name && !p.projects.rename(id, &name).await? {
                return Err(ProjectError::NameTaken);
            }
        }
        if let Some(settings) = changes.settings {
            settings.validate()?;
            p.projects.update_settings(id, &settings).await?;
        }
        self.cache.clear().await;
        let project = find(p, id).await?;
        audit(p, AuditAction::ProjectUpdate, actor, project.name.as_str(), request).await?;
        Ok(project)
    }
}

/// Command: deletes a project with its stages, keys and issues.
#[derive(Clone)]
pub struct DeleteProject {
    ports: Arc<ProjectPorts>,
    cache: KeyCache,
}

impl DeleteProject {
    pub(crate) fn new(ports: Arc<ProjectPorts>, cache: KeyCache) -> Self {
        Self { ports, cache }
    }

    pub async fn execute(
        &self,
        actor: &Actor,
        id: ProjectId,
        request: &RequestContext,
    ) -> Result<(), ProjectError> {
        let p = &self.ports;
        let project = find(p, id).await?;
        p.projects.delete(id).await?;
        self.cache.clear().await;
        audit(p, AuditAction::ProjectDelete, actor, project.name.as_str(), request).await
    }
}

/// Command: adds a stage to a project.
#[derive(Clone)]
pub struct AddEnvironment {
    ports: Arc<ProjectPorts>,
    cache: KeyCache,
}

impl AddEnvironment {
    pub(crate) fn new(ports: Arc<ProjectPorts>, cache: KeyCache) -> Self {
        Self { ports, cache }
    }

    pub async fn execute(
        &self,
        actor: &Actor,
        id: ProjectId,
        name: &str,
        request: &RequestContext,
    ) -> Result<Environment, ProjectError> {
        let p = &self.ports;
        let project = find(p, id).await?;
        let name = EnvironmentName::parse(name)?;
        let environment = p
            .projects
            .add_environment(id, &name, p.clock.now())
            .await?
            .ok_or(ProjectError::EnvironmentTaken)?;
        self.cache.clear().await;
        let target = format!("{}/{}", project.name, name);
        audit(p, AuditAction::EnvironmentAdd, actor, &target, request).await?;
        Ok(environment)
    }
}

/// Command: removes a stage that has no active keys.
#[derive(Clone)]
pub struct RemoveEnvironment {
    ports: Arc<ProjectPorts>,
    cache: KeyCache,
}

impl RemoveEnvironment {
    pub(crate) fn new(ports: Arc<ProjectPorts>, cache: KeyCache) -> Self {
        Self { ports, cache }
    }

    pub async fn execute(
        &self,
        actor: &Actor,
        id: ProjectId,
        environment: EnvironmentId,
        request: &RequestContext,
    ) -> Result<(), ProjectError> {
        let p = &self.ports;
        let project = find(p, id).await?;
        let env = project.environment(environment).ok_or(ProjectError::EnvironmentNotFound)?;
        let keys = p.keys.list(id).await?;
        if keys.iter().any(|key| key.environment == Some(environment) && key.is_active()) {
            return Err(ProjectError::EnvironmentInUse);
        }
        p.projects.remove_environment(id, environment).await?;
        self.cache.clear().await;
        let target = format!("{}/{}", project.name, env.name);
        audit(p, AuditAction::EnvironmentRemove, actor, &target, request).await
    }
}

/// Query: what ingest accepted and refused for a project, per day and key.
#[derive(Clone)]
pub struct GetIngestCounts {
    ports: Arc<ProjectPorts>,
}

impl GetIngestCounts {
    pub(crate) fn new(ports: Arc<ProjectPorts>) -> Self {
        Self { ports }
    }

    /// The last `days` days including today.
    pub async fn execute(&self, id: ProjectId, days: u32) -> Result<Vec<IngestCount>, ProjectError> {
        let p = &self.ports;
        find(p, id).await?;
        let today = p.clock.now().day();
        let since = Day::from_days(today.days() - i64::from(days.clamp(1, 90)) + 1);
        Ok(p.counter.counts(id, since).await?)
    }
}

/// Command: sends a test e-mail to the project's addresses, so the SMTP
/// settings and the list can be checked before an issue needs them.
#[derive(Clone)]
pub struct SendTestMail {
    ports: Arc<ProjectPorts>,
}

impl SendTestMail {
    pub(crate) fn new(ports: Arc<ProjectPorts>) -> Self {
        Self { ports }
    }

    pub async fn execute(&self, id: ProjectId) -> Result<usize, ProjectError> {
        let p = &self.ports;
        let project = find(p, id).await?;
        if !p.mailer.configured() {
            return Err(ProjectError::MailNotConfigured);
        }
        let to: Vec<String> = project.settings.notify.iter().map(|a| a.as_str().to_owned()).collect();
        if to.is_empty() {
            return Err(ProjectError::NoRecipients);
        }
        let count = to.len();
        let mail = logdock_kernel::Mail {
            to,
            subject: format!("[Logdock] Test e-mail for {}", project.name),
            text: format!(
                "This is a test e-mail from Logdock for the project {}.\n\nNew and returning issues will be sent to this address.\n\n-- \nLogdock\n",
                project.name
            ),
        };
        p.mailer.send(mail).await.map_err(|error| ProjectError::MailFailed(chain(&error)))?;
        Ok(count)
    }
}

/// An error with its causes, as one line.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
