//! A scheduled task as Logdock knows it from its check-ins, and how its
//! health follows from them.

use std::fmt;
use std::time::Duration;

use logdock_kernel::{Scope, Timestamp, Value};

use super::schedule::{Crontab, IntervalUnit, Schedule};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MonitorId(i64);

impl MonitorId {
    pub const fn new(id: i64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What the SDK says about a task's schedule with every check-in
/// (`monitor_config`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorConfig {
    pub schedule: Schedule,
    /// An IANA zone such as `Europe/Berlin`; `UTC` when none is given.
    pub timezone: String,
    /// How late a run may start before it counts as missed.
    pub margin: Duration,
    /// How long a run may take before it counts as failed.
    pub max_runtime: Duration,
}

impl MonitorConfig {
    /// Sentry's default for a run that is late.
    pub const DEFAULT_MARGIN: Duration = Duration::from_secs(2 * 60);
    pub const DEFAULT_MAX_RUNTIME: Duration = Duration::from_secs(30 * 60);

    pub fn from_payload(config: &Value) -> Option<MonitorConfig> {
        let schedule = config.get("schedule")?;
        let schedule = match schedule.get("type").and_then(Value::as_str) {
            Some("interval") => Schedule::Interval {
                every: schedule.get("value").and_then(Value::as_i64).and_then(|v| u32::try_from(v).ok())?,
                unit: IntervalUnit::parse(schedule.get("unit").and_then(Value::as_str)?)?,
            },
            _ => Schedule::Crontab(Crontab::parse(schedule.get("value").and_then(Value::as_str)?).ok()?),
        };
        let minutes = |key: &str, default: Duration| {
            config
                .get(key)
                .and_then(Value::as_f64)
                .filter(|m| *m > 0.0 && *m < 525_600.0)
                .map_or(default, |m| Duration::from_secs_f64(m * 60.0))
        };
        Some(MonitorConfig {
            schedule,
            timezone: config
                .get("timezone")
                .and_then(Value::as_str)
                .filter(|z| !z.is_empty())
                .unwrap_or("UTC")
                .to_owned(),
            margin: minutes("checkin_margin", Self::DEFAULT_MARGIN),
            max_runtime: minutes("max_runtime", Self::DEFAULT_MAX_RUNTIME),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckInStatus {
    InProgress,
    Ok,
    Error,
    /// Logdock's own: the run did not start when it should have.
    Missed,
    /// Logdock's own: the run did not finish within the maximum runtime.
    Timeout,
}

impl CheckInStatus {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "in_progress" => Self::InProgress,
            "ok" => Self::Ok,
            "error" => Self::Error,
            "missed" => Self::Missed,
            "timeout" => Self::Timeout,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Missed => "missed",
            Self::Timeout => "timeout",
        }
    }

    pub fn finished(self) -> bool {
        !matches!(self, Self::InProgress)
    }
}

/// How a task is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// No run has finished yet.
    New,
    Ok,
    /// Its last run failed or took too long.
    Failing,
    /// A run did not start when it should have.
    Missed,
}

impl Health {
    pub fn as_str(self) -> &'static str {
        match self {
            Health::New => "new",
            Health::Ok => "ok",
            Health::Failing => "failing",
            Health::Missed => "missed",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "ok" => Health::Ok,
            "failing" => Health::Failing,
            "missed" => Health::Missed,
            _ => Health::New,
        }
    }

    /// Whether it is in trouble, for notifications.
    pub fn bad(self) -> bool {
        matches!(self, Health::Failing | Health::Missed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub id: MonitorId,
    pub scope: Scope,
    /// As the SDK names it: `scheduled_nightly-report`.
    pub slug: String,
    /// Without a configuration (`->sentryMonitor()` without a schedule),
    /// missed runs cannot be told.
    pub config: Option<MonitorConfig>,
    pub health: Health,
    pub created_at: Timestamp,
    pub last_check_in: Option<Timestamp>,
    pub last_status: Option<CheckInStatus>,
    /// Runs are expected after this: the last start, or the last run
    /// counted as missed.
    pub expected_after: Timestamp,
}

impl Monitor {
    /// The slug without the prefix sentry-laravel gives scheduled tasks.
    pub fn name(&self) -> &str {
        self.slug.strip_prefix("scheduled_").unwrap_or(&self.slug)
    }

    /// Takes a check-in received at `at`; returns the health before.
    pub fn record(&mut self, status: CheckInStatus, started: Timestamp, at: Timestamp) -> Health {
        let before = self.health;
        self.last_check_in = Some(at);
        self.last_status = Some(status);
        if started > self.expected_after {
            self.expected_after = started;
        }
        self.health = match status {
            CheckInStatus::Ok => Health::Ok,
            CheckInStatus::Error | CheckInStatus::Timeout => Health::Failing,
            CheckInStatus::Missed => Health::Missed,
            // A run that starts after a missed one: missed until it is done.
            CheckInStatus::InProgress => before,
        };
        before
    }

    /// The run that should have started by `now` and did not: its expected
    /// start. `offset` is the schedule's time zone's offset (see
    /// [`Schedule::next_run`]).
    pub fn missed_run(&self, now: Timestamp, offset: &dyn Fn(Timestamp) -> i32) -> Option<Timestamp> {
        let config = self.config.as_ref()?;
        // The run of `expected_after` itself may still be reported.
        let after = self.expected_after.saturating_add(Duration::from_secs(1));
        let next = config.schedule.next_run(after, offset)?;
        (now >= next.saturating_add(config.margin)).then_some(next)
    }
}

/// A run of a task, or a run that did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckIn {
    pub monitor: MonitorId,
    /// The SDK's ID, shared by a run's `in_progress` and its end; Logdock's
    /// own for missed runs.
    pub check_in_id: String,
    pub status: CheckInStatus,
    pub started: Timestamp,
    pub finished: Option<Timestamp>,
    pub duration: Option<Duration>,
    pub release: Option<String>,
    pub trace_id: Option<String>,
}

/// A check-in as the SDK sends it (`check_in` item).
#[derive(Debug, Clone, PartialEq)]
pub struct NewCheckIn {
    pub check_in_id: String,
    pub slug: String,
    pub status: CheckInStatus,
    /// Seconds, when the SDK measured them.
    pub duration: Option<Duration>,
    pub release: Option<String>,
    pub environment: Option<String>,
    pub trace_id: Option<String>,
    pub config: Option<MonitorConfig>,
}

impl NewCheckIn {
    pub fn from_payload(payload: &Value) -> Option<NewCheckIn> {
        let text = |value: Option<&Value>| {
            value.and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_owned)
        };
        let slug = text(payload.get("monitor_slug"))?;
        let status = CheckInStatus::parse(payload.get("status")?.as_str()?)?;
        if matches!(status, CheckInStatus::Missed | CheckInStatus::Timeout) {
            return None;
        }
        Some(NewCheckIn {
            check_in_id: text(payload.get("check_in_id")).unwrap_or_default().replace('-', ""),
            slug: slug.chars().take(200).collect(),
            status,
            duration: payload
                .get("duration")
                .and_then(Value::as_f64)
                .filter(|d| *d >= 0.0 && d.is_finite())
                .map(Duration::from_secs_f64),
            release: text(payload.get("release")),
            environment: text(payload.get("environment")),
            trace_id: text(payload.pointer(&["contexts", "trace", "trace_id"])),
            config: payload.get("monitor_config").and_then(MonitorConfig::from_payload),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use logdock_kernel::{EnvironmentId, ProjectId};

    use super::*;

    pub(crate) fn object(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    pub(crate) fn s(text: &str) -> Value {
        Value::String(text.to_owned())
    }

    /// A check-in as sentry-laravel 4.30 sends it (dev/laravel, schedule:run).
    pub(crate) fn check_in(id: &str, slug: &str, status: &str, crontab: &str) -> Value {
        object(vec![
            ("check_in_id", s(id)),
            ("monitor_slug", s(slug)),
            ("status", s(status)),
            ("duration", Value::Null),
            ("release", s("dev-1")),
            ("environment", s("production")),
            (
                "monitor_config",
                object(vec![
                    (
                        "schedule",
                        object(vec![("type", s("crontab")), ("value", s(crontab)), ("unit", s(""))]),
                    ),
                    ("checkin_margin", Value::Null),
                    ("max_runtime", Value::Null),
                    ("timezone", s("UTC")),
                ]),
            ),
            (
                "contexts",
                object(vec![("trace", object(vec![("trace_id", s("be92ff9330ac40d6b76e2551b2200697"))]))]),
            ),
        ])
    }

    fn at(text: &str) -> Timestamp {
        Timestamp::parse_rfc3339(text).unwrap()
    }

    fn monitor(crontab: &str, expected_after: &str) -> Monitor {
        let new = NewCheckIn::from_payload(&check_in("a", "scheduled_report", "ok", crontab)).unwrap();
        Monitor {
            id: MonitorId::new(1),
            scope: Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) },
            slug: new.slug,
            config: new.config,
            health: Health::New,
            created_at: at(expected_after),
            last_check_in: None,
            last_status: None,
            expected_after: at(expected_after),
        }
    }

    #[test]
    fn reads_sentry_laravels_check_ins() {
        let new = NewCheckIn::from_payload(&check_in(
            "1f02-1dc4",
            "scheduled_heartbeat",
            "in_progress",
            "* * * * *",
        ))
        .unwrap();
        assert_eq!(new.check_in_id, "1f021dc4");
        assert_eq!(new.status, CheckInStatus::InProgress);
        assert_eq!(new.trace_id.as_deref(), Some("be92ff9330ac40d6b76e2551b2200697"));
        let config = new.config.unwrap();
        assert_eq!(config.schedule.text(), "* * * * *");
        assert_eq!(
            (config.margin, config.max_runtime),
            (MonitorConfig::DEFAULT_MARGIN, MonitorConfig::DEFAULT_MAX_RUNTIME)
        );
        assert!(
            NewCheckIn::from_payload(&check_in("1", "x", "missed", "* * * * *")).is_none(),
            "missed is Logdock's"
        );
        assert!(NewCheckIn::from_payload(&object(vec![("status", s("ok"))])).is_none());
        let interval = object(vec![
            (
                "schedule",
                object(vec![("type", s("interval")), ("value", Value::Int(10)), ("unit", s("minute"))]),
            ),
            ("checkin_margin", Value::Int(5)),
            ("max_runtime", Value::Float(1.5)),
            ("timezone", s("Europe/Berlin")),
        ]);
        let config = MonitorConfig::from_payload(&interval).unwrap();
        assert_eq!(config.schedule.text(), "every 10 minute");
        assert_eq!(config.margin, Duration::from_secs(300));
        assert_eq!(config.max_runtime, Duration::from_secs(90));
        assert_eq!(config.timezone, "Europe/Berlin");
        for status in ["in_progress", "ok", "error", "missed", "timeout"] {
            assert_eq!(CheckInStatus::parse(status).unwrap().as_str(), status);
        }
    }

    #[test]
    fn health_follows_the_check_ins() {
        let mut m = monitor("0 3 * * *", "2026-11-19T03:00:00Z");
        assert_eq!(m.name(), "report");
        assert_eq!(
            m.record(CheckInStatus::InProgress, at("2026-11-20T03:00:01Z"), at("2026-11-20T03:00:01Z")),
            Health::New
        );
        assert_eq!(m.health, Health::New);
        m.record(CheckInStatus::Error, at("2026-11-20T03:00:01Z"), at("2026-11-20T03:00:09Z"));
        assert_eq!(m.health, Health::Failing);
        assert!(m.health.bad());
        assert_eq!(
            m.record(CheckInStatus::Ok, at("2026-11-21T03:00:00Z"), at("2026-11-21T03:00:05Z")),
            Health::Failing
        );
        assert_eq!(m.health, Health::Ok);
        assert_eq!(m.expected_after, at("2026-11-21T03:00:00Z"));
        for health in [Health::New, Health::Ok, Health::Failing, Health::Missed] {
            assert_eq!(Health::parse(health.as_str()), health);
        }
    }

    #[test]
    fn tells_a_run_that_did_not_happen() {
        let m = monitor("0 3 * * *", "2026-11-19T03:00:00Z");
        assert_eq!(m.missed_run(at("2026-11-20T03:01:00Z"), &|_| 0), None, "within the margin");
        assert_eq!(m.missed_run(at("2026-11-20T03:02:00Z"), &|_| 0), Some(at("2026-11-20T03:00:00Z")));
        let every_minute = monitor("* * * * *", "2026-11-19T03:00:00Z");
        assert_eq!(
            every_minute.missed_run(at("2026-11-19T03:03:30Z"), &|_| 0),
            Some(at("2026-11-19T03:01:00Z"))
        );
        let mut unscheduled = monitor("* * * * *", "2026-11-19T03:00:00Z");
        unscheduled.config = None;
        assert_eq!(unscheduled.missed_run(at("2027-01-01T00:00:00Z"), &|_| 0), None);
    }
}
