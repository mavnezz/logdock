//! What this context needs from the outside world.

use async_trait::async_trait;
use logdock_kernel::{EnvironmentId, PortError, ProjectId, Scope, Timestamp};

use crate::domain::{CheckIn, Monitor, MonitorConfig, MonitorId};

/// Which monitors a list shows. Empty lists mean "all".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MonitorQuery {
    pub projects: Vec<ProjectId>,
    pub environments: Vec<EnvironmentId>,
}

#[async_trait]
pub trait MonitorRepository: Send + Sync {
    async fn find(&self, scope: Scope, slug: &str) -> Result<Option<Monitor>, PortError>;

    /// The monitor of `slug`, created with `config` if there is none.
    async fn create(
        &self,
        scope: Scope,
        slug: &str,
        config: Option<&MonitorConfig>,
        at: Timestamp,
    ) -> Result<Monitor, PortError>;

    /// Stores its configuration, health and times.
    async fn save(&self, monitor: &Monitor) -> Result<(), PortError>;

    async fn get(&self, id: MonitorId) -> Result<Option<Monitor>, PortError>;

    async fn list(&self, query: &MonitorQuery) -> Result<Vec<Monitor>, PortError>;

    /// Every monitor with a schedule, for telling missed runs.
    async fn scheduled(&self) -> Result<Vec<Monitor>, PortError>;

    async fn delete(&self, id: MonitorId) -> Result<(), PortError>;

    /// Stores a check-in; one with the same monitor and ID replaces it.
    async fn record(&self, check_in: &CheckIn) -> Result<(), PortError>;

    async fn check_in(&self, monitor: MonitorId, check_in_id: &str) -> Result<Option<CheckIn>, PortError>;

    /// A monitor's check-ins, newest first.
    async fn check_ins(&self, monitor: MonitorId, limit: u32) -> Result<Vec<CheckIn>, PortError>;

    /// Runs still in progress that started before `before`.
    async fn running(&self, before: Timestamp) -> Result<Vec<CheckIn>, PortError>;
}

/// Offsets of time zones from UTC.
pub trait TimeZones: Send + Sync {
    /// Seconds east of UTC of `zone` (IANA, e.g. `Europe/Berlin`) at `at`;
    /// `None` for a zone it does not know.
    fn offset(&self, zone: &str, at: Timestamp) -> Option<i32>;
}

/// Who hears about a project's scheduled tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeTarget {
    pub recipients: Vec<String>,
    /// The project's setting "scheduled tasks that fail or do not run".
    pub enabled: bool,
    pub project: String,
    pub environment: String,
}

#[async_trait]
pub trait NoticeTargets: Send + Sync {
    async fn target(&self, scope: Scope) -> Result<Option<NoticeTarget>, PortError>;
}
