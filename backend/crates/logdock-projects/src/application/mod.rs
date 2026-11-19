//! Use cases of the Projects context and the ports they need.

mod auth;
mod keys;
pub mod ports;
mod projects;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use logdock_kernel::{AuditLog, Clock, Mailer, PortError};

use crate::domain::InvalidProject;
pub use auth::{
    AuthenticateKey, DEFAULT_STAGE, IngestScope, KeyCache, MAX_STAGES, RecordIngest, ResolveStage, Stage,
    StageFor,
};
pub use keys::{CreateKey, CreatedKey, ListKeys, RevokeKey};
use ports::{IngestCounter, KeyRepository, KeyTokens, ProjectRepository};
pub use projects::{
    AddEnvironment, CreateProject, DeleteProject, GetIngestCounts, GetProject, ListProjects, ProjectChanges,
    RemoveEnvironment, SendTestMail, UpdateProject,
};

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error(transparent)]
    Invalid(#[from] InvalidProject),
    #[error("a project of that name exists")]
    NameTaken,
    #[error("there is no such project")]
    NotFound,
    #[error("the project has no such stage")]
    EnvironmentNotFound,
    #[error("the project has a stage of that name")]
    EnvironmentTaken,
    #[error("the stage still has keys; revoke them first")]
    EnvironmentInUse,
    #[error("there is no such key")]
    KeyNotFound,
    #[error("no SMTP server is configured")]
    MailNotConfigured,
    #[error("the project has no addresses to notify")]
    NoRecipients,
    /// The mail server refused or could not be reached; what it said.
    #[error("the e-mail could not be sent: {0}")]
    MailFailed(String),
    #[error(transparent)]
    Port(#[from] PortError),
}

/// Adapters for every port of this context, plugged in by the composition
/// root.
#[derive(Clone)]
pub struct ProjectPorts {
    pub projects: Arc<dyn ProjectRepository>,
    pub keys: Arc<dyn KeyRepository>,
    pub tokens: Arc<dyn KeyTokens>,
    pub counter: Arc<dyn IngestCounter>,
    pub audit: Arc<dyn AuditLog>,
    pub mailer: Arc<dyn Mailer>,
    pub clock: Arc<dyn Clock>,
}

/// All use cases of this context.
#[derive(Clone)]
pub struct ProjectUseCases {
    pub list: ListProjects,
    pub get: GetProject,
    pub create: CreateProject,
    pub update: UpdateProject,
    pub delete: DeleteProject,
    pub add_environment: AddEnvironment,
    pub remove_environment: RemoveEnvironment,
    pub list_keys: ListKeys,
    pub create_key: CreateKey,
    pub revoke_key: RevokeKey,
    pub authenticate_key: AuthenticateKey,
    pub resolve_stage: ResolveStage,
    pub record_ingest: RecordIngest,
    pub ingest_counts: GetIngestCounts,
    pub test_mail: SendTestMail,
}

impl ProjectUseCases {
    pub fn new(ports: ProjectPorts) -> Self {
        let ports = Arc::new(ports);
        let cache = KeyCache::default();
        Self {
            list: ListProjects::new(Arc::clone(&ports)),
            get: GetProject::new(Arc::clone(&ports)),
            create: CreateProject::new(Arc::clone(&ports)),
            update: UpdateProject::new(Arc::clone(&ports), cache.clone()),
            delete: DeleteProject::new(Arc::clone(&ports), cache.clone()),
            add_environment: AddEnvironment::new(Arc::clone(&ports), cache.clone()),
            remove_environment: RemoveEnvironment::new(Arc::clone(&ports), cache.clone()),
            list_keys: ListKeys::new(Arc::clone(&ports)),
            create_key: CreateKey::new(Arc::clone(&ports)),
            revoke_key: RevokeKey::new(Arc::clone(&ports), cache.clone()),
            authenticate_key: AuthenticateKey::new(Arc::clone(&ports), cache.clone()),
            resolve_stage: ResolveStage::new(Arc::clone(&ports), cache),
            record_ingest: RecordIngest::new(Arc::clone(&ports)),
            ingest_counts: GetIngestCounts::new(Arc::clone(&ports)),
            test_mail: SendTestMail::new(ports),
        }
    }
}
