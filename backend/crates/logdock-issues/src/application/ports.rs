//! What this context needs from the outside world.

use async_trait::async_trait;
use logdock_kernel::{EnvironmentId, PortError, ProjectId, Scope, Timestamp, Value};

use crate::domain::{ErrorEvent, EventLevel, Issue, IssueId, NewIssue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IssueSort {
    #[default]
    LastSeen,
    FirstSeen,
    Count,
}

/// Which issues a list shows. Empty lists mean "all".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssueQuery {
    pub projects: Vec<ProjectId>,
    pub environments: Vec<EnvironmentId>,
    /// `unresolved`, `resolved` or `muted`; all when `None`.
    pub status: Option<String>,
    /// Words that appear in the title or culprit.
    pub text: Option<String>,
    pub sort: IssueSort,
    pub limit: u32,
    pub offset: u32,
}

/// An issue in a list, with its events of the last 24 hours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSummary {
    pub issue: Issue,
    pub last_day: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCount {
    pub release: String,
    pub count: u64,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
}

#[async_trait]
pub trait IssueRepository: Send + Sync {
    async fn find(&self, scope: Scope, fingerprint: &str) -> Result<Option<Issue>, PortError>;

    /// `None` if the scope has an issue of that fingerprint already.
    async fn create(&self, issue: NewIssue, event: &Value) -> Result<Option<Issue>, PortError>;

    /// Stores counts, times, releases, level and state; with `event`, it
    /// becomes the issue's latest event, kept beyond the segments.
    async fn save(&self, issue: &Issue, event: Option<&Value>) -> Result<(), PortError>;

    async fn get(&self, id: IssueId) -> Result<Option<Issue>, PortError>;

    async fn list(&self, query: &IssueQuery, since: Timestamp) -> Result<Vec<IssueSummary>, PortError>;

    async fn delete(&self, id: IssueId) -> Result<(), PortError>;

    /// Counts an event for its hour and release.
    async fn record(&self, id: IssueId, at: Timestamp, release: Option<&str>) -> Result<(), PortError>;

    /// Events per hour (Unix seconds of the hour's start) in `[from, to)`.
    async fn hours(&self, id: IssueId, from: Timestamp, to: Timestamp) -> Result<Vec<(i64, u64)>, PortError>;

    async fn releases(&self, id: IssueId) -> Result<Vec<ReleaseCount>, PortError>;

    async fn latest_event(&self, id: IssueId) -> Result<Option<Value>, PortError>;
}

/// An event as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventSummary {
    pub event_id: String,
    pub issue: IssueId,
    pub scope: Scope,
    pub time: Timestamp,
    pub level: EventLevel,
    pub release: Option<String>,
    pub message: String,
    pub transaction: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
    pub trace_id: Option<String>,
}

/// An event as kept.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub issue: IssueId,
    pub scope: Scope,
    pub received: Timestamp,
    pub event: ErrorEvent,
}

#[async_trait]
pub trait EventStore: Send + Sync {
    async fn insert(
        &self,
        issue: IssueId,
        scope: Scope,
        event: &ErrorEvent,
        time: Timestamp,
        received: Timestamp,
    ) -> Result<(), PortError>;

    async fn get(&self, event_id: &str) -> Result<Option<StoredEvent>, PortError>;

    /// An issue's events, newest first, older than `before`.
    async fn list(
        &self,
        issue: IssueId,
        before: Option<Timestamp>,
        limit: u32,
    ) -> Result<Vec<EventSummary>, PortError>;

    async fn by_trace(&self, trace_id: &str) -> Result<Vec<EventSummary>, PortError>;
}
