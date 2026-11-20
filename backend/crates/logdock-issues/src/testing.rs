//! In-memory implementations of every port, for tests.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::{
    AuditEntry, AuditLog, Clock, EnvironmentId, Mail, Mailer, PortError, ProjectId, Scope, Timestamp, Value,
};

use crate::application::ports::{
    EventStore, EventSummary, IssueQuery, IssueRepository, IssueSort, IssueSummary, ReleaseCount,
    ReleaseErrors, StoredEvent, TagCount, TagKey,
};
use crate::application::{IssuePorts, IssueUseCases};
use crate::domain::{ErrorEvent, Issue, IssueId, IssueStatus, NewIssue};

#[derive(Default)]
pub struct MemoryIssues {
    pub issues: Mutex<Vec<(Issue, Value)>>,
    hits: Mutex<Vec<(IssueId, Timestamp, Option<String>)>>,
    /// Each event's tags.
    tags: Mutex<Vec<(IssueId, String, String)>>,
    next: AtomicI64,
}

impl MemoryIssues {
    fn users(&self, id: IssueId) -> u64 {
        let mut users: Vec<String> = self
            .tags
            .lock()
            .unwrap()
            .iter()
            .filter(|(i, k, _)| *i == id && k == crate::domain::USER)
            .map(|(_, _, v)| v.clone())
            .collect();
        users.sort();
        users.dedup();
        users.len() as u64
    }

    fn has(&self, id: IssueId, filter: &crate::domain::TagFilter) -> bool {
        let found = self.tags.lock().unwrap().iter().any(|(i, k, v)| {
            *i == id
                && *k == filter.key
                && if filter.prefix { v.starts_with(&filter.value) } else { *v == filter.value }
        });
        found != filter.negated
    }
}

#[async_trait]
impl IssueRepository for MemoryIssues {
    async fn find(&self, scope: Scope, fingerprint: &str) -> Result<Option<Issue>, PortError> {
        Ok(self
            .issues
            .lock()
            .unwrap()
            .iter()
            .find(|(i, _)| i.scope == scope && i.fingerprint == fingerprint)
            .map(|(i, _)| i.clone()))
    }

    async fn create(&self, new: NewIssue, event: &Value) -> Result<Option<Issue>, PortError> {
        if self.find(new.scope, &new.fingerprint).await?.is_some() {
            return Ok(None);
        }
        let issue = Issue {
            id: IssueId::new(self.next.fetch_add(1, Ordering::SeqCst) + 1),
            scope: new.scope,
            fingerprint: new.fingerprint,
            title: new.title,
            culprit: new.culprit,
            level: new.level,
            status: IssueStatus::Unresolved,
            first_seen: new.seen,
            last_seen: new.seen,
            count: 1,
            first_release: new.release.clone(),
            last_release: new.release,
            regressed_at: None,
            last_event_id: Some(new.event_id),
        };
        self.issues.lock().unwrap().push((issue.clone(), event.clone()));
        Ok(Some(issue))
    }

    async fn save(&self, issue: &Issue, event: Option<&Value>) -> Result<(), PortError> {
        for (stored, payload) in self.issues.lock().unwrap().iter_mut() {
            if stored.id == issue.id {
                *stored = issue.clone();
                if let Some(event) = event {
                    *payload = event.clone();
                }
            }
        }
        Ok(())
    }

    async fn get(&self, id: IssueId) -> Result<Option<Issue>, PortError> {
        Ok(self.issues.lock().unwrap().iter().find(|(i, _)| i.id == id).map(|(i, _)| i.clone()))
    }

    async fn list(&self, query: &IssueQuery, since: Timestamp) -> Result<Vec<IssueSummary>, PortError> {
        let hits = self.hits.lock().unwrap();
        Ok(self
            .issues
            .lock()
            .unwrap()
            .iter()
            .filter(|(i, _)| query.status.as_deref().is_none_or(|s| s == i.status.name()))
            .filter(|(i, _)| {
                query.text.as_deref().is_none_or(|t| i.title.to_lowercase().contains(&t.to_lowercase()))
            })
            .filter(|(i, _)| query.tags.iter().all(|filter| self.has(i.id, filter)))
            .map(|(issue, _)| IssueSummary {
                issue: issue.clone(),
                last_day: hits.iter().filter(|(id, at, _)| *id == issue.id && *at >= since).count() as u64,
                users: self.users(issue.id),
            })
            .collect::<Vec<_>>())
        .map(|mut list| {
            if query.sort == IssueSort::Users {
                list.sort_by_key(|s| std::cmp::Reverse(s.users));
            }
            list
        })
    }

    async fn delete(&self, id: IssueId) -> Result<(), PortError> {
        self.issues.lock().unwrap().retain(|(i, _)| i.id != id);
        Ok(())
    }

    async fn record(
        &self,
        id: IssueId,
        at: Timestamp,
        release: Option<&str>,
        tags: &[(String, String)],
    ) -> Result<(), PortError> {
        self.hits.lock().unwrap().push((id, at, release.map(str::to_owned)));
        self.tags.lock().unwrap().extend(tags.iter().map(|(k, v)| (id, k.clone(), v.clone())));
        Ok(())
    }

    async fn tag_keys(&self, id: IssueId) -> Result<Vec<TagKey>, PortError> {
        let mut keys: Vec<TagKey> = Vec::new();
        let tags = self.tags.lock().unwrap();
        for (_, key, _) in tags.iter().filter(|(i, _, _)| *i == id) {
            match keys.iter_mut().find(|k| &k.key == key) {
                Some(known) => known.events += 1,
                None => keys.push(TagKey { key: key.clone(), events: 1, values: 0 }),
            }
        }
        for known in &mut keys {
            let mut values: Vec<&String> =
                tags.iter().filter(|(i, k, _)| *i == id && *k == known.key).map(|(_, _, v)| v).collect();
            values.sort();
            values.dedup();
            known.values = values.len() as u32;
        }
        Ok(keys)
    }

    async fn tag_values(&self, id: IssueId, key: &str, limit: u32) -> Result<Vec<TagCount>, PortError> {
        let mut values: Vec<TagCount> = Vec::new();
        for (_, _, value) in self.tags.lock().unwrap().iter().filter(|(i, k, _)| *i == id && k == key) {
            match values.iter_mut().find(|c| &c.value == value) {
                Some(known) => known.count += 1,
                None => values.push(TagCount { value: value.clone(), count: 1 }),
            }
        }
        values.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
        values.truncate(limit as usize);
        Ok(values)
    }

    async fn hours(&self, id: IssueId, from: Timestamp, to: Timestamp) -> Result<Vec<(i64, u64)>, PortError> {
        let mut hours: Vec<(i64, u64)> = Vec::new();
        for (_, at, _) in
            self.hits.lock().unwrap().iter().filter(|(i, at, _)| *i == id && *at >= from && *at < to)
        {
            match hours.iter_mut().find(|(h, _)| *h == at.hour()) {
                Some((_, n)) => *n += 1,
                None => hours.push((at.hour(), 1)),
            }
        }
        Ok(hours)
    }

    async fn errors_by_release(
        &self,
        projects: &[ProjectId],
        environments: &[EnvironmentId],
        limit: u32,
    ) -> Result<Vec<ReleaseErrors>, PortError> {
        let issues = self.issues.lock().unwrap().clone();
        let within = |issue: &Issue| {
            (projects.is_empty() || projects.contains(&issue.scope.project))
                && (environments.is_empty() || environments.contains(&issue.scope.environment))
        };
        let mut out: Vec<ReleaseErrors> = Vec::new();
        for (id, at, release) in self.hits.lock().unwrap().iter() {
            let Some(release) = release else { continue };
            if !issues.iter().any(|(i, _)| i.id == *id && within(i)) {
                continue;
            }
            let new = |r: &str| {
                issues.iter().filter(|(i, _)| within(i) && i.first_release.as_deref() == Some(r)).count()
                    as u64
            };
            match out.iter_mut().find(|r| &r.release == release) {
                Some(known) => {
                    known.events += 1;
                    known.first_seen = known.first_seen.min(*at);
                    known.last_seen = known.last_seen.max(*at);
                }
                None => out.push(ReleaseErrors {
                    release: release.clone(),
                    events: 1,
                    issues: 0,
                    new_issues: new(release),
                    first_seen: *at,
                    last_seen: *at,
                }),
            }
        }
        for summary in &mut out {
            let mut ids: Vec<IssueId> = self
                .hits
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, _, r)| r.as_deref() == Some(summary.release.as_str()))
                .map(|(id, _, _)| *id)
                .collect();
            ids.sort_by_key(|id| id.get());
            ids.dedup();
            summary.issues = ids.len() as u64;
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.last_seen));
        out.truncate(limit as usize);
        Ok(out)
    }

    async fn releases(&self, id: IssueId) -> Result<Vec<ReleaseCount>, PortError> {
        let mut releases: Vec<ReleaseCount> = Vec::new();
        for (_, at, release) in self.hits.lock().unwrap().iter().filter(|(i, _, _)| *i == id) {
            let Some(release) = release else { continue };
            match releases.iter_mut().find(|r| &r.release == release) {
                Some(r) => {
                    r.count += 1;
                    r.last_seen = r.last_seen.max(*at);
                }
                None => releases.push(ReleaseCount {
                    release: release.clone(),
                    count: 1,
                    first_seen: *at,
                    last_seen: *at,
                }),
            }
        }
        Ok(releases)
    }

    async fn latest_event(&self, id: IssueId) -> Result<Option<Value>, PortError> {
        Ok(self.issues.lock().unwrap().iter().find(|(i, _)| i.id == id).map(|(_, e)| e.clone()))
    }
}

#[derive(Default)]
pub struct MemoryEvents(pub Mutex<Vec<StoredEvent>>);

fn summary(stored: &StoredEvent, time: Timestamp) -> EventSummary {
    let event = &stored.event;
    EventSummary {
        event_id: event.event_id.clone(),
        issue: stored.issue,
        scope: stored.scope,
        time,
        level: event.level,
        release: event.release.clone(),
        message: event
            .primary()
            .map_or_else(|| event.message.clone().unwrap_or_default(), |e| e.value.clone()),
        transaction: event.transaction.clone(),
        service: event.tag("service").map(str::to_owned),
        host: event.tag("host").map(str::to_owned),
        trace_id: event.trace_id.clone(),
    }
}

#[async_trait]
impl EventStore for MemoryEvents {
    async fn insert(
        &self,
        issue: IssueId,
        scope: Scope,
        event: &ErrorEvent,
        _time: Timestamp,
        received: Timestamp,
    ) -> Result<(), PortError> {
        self.0.lock().unwrap().push(StoredEvent { issue, scope, received, event: event.clone() });
        Ok(())
    }

    async fn get(&self, event_id: &str) -> Result<Option<StoredEvent>, PortError> {
        Ok(self.0.lock().unwrap().iter().find(|e| e.event.event_id == event_id).cloned())
    }

    async fn seen(&self, event_id: &str, since: Timestamp) -> Result<Option<IssueId>, PortError> {
        let events = self.0.lock().unwrap();
        Ok(events.iter().find(|e| e.event.event_id == event_id && e.received >= since).map(|e| e.issue))
    }

    async fn list(
        &self,
        issue: IssueId,
        _before: Option<Timestamp>,
        limit: u32,
    ) -> Result<Vec<EventSummary>, PortError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .filter(|e| e.issue == issue)
            .take(limit as usize)
            .map(|e| summary(e, e.received))
            .collect())
    }

    async fn by_trace(&self, trace_id: &str) -> Result<Vec<EventSummary>, PortError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.event.trace_id.as_deref() == Some(trace_id))
            .map(|e| summary(e, e.received))
            .collect())
    }
}

/// Keeps what would be sent.
#[derive(Default)]
pub struct MemoryMailer {
    pub sent: Mutex<Vec<Mail>>,
    pub off: bool,
}

#[async_trait]
impl Mailer for MemoryMailer {
    fn configured(&self) -> bool {
        !self.off
    }

    async fn send(&self, mail: Mail) -> Result<(), PortError> {
        self.sent.lock().unwrap().push(mail);
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryAudit(pub Mutex<Vec<AuditEntry>>);

#[async_trait]
impl AuditLog for MemoryAudit {
    async fn record(&self, entry: AuditEntry) -> Result<(), PortError> {
        self.0.lock().unwrap().push(entry);
        Ok(())
    }
}

pub struct FixedClock(pub Mutex<Timestamp>);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

pub struct TestIssues {
    pub use_cases: IssueUseCases,
    pub issues: Arc<MemoryIssues>,
    pub events: Arc<MemoryEvents>,
    pub mailer: Arc<MemoryMailer>,
    pub audit: Arc<MemoryAudit>,
    pub clock: Arc<FixedClock>,
}

impl TestIssues {
    pub fn new() -> Self {
        let issues = Arc::new(MemoryIssues::default());
        let events = Arc::new(MemoryEvents::default());
        let mailer = Arc::new(MemoryMailer::default());
        let audit = Arc::new(MemoryAudit::default());
        let clock = Arc::new(FixedClock(Mutex::new(Timestamp::from_unix_seconds(1_795_000_000))));
        let ports = IssuePorts {
            issues: issues.clone(),
            events: events.clone(),
            mailer: mailer.clone(),
            audit: audit.clone(),
            clock: clock.clone(),
        };
        let use_cases = IssueUseCases::new(ports, Some("https://logs.example.com".into()));
        Self { use_cases, issues, events, mailer, audit, clock }
    }
}

impl Default for TestIssues {
    fn default() -> Self {
        Self::new()
    }
}
