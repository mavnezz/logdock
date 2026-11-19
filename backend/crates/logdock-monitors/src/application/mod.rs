//! Use cases of the Monitors context and the ports they need.

pub mod ports;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use logdock_kernel::{
    Actor, AuditEntry, AuditLog, Clock, Mail, Mailer, Outcome, PortError, RequestContext, Scope, Timestamp,
    Value,
};
use tokio::sync::Mutex;

use crate::domain::{CheckIn, CheckInStatus, Health, Monitor, MonitorConfig, MonitorId, NewCheckIn};
use ports::{MonitorQuery, MonitorRepository, NoticeTargets, TimeZones};

#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error("the check-in has no monitor slug or no status")]
    Unreadable,
    #[error("there is no such monitor")]
    NotFound,
    #[error(transparent)]
    Port(#[from] PortError),
}

/// Adapters for every port of this context.
#[derive(Clone)]
pub struct MonitorPorts {
    pub monitors: Arc<dyn MonitorRepository>,
    pub zones: Arc<dyn TimeZones>,
    pub targets: Arc<dyn NoticeTargets>,
    pub mailer: Arc<dyn Mailer>,
    pub audit: Arc<dyn AuditLog>,
    pub clock: Arc<dyn Clock>,
}

pub(crate) struct Context {
    ports: MonitorPorts,
    /// Check-ins and the sweep change monitors one at a time.
    lock: Mutex<()>,
    base_url: Option<String>,
}

/// All use cases of this context.
#[derive(Clone)]
pub struct MonitorUseCases {
    pub ingest: IngestCheckIn,
    pub sweep: Sweep,
    pub list: ListMonitors,
    pub get: GetMonitor,
    pub delete: DeleteMonitor,
}

impl MonitorUseCases {
    pub fn new(ports: MonitorPorts, base_url: Option<String>) -> Self {
        let context = Arc::new(Context { ports, lock: Mutex::new(()), base_url });
        Self {
            ingest: IngestCheckIn { context: Arc::clone(&context) },
            sweep: Sweep { context: Arc::clone(&context) },
            list: ListMonitors { context: Arc::clone(&context) },
            get: GetMonitor { context: Arc::clone(&context) },
            delete: DeleteMonitor { context },
        }
    }
}

impl Context {
    fn offset<'a>(&'a self, monitor: &'a Monitor) -> impl Fn(Timestamp) -> i32 + 'a {
        let zone = monitor.config.as_ref().map_or("UTC", |c| c.timezone.as_str());
        move |at| self.ports.zones.offset(zone, at).unwrap_or(0)
    }

    fn next_run(&self, monitor: &Monitor) -> Option<Timestamp> {
        let config = monitor.config.as_ref()?;
        let offset = self.offset(monitor);
        config.schedule.next_run(monitor.expected_after.saturating_add(Duration::from_secs(1)), &offset)
    }

    /// Mail on a change between good and bad health; the sender is not
    /// waited for.
    async fn tell(&self, monitor: &Monitor, before: Health, check_in: &CheckIn) {
        let what = match (before.bad(), monitor.health) {
            (false, Health::Failing) if check_in.status == CheckInStatus::Timeout => "took too long",
            (false, Health::Failing) => "failed",
            (false, Health::Missed) => "did not run",
            (true, Health::Ok) => "runs again",
            _ => return,
        };
        if !self.ports.mailer.configured() {
            return;
        }
        let target = match self.ports.targets.target(monitor.scope).await {
            Ok(Some(target)) if target.enabled && !target.recipients.is_empty() => target,
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(%error, "cannot tell whom to notify about a scheduled task");
                return;
            }
        };
        let place = format!("{} / {}", target.project, target.environment);
        let mut text = format!("The scheduled task {} {what} in {place}.\n\n", monitor.name());
        if let Some(config) = &monitor.config {
            text.push_str(&format!("Schedule: {} ({})\n", config.schedule.text(), config.timezone));
        }
        let when = if check_in.status == CheckInStatus::Missed { "Expected at" } else { "Started" };
        text.push_str(&format!("{when}: {}\n", check_in.started.utc()));
        if let Some(release) = &check_in.release {
            text.push_str(&format!("Release: {release}\n"));
        }
        if let Some(base) = &self.base_url {
            text.push_str(&format!("\n{}/monitors/{}\n", base.trim_end_matches('/'), monitor.id));
        }
        text.push_str("\n-- \nLogdock\n");
        let mail = Mail {
            to: target.recipients,
            subject: format!("[Logdock] {} {what} in {place}", monitor.name()),
            text,
        };
        let mailer = Arc::clone(&self.ports.mailer);
        tokio::spawn(async move {
            if let Err(error) = mailer.send(mail).await {
                tracing::warn!(%error, "cannot send the scheduled task's e-mail");
            }
        });
    }
}

/// Command: takes a check-in of a scheduled task; creates its monitor with
/// the first one.
#[derive(Clone)]
pub struct IngestCheckIn {
    context: Arc<Context>,
}

impl IngestCheckIn {
    pub async fn execute(&self, scope: Scope, payload: &Value) -> Result<Monitor, MonitorError> {
        let c = &self.context;
        let new = NewCheckIn::from_payload(payload).ok_or(MonitorError::Unreadable)?;
        let now = c.ports.clock.now();
        let _lock = c.lock.lock().await;
        let monitors = &c.ports.monitors;
        let mut monitor = match monitors.find(scope, &new.slug).await? {
            Some(monitor) => monitor,
            None => monitors.create(scope, &new.slug, new.config.as_ref(), now).await?,
        };
        if new.config.is_some() {
            monitor.config = new.config.clone();
        }
        let earlier = match new.check_in_id.as_str() {
            "" => None,
            id => monitors.check_in(monitor.id, id).await?,
        };
        let started = match &earlier {
            Some(earlier) => earlier.started,
            None if new.status == CheckInStatus::InProgress => now,
            None => now.saturating_sub(new.duration.unwrap_or_default()),
        };
        let finished = new.status.finished().then_some(now);
        let check_in = CheckIn {
            monitor: monitor.id,
            check_in_id: if new.check_in_id.is_empty() {
                format!("at-{}", now.unix_micros())
            } else {
                new.check_in_id
            },
            status: new.status,
            started,
            finished,
            duration: new.duration.or_else(|| finished.map(|f| f.duration_since(started))),
            release: new.release.or_else(|| earlier.as_ref().and_then(|e| e.release.clone())),
            trace_id: new.trace_id,
        };
        monitors.record(&check_in).await?;
        let before = monitor.record(check_in.status, started, now);
        monitors.save(&monitor).await?;
        c.tell(&monitor, before, &check_in).await;
        Ok(monitor)
    }
}

/// What a sweep found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SweepReport {
    pub missed: u32,
    pub timed_out: u32,
}

/// At most this many missed runs per monitor and sweep, so a long pause
/// (Logdock was stopped) is caught up with over a few sweeps.
const MISSED_PER_SWEEP: u32 = 100;

/// Command, every minute: runs that took too long, and runs that did not
/// happen when their schedule says.
#[derive(Clone)]
pub struct Sweep {
    context: Arc<Context>,
}

impl Sweep {
    pub async fn execute(&self) -> Result<SweepReport, MonitorError> {
        let c = &self.context;
        let monitors = &c.ports.monitors;
        let now = c.ports.clock.now();
        let _lock = c.lock.lock().await;
        let mut report = SweepReport::default();

        for mut running in monitors.running(now.saturating_sub(Duration::from_secs(60))).await? {
            let Some(mut monitor) = monitors.get(running.monitor).await? else { continue };
            let max = monitor.config.as_ref().map_or(MonitorConfig::DEFAULT_MAX_RUNTIME, |c| c.max_runtime);
            if now.duration_since(running.started) < max {
                continue;
            }
            running.status = CheckInStatus::Timeout;
            running.finished = Some(now);
            running.duration = Some(now.duration_since(running.started));
            monitors.record(&running).await?;
            let before = monitor.record(CheckInStatus::Timeout, running.started, now);
            monitors.save(&monitor).await?;
            c.tell(&monitor, before, &running).await;
            report.timed_out += 1;
        }

        for mut monitor in monitors.scheduled().await? {
            let mut missed = 0;
            loop {
                let expected = {
                    let offset = c.offset(&monitor);
                    monitor.missed_run(now, &offset)
                };
                let Some(expected) = expected else { break };
                let check_in = CheckIn {
                    monitor: monitor.id,
                    check_in_id: format!("missed-{}", expected.unix_seconds()),
                    status: CheckInStatus::Missed,
                    started: expected,
                    finished: Some(expected),
                    duration: None,
                    release: None,
                    trace_id: None,
                };
                monitors.record(&check_in).await?;
                let before = monitor.record(CheckInStatus::Missed, expected, now);
                monitors.save(&monitor).await?;
                c.tell(&monitor, before, &check_in).await;
                missed += 1;
                if missed >= MISSED_PER_SWEEP {
                    break;
                }
            }
            report.missed += missed;
        }
        Ok(report)
    }
}

/// A monitor with when it should run next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorSummary {
    pub monitor: Monitor,
    pub next_run: Option<Timestamp>,
}

/// Query: the monitors, by project and stage.
#[derive(Clone)]
pub struct ListMonitors {
    context: Arc<Context>,
}

impl ListMonitors {
    pub async fn execute(&self, query: MonitorQuery) -> Result<Vec<MonitorSummary>, MonitorError> {
        let c = &self.context;
        let mut monitors = c.ports.monitors.list(&query).await?;
        monitors.sort_by(|a, b| {
            let rank = |m: &Monitor| if m.health.bad() { 0 } else { 1 };
            rank(a).cmp(&rank(b)).then_with(|| a.slug.cmp(&b.slug))
        });
        Ok(monitors
            .into_iter()
            .map(|monitor| MonitorSummary { next_run: c.next_run(&monitor), monitor })
            .collect())
    }
}

/// A monitor with its latest check-ins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorDetail {
    pub summary: MonitorSummary,
    pub check_ins: Vec<CheckIn>,
}

/// Query: one monitor.
#[derive(Clone)]
pub struct GetMonitor {
    context: Arc<Context>,
}

impl GetMonitor {
    pub async fn execute(&self, id: MonitorId) -> Result<MonitorDetail, MonitorError> {
        let c = &self.context;
        let monitor = c.ports.monitors.get(id).await?.ok_or(MonitorError::NotFound)?;
        let check_ins = c.ports.monitors.check_ins(id, 100).await?;
        Ok(MonitorDetail { summary: MonitorSummary { next_run: c.next_run(&monitor), monitor }, check_ins })
    }
}

/// Command: forgets a monitor and its check-ins; the next check-in creates
/// it again.
#[derive(Clone)]
pub struct DeleteMonitor {
    context: Arc<Context>,
}

impl DeleteMonitor {
    pub async fn execute(
        &self,
        actor: &Actor,
        id: MonitorId,
        request: &RequestContext,
    ) -> Result<(), MonitorError> {
        let c = &self.context;
        let monitor = c.ports.monitors.get(id).await?.ok_or(MonitorError::NotFound)?;
        c.ports.monitors.delete(id).await?;
        let entry = AuditEntry::new(c.ports.clock.now(), "monitor.delete", Outcome::Success, request)
            .by(actor)
            .target(monitor.slug);
        c.ports.audit.record(entry).await?;
        Ok(())
    }
}
