use std::sync::Arc;

use logdock_kernel::Timestamp;

use super::ports::{EventSummary, IssueQuery, IssueSummary, ReleaseCount, StoredEvent};
use super::{Context, IssueError, hours_ago};
use crate::domain::{ErrorEvent, Issue, IssueId};

/// Query: issues, filtered and sorted.
#[derive(Clone)]
pub struct ListIssues {
    pub(crate) context: Arc<Context>,
}

impl ListIssues {
    pub async fn execute(&self, mut query: IssueQuery) -> Result<Vec<IssueSummary>, IssueError> {
        query.limit = query.limit.clamp(1, 200);
        let since = hours_ago(self.context.ports.clock.now(), 24);
        Ok(self.context.ports.issues.list(&query, since).await?)
    }
}

/// An issue with what its page shows beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueDetail {
    pub issue: Issue,
    /// Events per hour of the last 14 days, oldest first, also the empty
    /// hours.
    pub hours: Vec<(i64, u64)>,
    pub releases: Vec<ReleaseCount>,
    /// The latest event, kept with the issue beyond the segments.
    pub latest: Option<ErrorEvent>,
}

/// Query: one issue.
#[derive(Clone)]
pub struct GetIssue {
    pub(crate) context: Arc<Context>,
}

impl GetIssue {
    pub const DAYS: u64 = 14;

    pub async fn execute(&self, id: IssueId) -> Result<IssueDetail, IssueError> {
        let c = &self.context;
        let issue = c.ports.issues.get(id).await?.ok_or(IssueError::NotFound)?;
        let now = c.ports.clock.now();
        let from = Timestamp::from_unix_seconds(hours_ago(now, Self::DAYS * 24).hour() + 3600);
        let counted = c.ports.issues.hours(id, from, now).await?;
        let mut hours = Vec::with_capacity((Self::DAYS * 24) as usize);
        let mut hour = from.hour();
        while hour <= now.hour() {
            let count = counted.iter().find(|(h, _)| *h == hour).map_or(0, |(_, n)| *n);
            hours.push((hour, count));
            hour += 3600;
        }
        let releases = c.ports.issues.releases(id).await?;
        let latest =
            c.ports.issues.latest_event(id).await?.map(|payload| ErrorEvent::from_payload(payload, None));
        Ok(IssueDetail { issue, hours, releases, latest })
    }
}

/// Query: an issue's events, newest first.
#[derive(Clone)]
pub struct ListEvents {
    pub(crate) context: Arc<Context>,
}

impl ListEvents {
    pub async fn execute(
        &self,
        id: IssueId,
        before: Option<Timestamp>,
        limit: u32,
    ) -> Result<Vec<EventSummary>, IssueError> {
        Ok(self.context.ports.events.list(id, before, limit.clamp(1, 100)).await?)
    }
}

/// How often one value of a tag came in an issue's recent events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagValue {
    pub value: String,
    pub count: u32,
}

/// One tag across an issue's recent events, its values most frequent first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSummary {
    pub key: &'static str,
    /// Events that had the tag at all.
    pub total: u32,
    pub values: Vec<TagValue>,
}

/// Reads one tag of an event.
type TagOf = fn(&EventSummary) -> Option<&str>;

/// How many recent events [`SummarizeTags`] looks at, as Sentry samples.
pub const TAG_SAMPLE: u32 = 1000;

/// Query: where an issue's recent events came from: release, route or job,
/// service and host, each value with how often it came.
#[derive(Clone)]
pub struct SummarizeTags {
    pub(crate) context: Arc<Context>,
}

impl SummarizeTags {
    pub async fn execute(&self, id: IssueId) -> Result<Vec<TagSummary>, IssueError> {
        let events = self.context.ports.events.list(id, None, TAG_SAMPLE).await?;
        let tags: [(&'static str, TagOf); 4] = [
            ("release", |e| e.release.as_deref()),
            ("transaction", |e| e.transaction.as_deref()),
            ("service", |e| e.service.as_deref()),
            ("host", |e| e.host.as_deref()),
        ];
        Ok(tags
            .into_iter()
            .map(|(key, of)| {
                let mut counts: Vec<TagValue> = Vec::new();
                for value in events.iter().filter_map(of) {
                    match counts.iter_mut().find(|c| c.value == value) {
                        Some(known) => known.count += 1,
                        None => counts.push(TagValue { value: value.to_owned(), count: 1 }),
                    }
                }
                counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
                let total = counts.iter().map(|c| c.count).sum();
                counts.truncate(10);
                TagSummary { key, total, values: counts }
            })
            .filter(|summary| summary.total > 0)
            .collect())
    }
}

/// Query: one event by the SDK's event ID.
#[derive(Clone)]
pub struct GetEvent {
    pub(crate) context: Arc<Context>,
}

impl GetEvent {
    pub async fn execute(&self, event_id: &str) -> Result<Option<StoredEvent>, IssueError> {
        let id = event_id.replace('-', "").to_ascii_lowercase();
        Ok(self.context.ports.events.get(&id).await?)
    }
}

/// Query: the error events of one trace.
#[derive(Clone)]
pub struct EventsOfTrace {
    pub(crate) context: Arc<Context>,
}

impl EventsOfTrace {
    pub async fn execute(&self, trace_id: &str) -> Result<Vec<EventSummary>, IssueError> {
        Ok(self.context.ports.events.by_trace(trace_id).await?)
    }
}
