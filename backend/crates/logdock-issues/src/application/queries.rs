use std::sync::Arc;

use logdock_kernel::Timestamp;

use super::ports::{EventSummary, IssueQuery, IssueSummary, ReleaseCount, ReleaseErrors, StoredEvent};
use super::{Context, IssueError, hours_ago};
use crate::domain::{ErrorEvent, Issue, IssueId, IssueSearch, TAG_ORDER, USER, values_kept};

/// Query: issues, filtered and sorted.
#[derive(Clone)]
pub struct ListIssues {
    pub(crate) context: Arc<Context>,
}

impl ListIssues {
    /// `query.text` may hold tags (`user.email:ada@example.com`), which
    /// become tag filters.
    pub async fn execute(&self, mut query: IssueQuery) -> Result<Vec<IssueSummary>, IssueError> {
        query.limit = query.limit.clamp(1, 200);
        if let Some(text) = query.text.take() {
            let search = IssueSearch::parse(&text);
            query.text = (!search.words.is_empty()).then(|| search.words.join(" "));
            query.tags.extend(search.tags);
        }
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
    /// The users affected.
    pub users: u64,
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
        let users = c.ports.issues.tag_keys(id).await?.iter().find(|k| k.key == USER).map_or(0, |k| k.values);
        Ok(IssueDetail { issue, hours, releases, latest, users: u64::from(users) })
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

/// Query: what each release brought in errors, last seen first.
#[derive(Clone)]
pub struct ReleasesErrors {
    pub(crate) context: Arc<Context>,
}

impl ReleasesErrors {
    pub async fn execute(
        &self,
        projects: &[logdock_kernel::ProjectId],
        environments: &[logdock_kernel::EnvironmentId],
    ) -> Result<Vec<ReleaseErrors>, IssueError> {
        Ok(self.context.ports.issues.errors_by_release(projects, environments, 200).await?)
    }
}

/// How often one value of a tag came in an issue's events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagValue {
    pub value: String,
    pub count: u64,
}

/// One tag across an issue's events, its values most frequent first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSummary {
    pub key: String,
    /// Events that had the tag at all.
    pub total: u64,
    /// Different values seen; once it reaches the most kept, there may be
    /// more.
    pub distinct: u32,
    /// Whether [`distinct`](Self::distinct) is all there were.
    pub complete: bool,
    pub values: Vec<TagValue>,
}

/// Tags whose values another tag shows already.
const SHOWN_ELSEWHERE: &[&str] =
    &["user.id", "user.email", "user.username", "user.ip", "browser.name", "client_os.name", "level"];

/// Query: what an issue's events had in common: users, route or job, URL,
/// release, browser, service, host and the app's own tags, each value with
/// how often it came.
#[derive(Clone)]
pub struct SummarizeTags {
    pub(crate) context: Arc<Context>,
}

impl SummarizeTags {
    /// Values shown per tag.
    pub const TOP: u32 = 10;

    pub async fn execute(&self, id: IssueId) -> Result<Vec<TagSummary>, IssueError> {
        let issues = &self.context.ports.issues;
        let mut keys = issues.tag_keys(id).await?;
        keys.retain(|k| !SHOWN_ELSEWHERE.contains(&k.key.as_str()));
        let rank = |key: &str| TAG_ORDER.iter().position(|k| *k == key).unwrap_or(TAG_ORDER.len());
        keys.sort_by(|a, b| rank(&a.key).cmp(&rank(&b.key)).then_with(|| a.key.cmp(&b.key)));
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let values = issues.tag_values(id, &key.key, Self::TOP).await?;
            out.push(TagSummary {
                complete: key.values < values_kept(&key.key),
                distinct: key.values,
                total: key.events,
                values: values.into_iter().map(|v| TagValue { value: v.value, count: v.count }).collect(),
                key: key.key,
            });
        }
        Ok(out)
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
