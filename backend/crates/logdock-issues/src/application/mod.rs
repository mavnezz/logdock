//! Use cases of the Issues context and the ports they need.

mod alerts;
mod ingest;
pub mod ports;
mod queries;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use logdock_kernel::{
    Actor, AuditEntry, AuditLog, Clock, Mailer, Outcome, PortError, RequestContext, Timestamp,
};
use tokio::sync::Mutex;

use crate::domain::{IssueId, IssueStatus};
pub use alerts::{NotifyPolicy, alert_mail};
pub use ingest::{IngestEvent, IngestedEvent};
use ports::{EventStore, IssueRepository};
pub use queries::{EventsOfTrace, GetEvent, GetIssue, IssueDetail, ListEvents, ListIssues};

#[derive(Debug, thiserror::Error)]
pub enum IssueError {
    #[error("the event has no ID")]
    NoEventId,
    #[error("there is no such issue")]
    NotFound,
    #[error(transparent)]
    Port(#[from] PortError),
}

/// Adapters for every port of this context.
#[derive(Clone)]
pub struct IssuePorts {
    pub issues: Arc<dyn IssueRepository>,
    pub events: Arc<dyn EventStore>,
    pub mailer: Arc<dyn Mailer>,
    pub audit: Arc<dyn AuditLog>,
    pub clock: Arc<dyn Clock>,
}

pub(crate) struct Context {
    pub ports: IssuePorts,
    /// Grouping is serialized: two events of a new issue at once must not
    /// make two issues. Errors are few; one lock is enough.
    pub grouping: Mutex<()>,
    /// Where the interface is reached, for links in e-mail.
    pub base_url: Option<String>,
}

/// All use cases of this context.
#[derive(Clone)]
pub struct IssueUseCases {
    pub ingest: IngestEvent,
    pub list: ListIssues,
    pub get: GetIssue,
    pub events: ListEvents,
    pub event: GetEvent,
    pub trace: EventsOfTrace,
    pub change: ChangeStatus,
    pub delete: DeleteIssue,
}

impl IssueUseCases {
    pub fn new(ports: IssuePorts, base_url: Option<String>) -> Self {
        let context = Arc::new(Context { ports, grouping: Mutex::new(()), base_url });
        Self {
            ingest: IngestEvent { context: Arc::clone(&context) },
            list: ListIssues { context: Arc::clone(&context) },
            get: GetIssue { context: Arc::clone(&context) },
            events: ListEvents { context: Arc::clone(&context) },
            event: GetEvent { context: Arc::clone(&context) },
            trace: EventsOfTrace { context: Arc::clone(&context) },
            change: ChangeStatus { context: Arc::clone(&context) },
            delete: DeleteIssue { context },
        }
    }
}

/// What someone does with an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusChange {
    Resolve,
    /// Resolved in the release after the latest seen: events of that
    /// release do not bring it back.
    ResolveInNextRelease,
    /// For a while (`for_duration`), a number of events, or for good.
    Mute {
        for_duration: Option<Duration>,
        events: Option<u32>,
    },
    Reopen,
}

/// Command: resolves, mutes or reopens an issue.
#[derive(Clone)]
pub struct ChangeStatus {
    context: Arc<Context>,
}

impl ChangeStatus {
    pub async fn execute(
        &self,
        actor: &Actor,
        id: IssueId,
        change: StatusChange,
        request: &RequestContext,
    ) -> Result<crate::domain::Issue, IssueError> {
        let c = &self.context;
        let _grouping = c.grouping.lock().await;
        let mut issue = c.ports.issues.get(id).await?.ok_or(IssueError::NotFound)?;
        let now = c.ports.clock.now();
        let action = match &change {
            StatusChange::Resolve => "issue.resolve",
            StatusChange::ResolveInNextRelease => "issue.resolve_next_release",
            StatusChange::Mute { .. } => "issue.mute",
            StatusChange::Reopen => "issue.reopen",
        };
        issue.status = match change {
            StatusChange::Resolve => IssueStatus::Resolved { at: now, in_next_release: None },
            StatusChange::ResolveInNextRelease => {
                IssueStatus::Resolved { at: now, in_next_release: issue.last_release.clone() }
            }
            StatusChange::Mute { for_duration, events } => {
                IssueStatus::Muted { until: for_duration.map(|d| now.saturating_add(d)), remaining: events }
            }
            StatusChange::Reopen => IssueStatus::Unresolved,
        };
        c.ports.issues.save(&issue, None).await?;
        let entry =
            AuditEntry::new(now, action, Outcome::Success, request).by(actor).target(issue.title.clone());
        c.ports.audit.record(entry).await?;
        Ok(issue)
    }
}

/// Command: deletes an issue; its events go with their segments.
#[derive(Clone)]
pub struct DeleteIssue {
    context: Arc<Context>,
}

impl DeleteIssue {
    pub async fn execute(
        &self,
        actor: &Actor,
        id: IssueId,
        request: &RequestContext,
    ) -> Result<(), IssueError> {
        let c = &self.context;
        let issue = c.ports.issues.get(id).await?.ok_or(IssueError::NotFound)?;
        c.ports.issues.delete(id).await?;
        let entry = AuditEntry::new(c.ports.clock.now(), "issue.delete", Outcome::Success, request)
            .by(actor)
            .target(issue.title);
        c.ports.audit.record(entry).await?;
        Ok(())
    }
}

pub(crate) fn hours_ago(now: Timestamp, hours: u64) -> Timestamp {
    now.saturating_sub(Duration::from_secs(hours * 3600))
}
