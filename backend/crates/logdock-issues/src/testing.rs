//! In-memory implementations of every port, for tests.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::{AuditEntry, AuditLog, Clock, Mail, Mailer, PortError, Scope, Timestamp, Value};

use crate::application::ports::{
    EventStore, EventSummary, IssueQuery, IssueRepository, IssueSummary, ReleaseCount, StoredEvent,
};
use crate::application::{IssuePorts, IssueUseCases};
use crate::domain::{ErrorEvent, Issue, IssueId, IssueStatus, NewIssue};

#[derive(Default)]
pub struct MemoryIssues {
    pub issues: Mutex<Vec<(Issue, Value)>>,
    hits: Mutex<Vec<(IssueId, Timestamp, Option<String>)>>,
    next: AtomicI64,
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
            .map(|(issue, _)| IssueSummary {
                issue: issue.clone(),
                last_day: hits.iter().filter(|(id, at, _)| *id == issue.id && *at >= since).count() as u64,
            })
            .collect())
    }

    async fn delete(&self, id: IssueId) -> Result<(), PortError> {
        self.issues.lock().unwrap().retain(|(i, _)| i.id != id);
        Ok(())
    }

    async fn record(&self, id: IssueId, at: Timestamp, release: Option<&str>) -> Result<(), PortError> {
        self.hits.lock().unwrap().push((id, at, release.map(str::to_owned)));
        Ok(())
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
