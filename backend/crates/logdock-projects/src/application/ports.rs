//! What this context needs from the outside world.

use async_trait::async_trait;
use logdock_kernel::{Day, EnvironmentId, PortError, ProjectId, Secret, Timestamp};

use crate::domain::{
    Environment, EnvironmentName, IngestCount, IngestKey, KeyHash, KeyId, KeyLabel, Project, ProjectName,
    ProjectSettings, Tally,
};

#[async_trait]
pub trait ProjectRepository: Send + Sync {
    /// All projects with their stages, ordered by name.
    async fn list(&self) -> Result<Vec<Project>, PortError>;

    async fn find(&self, id: ProjectId) -> Result<Option<Project>, PortError>;

    /// `None` if the name is taken.
    async fn create(
        &self,
        name: &ProjectName,
        environments: &[EnvironmentName],
        at: Timestamp,
    ) -> Result<Option<Project>, PortError>;

    /// `false` if the name is taken by another project.
    async fn rename(&self, id: ProjectId, name: &ProjectName) -> Result<bool, PortError>;

    async fn update_settings(&self, id: ProjectId, settings: &ProjectSettings) -> Result<(), PortError>;

    /// Deletes the project with its stages, keys, issues and counts. Its
    /// logs stay in their day segments until they expire, unseen.
    async fn delete(&self, id: ProjectId) -> Result<(), PortError>;

    /// `None` if the project has a stage of that name.
    async fn add_environment(
        &self,
        id: ProjectId,
        name: &EnvironmentName,
        at: Timestamp,
    ) -> Result<Option<Environment>, PortError>;

    async fn remove_environment(&self, id: ProjectId, environment: EnvironmentId) -> Result<(), PortError>;
}

#[async_trait]
pub trait KeyRepository: Send + Sync {
    /// The project's keys, newest first.
    async fn list(&self, project: ProjectId) -> Result<Vec<IngestKey>, PortError>;

    /// `environment`: the one stage the key sends to; `None` for the whole
    /// project.
    async fn create(
        &self,
        project: ProjectId,
        environment: Option<EnvironmentId>,
        label: Option<&KeyLabel>,
        prefix: &str,
        hash: &KeyHash,
        at: Timestamp,
    ) -> Result<IngestKey, PortError>;

    async fn find_by_hash(&self, hash: &KeyHash) -> Result<Option<IngestKey>, PortError>;

    async fn revoke(&self, id: KeyId, at: Timestamp) -> Result<(), PortError>;

    async fn touch(&self, id: KeyId, at: Timestamp) -> Result<(), PortError>;
}

/// New ingest keys and their hashes.
pub trait KeyTokens: Send + Sync {
    /// A new key: random, in characters a DSN can carry.
    fn new_key(&self) -> Secret;

    fn hash(&self, key: &str) -> KeyHash;
}

/// Counts of what ingest accepted and refused.
#[async_trait]
pub trait IngestCounter: Send + Sync {
    /// Adds a request's tally; may be kept in memory for a while.
    async fn record(&self, project: ProjectId, key: Option<KeyId>, tally: &Tally, at: Timestamp);

    /// Counts per day and key since `since`, newest first.
    async fn counts(&self, project: ProjectId, since: Day) -> Result<Vec<IngestCount>, PortError>;
}
