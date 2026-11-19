use std::sync::Arc;

use logdock_kernel::{Actor, AuditEntry, EnvironmentId, Outcome, ProjectId, RequestContext, Secret};

use super::auth::KeyCache;
use super::{ProjectError, ProjectPorts};
use crate::domain::{AuditAction, IngestKey, KeyId, KeyLabel};

/// A key just made: the secret is shown once, then only its prefix.
#[derive(Debug)]
pub struct CreatedKey {
    pub key: IngestKey,
    pub secret: Secret,
}

/// Query: a project's keys, newest first.
#[derive(Clone)]
pub struct ListKeys {
    ports: Arc<ProjectPorts>,
}

impl ListKeys {
    pub(crate) fn new(ports: Arc<ProjectPorts>) -> Self {
        Self { ports }
    }

    pub async fn execute(&self, project: ProjectId) -> Result<Vec<IngestKey>, ProjectError> {
        self.ports.projects.find(project).await?.ok_or(ProjectError::NotFound)?;
        Ok(self.ports.keys.list(project).await?)
    }
}

/// Command: a new key for a project, or for one of its stages.
#[derive(Clone)]
pub struct CreateKey {
    ports: Arc<ProjectPorts>,
}

impl CreateKey {
    pub(crate) fn new(ports: Arc<ProjectPorts>) -> Self {
        Self { ports }
    }

    pub async fn execute(
        &self,
        actor: &Actor,
        project: ProjectId,
        environment: Option<EnvironmentId>,
        label: &str,
        request: &RequestContext,
    ) -> Result<CreatedKey, ProjectError> {
        let p = &self.ports;
        let found = p.projects.find(project).await?.ok_or(ProjectError::NotFound)?;
        let stage = match environment {
            Some(id) => found.environment(id).ok_or(ProjectError::EnvironmentNotFound)?.name.as_str(),
            None => "*",
        };
        let label = KeyLabel::parse(label)?;
        let secret = p.tokens.new_key();
        let prefix: String = secret.expose().chars().take(IngestKey::PREFIX_LEN).collect();
        let hash = p.tokens.hash(secret.expose());
        let key = p.keys.create(project, environment, label.as_ref(), &prefix, &hash, p.clock.now()).await?;
        let target = format!("{}/{stage} {prefix}", found.name);
        let entry =
            AuditEntry::new(p.clock.now(), AuditAction::KeyCreate.as_str(), Outcome::Success, request);
        p.audit.record(entry.by(actor).target(target)).await?;
        Ok(CreatedKey { key, secret })
    }
}

/// Command: a key stops working at once.
#[derive(Clone)]
pub struct RevokeKey {
    ports: Arc<ProjectPorts>,
    cache: KeyCache,
}

impl RevokeKey {
    pub(crate) fn new(ports: Arc<ProjectPorts>, cache: KeyCache) -> Self {
        Self { ports, cache }
    }

    pub async fn execute(
        &self,
        actor: &Actor,
        project: ProjectId,
        id: KeyId,
        request: &RequestContext,
    ) -> Result<(), ProjectError> {
        let p = &self.ports;
        let found = p.projects.find(project).await?.ok_or(ProjectError::NotFound)?;
        let key =
            p.keys.list(project).await?.into_iter().find(|k| k.id == id).ok_or(ProjectError::KeyNotFound)?;
        if key.is_active() {
            p.keys.revoke(id, p.clock.now()).await?;
        }
        self.cache.clear().await;
        let target = format!("{} {}", found.name, key.prefix);
        let entry =
            AuditEntry::new(p.clock.now(), AuditAction::KeyRevoke.as_str(), Outcome::Success, request);
        p.audit.record(entry.by(actor).target(target)).await?;
        Ok(())
    }
}
