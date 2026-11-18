mod ingest;
mod key;
mod project;

pub use ingest::{IngestCount, Rejection, Tally};
pub use key::{IngestKey, KeyHash, KeyId, KeyLabel};
pub use project::{
    Environment, EnvironmentName, InvalidProject, NotifyAddress, Project, ProjectName, ProjectSettings,
};

/// What the audit log records of this context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    ProjectCreate,
    ProjectUpdate,
    ProjectDelete,
    EnvironmentAdd,
    EnvironmentRemove,
    KeyCreate,
    KeyRevoke,
}

impl AuditAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectCreate => "project.create",
            Self::ProjectUpdate => "project.update",
            Self::ProjectDelete => "project.delete",
            Self::EnvironmentAdd => "environment.add",
            Self::EnvironmentRemove => "environment.remove",
            Self::KeyCreate => "key.create",
            Self::KeyRevoke => "key.revoke",
        }
    }
}
