//! The Monitors context's adapters: monitors and check-ins in the main
//! database, whom to tell from the project's settings, and time zones from
//! the IANA database bundled with jiff.

use std::time::Duration;

use async_trait::async_trait;
use logdock_kernel::{EnvironmentId, PortError, ProjectId, Scope, Timestamp};
use logdock_monitors::application::ports::{
    MonitorQuery, MonitorRepository, NoticeTarget, NoticeTargets, TimeZones,
};
use logdock_monitors::domain::{
    CheckIn, CheckInStatus, Crontab, Health, IntervalUnit, Monitor, MonitorConfig, MonitorId, Schedule,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite};

use crate::sqlite::Database;

fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("monitors", error)
}

/// [`MonitorRepository`] on `monitors` and `check_ins`.
#[derive(Debug, Clone)]
pub struct SqliteMonitors {
    db: Database,
}

impl SqliteMonitors {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

const COLUMNS: &str = "id, project_id, environment_id, slug, schedule_type, schedule, interval_unit, timezone, \
    margin_s, max_runtime_s, health, created_at, last_check_in, last_status, expected_after";

fn config_of(row: &SqliteRow) -> Option<MonitorConfig> {
    let schedule: Option<String> = row.get("schedule");
    let schedule = match row.get::<Option<String>, _>("schedule_type").as_deref() {
        Some("interval") => Schedule::Interval {
            every: schedule?.parse().ok()?,
            unit: IntervalUnit::parse(&row.get::<Option<String>, _>("interval_unit")?)?,
        },
        Some(_) => Schedule::Crontab(Crontab::parse(&schedule?).ok()?),
        None => return None,
    };
    let seconds = |column: &str, default: Duration| {
        row.get::<Option<i64>, _>(column).map_or(default, |s| Duration::from_secs(s.max(0) as u64))
    };
    Some(MonitorConfig {
        schedule,
        timezone: row.get::<Option<String>, _>("timezone").unwrap_or_else(|| "UTC".to_owned()),
        margin: seconds("margin_s", MonitorConfig::DEFAULT_MARGIN),
        max_runtime: seconds("max_runtime_s", MonitorConfig::DEFAULT_MAX_RUNTIME),
    })
}

fn monitor(row: &SqliteRow) -> Monitor {
    Monitor {
        id: MonitorId::new(row.get("id")),
        scope: Scope {
            project: ProjectId::new(row.get("project_id")),
            environment: EnvironmentId::new(row.get("environment_id")),
        },
        slug: row.get("slug"),
        config: config_of(row),
        health: Health::parse(row.get::<&str, _>("health")),
        created_at: Timestamp::from_unix_micros(row.get("created_at")),
        last_check_in: row.get::<Option<i64>, _>("last_check_in").map(Timestamp::from_unix_micros),
        last_status: row.get::<Option<String>, _>("last_status").as_deref().and_then(CheckInStatus::parse),
        expected_after: Timestamp::from_unix_micros(row.get("expected_after")),
    }
}

fn check_in(row: &SqliteRow) -> CheckIn {
    CheckIn {
        monitor: MonitorId::new(row.get("monitor_id")),
        check_in_id: row.get("check_in_id"),
        status: CheckInStatus::parse(row.get::<&str, _>("status")).unwrap_or(CheckInStatus::Error),
        started: Timestamp::from_unix_micros(row.get("started")),
        finished: row.get::<Option<i64>, _>("finished").map(Timestamp::from_unix_micros),
        duration: row.get::<Option<i64>, _>("duration_us").map(|us| Duration::from_micros(us.max(0) as u64)),
        release: row.get("release"),
        trace_id: row.get("trace_id"),
    }
}

/// The columns of a configuration: type, schedule, unit, zone, margin,
/// maximum runtime.
type ConfigColumns =
    (Option<&'static str>, Option<String>, Option<&'static str>, Option<String>, Option<i64>, Option<i64>);

fn config_columns(config: Option<&MonitorConfig>) -> ConfigColumns {
    let Some(config) = config else { return (None, None, None, None, None, None) };
    let (kind, schedule, unit) = match &config.schedule {
        Schedule::Crontab(crontab) => ("crontab", crontab.text.clone(), None),
        Schedule::Interval { every, unit } => ("interval", every.to_string(), Some(unit.as_str())),
    };
    (
        Some(kind),
        Some(schedule),
        unit,
        Some(config.timezone.clone()),
        Some(config.margin.as_secs() as i64),
        Some(config.max_runtime.as_secs() as i64),
    )
}

#[async_trait]
impl MonitorRepository for SqliteMonitors {
    async fn find(&self, scope: Scope, slug: &str) -> Result<Option<Monitor>, PortError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM monitors WHERE project_id = ? AND environment_id = ? AND slug = ?"
        )))
        .bind(scope.project.get())
        .bind(scope.environment.get())
        .bind(slug)
        .fetch_optional(self.db.pool())
        .await
        .map_err(port)?;
        Ok(row.as_ref().map(monitor))
    }

    async fn create(
        &self,
        scope: Scope,
        slug: &str,
        config: Option<&MonitorConfig>,
        at: Timestamp,
    ) -> Result<Monitor, PortError> {
        let (kind, schedule, unit, zone, margin, max_runtime) = config_columns(config);
        sqlx::query(concat!(
            "INSERT INTO monitors (project_id, environment_id, slug, schedule_type, schedule, interval_unit, ",
            "timezone, margin_s, max_runtime_s, created_at, expected_after) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ",
            "ON CONFLICT (project_id, environment_id, slug) DO NOTHING"
        ))
        .bind(scope.project.get())
        .bind(scope.environment.get())
        .bind(slug)
        .bind(kind)
        .bind(schedule)
        .bind(unit)
        .bind(zone)
        .bind(margin)
        .bind(max_runtime)
        .bind(at.unix_micros())
        .bind(at.unix_micros())
        .execute(self.db.pool())
        .await
        .map_err(port)?;
        self.find(scope, slug).await?.ok_or_else(|| port("the monitor's project or stage is gone"))
    }

    async fn save(&self, m: &Monitor) -> Result<(), PortError> {
        let (kind, schedule, unit, zone, margin, max_runtime) = config_columns(m.config.as_ref());
        sqlx::query(concat!(
            "UPDATE monitors SET schedule_type = ?, schedule = ?, interval_unit = ?, timezone = ?, margin_s = ?, ",
            "max_runtime_s = ?, health = ?, last_check_in = ?, last_status = ?, expected_after = ? WHERE id = ?"
        ))
        .bind(kind)
        .bind(schedule)
        .bind(unit)
        .bind(zone)
        .bind(margin)
        .bind(max_runtime)
        .bind(m.health.as_str())
        .bind(m.last_check_in.map(Timestamp::unix_micros))
        .bind(m.last_status.map(CheckInStatus::as_str))
        .bind(m.expected_after.unix_micros())
        .bind(m.id.get())
        .execute(self.db.pool())
        .await
        .map_err(port)?;
        Ok(())
    }

    async fn get(&self, id: MonitorId) -> Result<Option<Monitor>, PortError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM monitors WHERE id = ?")))
            .bind(id.get())
            .fetch_optional(self.db.pool())
            .await
            .map_err(port)?;
        Ok(row.as_ref().map(monitor))
    }

    async fn list(&self, query: &MonitorQuery) -> Result<Vec<Monitor>, PortError> {
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT ");
        sql.push(COLUMNS).push(" FROM monitors WHERE 1 = 1");
        if !query.projects.is_empty() {
            sql.push(" AND project_id IN ");
            sql.push_tuples(&query.projects, |mut row, id| {
                row.push_bind(id.get());
            });
        }
        if !query.environments.is_empty() {
            sql.push(" AND environment_id IN ");
            sql.push_tuples(&query.environments, |mut row, id| {
                row.push_bind(id.get());
            });
        }
        sql.push(" ORDER BY slug");
        let rows = sql.build().fetch_all(self.db.pool()).await.map_err(port)?;
        Ok(rows.iter().map(monitor).collect())
    }

    async fn scheduled(&self) -> Result<Vec<Monitor>, PortError> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM monitors WHERE schedule_type IS NOT NULL"
        )))
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        Ok(rows.iter().map(monitor).collect())
    }

    async fn delete(&self, id: MonitorId) -> Result<(), PortError> {
        sqlx::query("DELETE FROM monitors WHERE id = ?")
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn record(&self, c: &CheckIn) -> Result<(), PortError> {
        sqlx::query(concat!(
            "INSERT INTO check_ins (monitor_id, check_in_id, status, started, finished, duration_us, release, trace_id) ",
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (monitor_id, check_in_id) DO UPDATE SET ",
            "status = excluded.status, started = excluded.started, finished = excluded.finished, ",
            "duration_us = excluded.duration_us, release = coalesce(excluded.release, release), ",
            "trace_id = coalesce(excluded.trace_id, trace_id)"
        ))
        .bind(c.monitor.get())
        .bind(&c.check_in_id)
        .bind(c.status.as_str())
        .bind(c.started.unix_micros())
        .bind(c.finished.map(Timestamp::unix_micros))
        .bind(c.duration.map(|d| d.as_micros().min(i64::MAX as u128) as i64))
        .bind(&c.release)
        .bind(&c.trace_id)
        .execute(self.db.pool())
        .await
        .map_err(port)?;
        Ok(())
    }

    async fn check_in(&self, monitor: MonitorId, check_in_id: &str) -> Result<Option<CheckIn>, PortError> {
        let row = sqlx::query("SELECT * FROM check_ins WHERE monitor_id = ? AND check_in_id = ?")
            .bind(monitor.get())
            .bind(check_in_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(port)?;
        Ok(row.as_ref().map(check_in))
    }

    async fn check_ins(&self, monitor: MonitorId, limit: u32) -> Result<Vec<CheckIn>, PortError> {
        let rows = sqlx::query("SELECT * FROM check_ins WHERE monitor_id = ? ORDER BY started DESC LIMIT ?")
            .bind(monitor.get())
            .bind(i64::from(limit))
            .fetch_all(self.db.pool())
            .await
            .map_err(port)?;
        Ok(rows.iter().map(check_in).collect())
    }

    async fn running(&self, before: Timestamp) -> Result<Vec<CheckIn>, PortError> {
        let rows = sqlx::query("SELECT * FROM check_ins WHERE status = 'in_progress' AND started < ?")
            .bind(before.unix_micros())
            .fetch_all(self.db.pool())
            .await
            .map_err(port)?;
        Ok(rows.iter().map(check_in).collect())
    }
}

/// [`NoticeTargets`] from the project's settings (`notify`,
/// `notify_monitors`).
#[derive(Debug, Clone)]
pub struct ProjectNoticeTargets {
    db: Database,
}

impl ProjectNoticeTargets {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl NoticeTargets for ProjectNoticeTargets {
    async fn target(&self, scope: Scope) -> Result<Option<NoticeTarget>, PortError> {
        let row = sqlx::query(
            "SELECT p.name AS project, p.settings, e.name AS environment FROM projects p \
             JOIN environments e ON e.project_id = p.id WHERE p.id = ? AND e.id = ?",
        )
        .bind(scope.project.get())
        .bind(scope.environment.get())
        .fetch_optional(self.db.pool())
        .await
        .map_err(port)?;
        let Some(row) = row else { return Ok(None) };
        let settings: serde_json::Value =
            serde_json::from_str(row.get::<&str, _>("settings")).unwrap_or(serde_json::Value::Null);
        let recipients = settings
            .get("notify")
            .and_then(serde_json::Value::as_array)
            .map(|list| list.iter().filter_map(|a| a.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        Ok(Some(NoticeTarget {
            recipients,
            enabled: settings.get("notify_monitors").and_then(serde_json::Value::as_bool).unwrap_or(true),
            project: row.get("project"),
            environment: row.get("environment"),
        }))
    }
}

/// [`TimeZones`] from the IANA time zone database built into the binary,
/// so the image needs none of its own.
#[derive(Debug, Default, Clone, Copy)]
pub struct BundledTimeZones;

impl TimeZones for BundledTimeZones {
    fn offset(&self, zone: &str, at: Timestamp) -> Option<i32> {
        let zone = jiff::tz::TimeZone::get(zone).ok()?;
        let at = jiff::Timestamp::from_microsecond(at.unix_micros()).ok()?;
        Some(zone.to_offset(at).seconds())
    }
}

#[cfg(test)]
mod tests {
    use logdock_kernel::Value;
    use logdock_monitors::domain::NewCheckIn;

    use super::*;
    use crate::sqlite::repository_tests::database;

    fn config(crontab: &str) -> MonitorConfig {
        let payload = Value::Object(vec![
            ("monitor_slug".into(), Value::String("scheduled_x".into())),
            ("status".into(), Value::String("ok".into())),
            (
                "monitor_config".into(),
                Value::Object(vec![
                    (
                        "schedule".into(),
                        Value::Object(vec![
                            ("type".into(), Value::String("crontab".into())),
                            ("value".into(), Value::String(crontab.into())),
                        ]),
                    ),
                    ("timezone".into(), Value::String("Europe/Berlin".into())),
                    ("checkin_margin".into(), Value::Int(5)),
                ]),
            ),
        ]);
        NewCheckIn::from_payload(&payload).unwrap().config.unwrap()
    }

    #[tokio::test]
    async fn keeps_monitors_and_their_check_ins() {
        let (db, _dir) = database().await;
        sqlx::query("INSERT INTO projects (id, name, created_at, settings) VALUES (1, 'orders', 0, ?)")
            .bind(r#"{"notify":["ops@example.com"],"notify_monitors":true}"#)
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, created_at) VALUES (2, 1, 'production', 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        let monitors = SqliteMonitors::new(db.clone());
        let scope = Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) };
        let at = Timestamp::from_unix_seconds(1_800_000_000);
        let config = config("0 3 * * *");
        let mut created = monitors.create(scope, "scheduled_x", Some(&config), at).await.unwrap();
        assert_eq!(monitors.create(scope, "scheduled_x", None, at).await.unwrap().id, created.id, "once");
        assert_eq!(created.config.as_ref(), Some(&config));
        assert_eq!(created.config.as_ref().unwrap().margin, Duration::from_secs(300));
        created.health = Health::Failing;
        created.last_status = Some(CheckInStatus::Error);
        created.config = Some(MonitorConfig {
            schedule: Schedule::Interval { every: 10, unit: IntervalUnit::Minute },
            ..config
        });
        monitors.save(&created).await.unwrap();
        let loaded = monitors.get(created.id).await.unwrap().unwrap();
        assert_eq!(loaded, created);
        assert_eq!(monitors.scheduled().await.unwrap().len(), 1);
        assert_eq!(monitors.list(&MonitorQuery::default()).await.unwrap().len(), 1);
        let none = MonitorQuery { projects: vec![ProjectId::new(5)], environments: vec![] };
        assert!(monitors.list(&none).await.unwrap().is_empty());

        let mut run = CheckIn {
            monitor: created.id,
            check_in_id: "run1".into(),
            status: CheckInStatus::InProgress,
            started: at,
            finished: None,
            duration: None,
            release: Some("1.0".into()),
            trace_id: None,
        };
        monitors.record(&run).await.unwrap();
        assert_eq!(monitors.running(at.saturating_add(Duration::from_secs(1))).await.unwrap().len(), 1);
        run.status = CheckInStatus::Ok;
        run.finished = Some(at.saturating_add(Duration::from_secs(4)));
        run.duration = Some(Duration::from_secs(4));
        run.release = None;
        monitors.record(&run).await.unwrap();
        let kept = monitors.check_in(created.id, "run1").await.unwrap().unwrap();
        assert_eq!(kept.status, CheckInStatus::Ok);
        assert_eq!(kept.release.as_deref(), Some("1.0"), "kept from the start");
        assert!(monitors.running(at.saturating_add(Duration::from_secs(10))).await.unwrap().is_empty());
        assert_eq!(monitors.check_ins(created.id, 10).await.unwrap().len(), 1);

        let target = ProjectNoticeTargets::new(db).target(scope).await.unwrap().unwrap();
        assert_eq!(target.recipients, ["ops@example.com"]);
        assert!(target.enabled);
        assert_eq!((target.project.as_str(), target.environment.as_str()), ("orders", "production"));

        monitors.delete(created.id).await.unwrap();
        assert!(monitors.get(created.id).await.unwrap().is_none());
        assert!(monitors.check_ins(created.id, 10).await.unwrap().is_empty(), "with its check-ins");
    }

    #[test]
    fn knows_time_zones_and_their_daylight_saving_time() {
        let zones = BundledTimeZones;
        let winter = Timestamp::parse_rfc3339("2026-11-19T12:00:00Z").unwrap();
        let summer = Timestamp::parse_rfc3339("2026-07-01T12:00:00Z").unwrap();
        assert_eq!(zones.offset("Europe/Berlin", winter), Some(3600));
        assert_eq!(zones.offset("Europe/Berlin", summer), Some(7200));
        assert_eq!(zones.offset("UTC", summer), Some(0));
        assert_eq!(zones.offset("America/New_York", winter), Some(-5 * 3600));
        assert_eq!(zones.offset("Mars/Olympus_Mons", winter), None);
    }
}
