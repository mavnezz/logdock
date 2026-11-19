//! In-memory implementations of every port, for tests.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::{AuditEntry, AuditLog, Clock, Mail, Mailer, PortError, Scope, Timestamp};

use crate::application::ports::{MonitorQuery, MonitorRepository, NoticeTarget, NoticeTargets, TimeZones};
use crate::application::{MonitorPorts, MonitorUseCases};
use crate::domain::{CheckIn, CheckInStatus, Health, Monitor, MonitorConfig, MonitorId};

#[derive(Default)]
pub struct MemoryMonitors {
    pub monitors: Mutex<Vec<Monitor>>,
    pub check_ins: Mutex<Vec<CheckIn>>,
    next: AtomicI64,
}

#[async_trait]
impl MonitorRepository for MemoryMonitors {
    async fn find(&self, scope: Scope, slug: &str) -> Result<Option<Monitor>, PortError> {
        Ok(self.monitors.lock().unwrap().iter().find(|m| m.scope == scope && m.slug == slug).cloned())
    }

    async fn create(
        &self,
        scope: Scope,
        slug: &str,
        config: Option<&MonitorConfig>,
        at: Timestamp,
    ) -> Result<Monitor, PortError> {
        let monitor = Monitor {
            id: MonitorId::new(self.next.fetch_add(1, Ordering::SeqCst) + 1),
            scope,
            slug: slug.to_owned(),
            config: config.cloned(),
            health: Health::New,
            created_at: at,
            last_check_in: None,
            last_status: None,
            expected_after: at,
        };
        self.monitors.lock().unwrap().push(monitor.clone());
        Ok(monitor)
    }

    async fn save(&self, monitor: &Monitor) -> Result<(), PortError> {
        for stored in self.monitors.lock().unwrap().iter_mut().filter(|m| m.id == monitor.id) {
            *stored = monitor.clone();
        }
        Ok(())
    }

    async fn get(&self, id: MonitorId) -> Result<Option<Monitor>, PortError> {
        Ok(self.monitors.lock().unwrap().iter().find(|m| m.id == id).cloned())
    }

    async fn list(&self, query: &MonitorQuery) -> Result<Vec<Monitor>, PortError> {
        Ok(self
            .monitors
            .lock()
            .unwrap()
            .iter()
            .filter(|m| query.projects.is_empty() || query.projects.contains(&m.scope.project))
            .filter(|m| query.environments.is_empty() || query.environments.contains(&m.scope.environment))
            .cloned()
            .collect())
    }

    async fn scheduled(&self) -> Result<Vec<Monitor>, PortError> {
        Ok(self.monitors.lock().unwrap().iter().filter(|m| m.config.is_some()).cloned().collect())
    }

    async fn delete(&self, id: MonitorId) -> Result<(), PortError> {
        self.monitors.lock().unwrap().retain(|m| m.id != id);
        self.check_ins.lock().unwrap().retain(|c| c.monitor != id);
        Ok(())
    }

    async fn record(&self, check_in: &CheckIn) -> Result<(), PortError> {
        let mut all = self.check_ins.lock().unwrap();
        all.retain(|c| !(c.monitor == check_in.monitor && c.check_in_id == check_in.check_in_id));
        all.push(check_in.clone());
        Ok(())
    }

    async fn check_in(&self, monitor: MonitorId, check_in_id: &str) -> Result<Option<CheckIn>, PortError> {
        let all = self.check_ins.lock().unwrap();
        Ok(all.iter().find(|c| c.monitor == monitor && c.check_in_id == check_in_id).cloned())
    }

    async fn check_ins(&self, monitor: MonitorId, limit: u32) -> Result<Vec<CheckIn>, PortError> {
        let mut found: Vec<CheckIn> =
            self.check_ins.lock().unwrap().iter().filter(|c| c.monitor == monitor).cloned().collect();
        found.sort_by_key(|c| std::cmp::Reverse(c.started));
        found.truncate(limit as usize);
        Ok(found)
    }

    async fn running(&self, before: Timestamp) -> Result<Vec<CheckIn>, PortError> {
        let all = self.check_ins.lock().unwrap();
        Ok(all
            .iter()
            .filter(|c| c.status == CheckInStatus::InProgress && c.started < before)
            .cloned()
            .collect())
    }
}

/// UTC, and `Europe/Berlin` at one hour east (winter time).
pub struct FixedZones;

impl TimeZones for FixedZones {
    fn offset(&self, zone: &str, _at: Timestamp) -> Option<i32> {
        match zone {
            "UTC" => Some(0),
            "Europe/Berlin" => Some(3600),
            _ => None,
        }
    }
}

pub struct FixedTargets(pub Mutex<Option<NoticeTarget>>);

#[async_trait]
impl NoticeTargets for FixedTargets {
    async fn target(&self, _scope: Scope) -> Result<Option<NoticeTarget>, PortError> {
        Ok(self.0.lock().unwrap().clone())
    }
}

#[derive(Default)]
pub struct MemoryMailer(pub Mutex<Vec<Mail>>);

#[async_trait]
impl Mailer for MemoryMailer {
    fn configured(&self) -> bool {
        true
    }

    async fn send(&self, mail: Mail) -> Result<(), PortError> {
        self.0.lock().unwrap().push(mail);
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

impl FixedClock {
    pub fn set(&self, at: Timestamp) {
        *self.0.lock().unwrap() = at;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

pub struct TestMonitors {
    pub use_cases: MonitorUseCases,
    pub monitors: Arc<MemoryMonitors>,
    pub targets: Arc<FixedTargets>,
    pub mailer: Arc<MemoryMailer>,
    pub audit: Arc<MemoryAudit>,
    pub clock: Arc<FixedClock>,
}

impl TestMonitors {
    pub fn new() -> Self {
        let monitors = Arc::new(MemoryMonitors::default());
        let targets = Arc::new(FixedTargets(Mutex::new(Some(NoticeTarget {
            recipients: vec!["ops@example.com".into()],
            enabled: true,
            project: "orders".into(),
            environment: "production".into(),
        }))));
        let mailer = Arc::new(MemoryMailer::default());
        let audit = Arc::new(MemoryAudit::default());
        let clock = Arc::new(FixedClock(Mutex::new(Timestamp::from_unix_seconds(1_795_050_499))));
        let use_cases = MonitorUseCases::new(
            MonitorPorts {
                monitors: monitors.clone(),
                zones: Arc::new(FixedZones),
                targets: targets.clone(),
                mailer: mailer.clone(),
                audit: audit.clone(),
                clock: clock.clone(),
            },
            Some("https://logs.example.com".into()),
        );
        Self { use_cases, monitors, targets, mailer, audit, clock }
    }
}

impl Default for TestMonitors {
    fn default() -> Self {
        Self::new()
    }
}
