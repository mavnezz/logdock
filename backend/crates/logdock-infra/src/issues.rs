//! The Issues context's adapters: issues and their counts in the main
//! database, events in the day segments.

use std::sync::Arc;

use async_trait::async_trait;
use logdock_issues::application::ports::{
    EventStore, EventSummary, IssueQuery, IssueRepository, IssueSort, IssueSummary, ReleaseCount, StoredEvent,
};
use logdock_issues::domain::{ErrorEvent, EventLevel, Issue, IssueId, IssueStatus, NewIssue};
use logdock_kernel::{Day, EnvironmentId, PortError, ProjectId, Scope, Timestamp, Value};
use logdock_retention::domain::Kind;
use sqlx::sqlite::SqliteRow;
use sqlx::{Connection, QueryBuilder, Row, Sqlite, SqliteConnection};
use tokio::sync::Mutex;

use crate::json::{from_json, packed, to_json, unpacked};
use crate::segments::Segments;
use crate::sqlite::Database;

fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("issues", error)
}

fn json_text(value: &Value) -> String {
    to_json(value).to_string()
}

fn value_of(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok().map(from_json)
}

/// [`IssueRepository`] on the `issues` tables.
#[derive(Debug, Clone)]
pub struct SqliteIssues {
    db: Database,
}

impl SqliteIssues {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn issue(row: &SqliteRow) -> Issue {
    let status: String = row.get("status");
    let at = |column: &str| row.get::<Option<i64>, _>(column).map(Timestamp::from_unix_micros);
    let status = match status.as_str() {
        "resolved" => IssueStatus::Resolved {
            at: at("resolved_at").unwrap_or_default(),
            in_next_release: row.get("resolved_in_next_release"),
        },
        "muted" => IssueStatus::Muted {
            until: at("muted_until"),
            remaining: row.get::<Option<i64>, _>("muted_remaining").map(|n| n.max(0) as u32),
        },
        _ => IssueStatus::Unresolved,
    };
    Issue {
        id: IssueId::new(row.get("id")),
        scope: Scope {
            project: ProjectId::new(row.get("project_id")),
            environment: EnvironmentId::new(row.get("environment_id")),
        },
        fingerprint: row.get("fingerprint"),
        title: row.get("title"),
        culprit: row.get("culprit"),
        level: EventLevel::parse(row.get("level")).unwrap_or(EventLevel::Error),
        status,
        first_seen: Timestamp::from_unix_micros(row.get("first_seen")),
        last_seen: Timestamp::from_unix_micros(row.get("last_seen")),
        count: row.get::<i64, _>("count") as u64,
        first_release: row.get("first_release"),
        last_release: row.get("last_release"),
        regressed_at: at("regressed_at"),
        last_event_id: row.get("last_event_id"),
    }
}

const COLUMNS: &str = "id, project_id, environment_id, fingerprint, title, culprit, level, status, resolved_at, \
    resolved_in_next_release, muted_until, muted_remaining, first_seen, last_seen, count, first_release, last_release, \
    regressed_at, last_event_id";

#[async_trait]
impl IssueRepository for SqliteIssues {
    async fn find(&self, scope: Scope, fingerprint: &str) -> Result<Option<Issue>, PortError> {
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT ");
        sql.push(COLUMNS).push(" FROM issues WHERE project_id = ").push_bind(scope.project.get());
        sql.push(" AND environment_id = ").push_bind(scope.environment.get());
        sql.push(" AND fingerprint = ").push_bind(fingerprint.to_owned());
        let row = sql.build().fetch_optional(self.db.pool()).await.map_err(port)?;
        Ok(row.as_ref().map(issue))
    }

    async fn create(&self, new: NewIssue, event: &Value) -> Result<Option<Issue>, PortError> {
        let id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO issues (project_id, environment_id, fingerprint, title, culprit, level, first_seen, last_seen, \
             count, first_release, last_release, last_event_id, last_event) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?) \
             ON CONFLICT (project_id, environment_id, fingerprint) DO NOTHING RETURNING id",
        )
        .bind(new.scope.project.get())
        .bind(new.scope.environment.get())
        .bind(&new.fingerprint)
        .bind(&new.title)
        .bind(&new.culprit)
        .bind(new.level.as_str())
        .bind(new.seen.unix_micros())
        .bind(new.seen.unix_micros())
        .bind(&new.release)
        .bind(&new.release)
        .bind(&new.event_id)
        .bind(json_text(event))
        .fetch_optional(self.db.pool())
        .await
        .map_err(port)?;
        match id {
            Some(id) => self.get(IssueId::new(id)).await,
            None => Ok(None),
        }
    }

    async fn save(&self, issue: &Issue, event: Option<&Value>) -> Result<(), PortError> {
        let (resolved_at, next_release, muted_until, muted_remaining) = match &issue.status {
            IssueStatus::Unresolved => (None, None, None, None),
            IssueStatus::Resolved { at, in_next_release } => {
                (Some(at.unix_micros()), in_next_release.clone(), None, None)
            }
            IssueStatus::Muted { until, remaining } => {
                (None, None, until.map(Timestamp::unix_micros), remaining.map(i64::from))
            }
        };
        sqlx::query(
            "UPDATE issues SET level = ?, status = ?, resolved_at = ?, resolved_in_next_release = ?, muted_until = ?, \
             muted_remaining = ?, first_seen = ?, last_seen = ?, count = ?, first_release = ?, last_release = ?, \
             regressed_at = ?, last_event_id = ?, last_event = coalesce(?, last_event) WHERE id = ?",
        )
        .bind(issue.level.as_str())
        .bind(issue.status.name())
        .bind(resolved_at)
        .bind(next_release)
        .bind(muted_until)
        .bind(muted_remaining)
        .bind(issue.first_seen.unix_micros())
        .bind(issue.last_seen.unix_micros())
        .bind(issue.count as i64)
        .bind(&issue.first_release)
        .bind(&issue.last_release)
        .bind(issue.regressed_at.map(Timestamp::unix_micros))
        .bind(&issue.last_event_id)
        .bind(event.map(json_text))
        .bind(issue.id.get())
        .execute(self.db.pool())
        .await
        .map_err(port)?;
        Ok(())
    }

    async fn get(&self, id: IssueId) -> Result<Option<Issue>, PortError> {
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT ");
        sql.push(COLUMNS).push(" FROM issues WHERE id = ").push_bind(id.get());
        let row = sql.build().fetch_optional(self.db.pool()).await.map_err(port)?;
        Ok(row.as_ref().map(issue))
    }

    async fn list(&self, query: &IssueQuery, since: Timestamp) -> Result<Vec<IssueSummary>, PortError> {
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT ");
        for column in COLUMNS.split(", ") {
            sql.push("i.").push(column.trim()).push(", ");
        }
        sql.push(
            "(SELECT coalesce(sum(h.count), 0) FROM issue_hours h WHERE h.issue_id = i.id AND h.hour >= ",
        )
        .push_bind(since.hour())
        .push(") AS last_day FROM issues i WHERE 1 = 1");
        if !query.projects.is_empty() {
            sql.push(" AND i.project_id IN (");
            let mut list = sql.separated(", ");
            for project in &query.projects {
                list.push_bind(project.get());
            }
            list.push_unseparated(")");
        }
        if !query.environments.is_empty() {
            sql.push(" AND i.environment_id IN (");
            let mut list = sql.separated(", ");
            for environment in &query.environments {
                list.push_bind(environment.get());
            }
            list.push_unseparated(")");
        }
        if let Some(status) = &query.status {
            sql.push(" AND i.status = ").push_bind(status.clone());
        }
        if let Some(text) = query.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            for word in text.split_whitespace() {
                let pattern = format!("%{}%", word.replace('%', "\\%").replace('_', "\\_"));
                sql.push(" AND (i.title LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\'");
                sql.push(" OR i.culprit LIKE ").push_bind(pattern).push(" ESCAPE '\\')");
            }
        }
        sql.push(match query.sort {
            IssueSort::LastSeen => " ORDER BY i.last_seen DESC",
            IssueSort::FirstSeen => " ORDER BY i.first_seen DESC",
            IssueSort::Count => " ORDER BY i.count DESC",
        });
        sql.push(" LIMIT ")
            .push_bind(i64::from(query.limit))
            .push(" OFFSET ")
            .push_bind(i64::from(query.offset));
        let rows = sql.build().fetch_all(self.db.pool()).await.map_err(port)?;
        Ok(rows
            .iter()
            .map(|row| IssueSummary { issue: issue(row), last_day: row.get::<i64, _>("last_day") as u64 })
            .collect())
    }

    async fn delete(&self, id: IssueId) -> Result<(), PortError> {
        sqlx::query("DELETE FROM issues WHERE id = ?")
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn record(&self, id: IssueId, at: Timestamp, release: Option<&str>) -> Result<(), PortError> {
        let mut tx = self.db.pool().begin().await.map_err(port)?;
        sqlx::query(
            "INSERT INTO issue_hours (issue_id, hour, count) VALUES (?, ?, 1) \
             ON CONFLICT (issue_id, hour) DO UPDATE SET count = count + 1",
        )
        .bind(id.get())
        .bind(at.hour())
        .execute(&mut *tx)
        .await
        .map_err(port)?;
        if let Some(release) = release {
            sqlx::query(
                "INSERT INTO issue_releases (issue_id, release, count, first_seen, last_seen) VALUES (?1, ?2, 1, ?3, ?3) \
                 ON CONFLICT (issue_id, release) DO UPDATE SET count = count + 1, \
                 first_seen = min(first_seen, excluded.first_seen), last_seen = max(last_seen, excluded.last_seen)",
            )
            .bind(id.get())
            .bind(release)
            .bind(at.unix_micros())
            .execute(&mut *tx)
            .await
            .map_err(port)?;
        }
        tx.commit().await.map_err(port)
    }

    async fn hours(&self, id: IssueId, from: Timestamp, to: Timestamp) -> Result<Vec<(i64, u64)>, PortError> {
        let rows = sqlx::query(
            "SELECT hour, count FROM issue_hours WHERE issue_id = ? AND hour >= ? AND hour < ? ORDER BY hour",
        )
        .bind(id.get())
        .bind(from.hour())
        .bind(to.unix_seconds())
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        Ok(rows.iter().map(|r| (r.get("hour"), r.get::<i64, _>("count") as u64)).collect())
    }

    async fn releases(&self, id: IssueId) -> Result<Vec<ReleaseCount>, PortError> {
        let rows =
            sqlx::query("SELECT * FROM issue_releases WHERE issue_id = ? ORDER BY last_seen DESC LIMIT 50")
                .bind(id.get())
                .fetch_all(self.db.pool())
                .await
                .map_err(port)?;
        Ok(rows
            .iter()
            .map(|r| ReleaseCount {
                release: r.get("release"),
                count: r.get::<i64, _>("count") as u64,
                first_seen: Timestamp::from_unix_micros(r.get("first_seen")),
                last_seen: Timestamp::from_unix_micros(r.get("last_seen")),
            })
            .collect())
    }

    async fn latest_event(&self, id: IssueId) -> Result<Option<Value>, PortError> {
        let text: Option<Option<String>> = sqlx::query_scalar("SELECT last_event FROM issues WHERE id = ?")
            .bind(id.get())
            .fetch_optional(self.db.pool())
            .await
            .map_err(port)?;
        Ok(text.flatten().as_deref().and_then(value_of))
    }
}

/// [`EventStore`] over the events' day segments.
#[derive(Debug)]
pub struct SegmentEvents {
    segments: Arc<Segments>,
    /// Today's segment and the connection to it.
    writer: Mutex<Option<(Day, SqliteConnection)>>,
}

impl SegmentEvents {
    pub fn new(segments: Arc<Segments>) -> Self {
        Self { segments, writer: Mutex::new(None) }
    }

    pub async fn close(&self) {
        if let Some((_, connection)) = self.writer.lock().await.take() {
            let _ = connection.close().await;
        }
    }

    async fn days(&self) -> Result<Vec<Day>, PortError> {
        Ok(self.segments.overlapping(Kind::Events, None, None).await?.into_iter().map(|s| s.day).collect())
    }
}

fn summary(row: &SqliteRow) -> EventSummary {
    EventSummary {
        event_id: row.get("event_id"),
        issue: IssueId::new(row.get("issue_id")),
        scope: Scope {
            project: ProjectId::new(row.get("project_id")),
            environment: EnvironmentId::new(row.get("environment_id")),
        },
        time: Timestamp::from_unix_micros(row.get("time")),
        level: level_of(row.get("level")),
        release: row.get("release"),
        message: row.get("message"),
        transaction: row.get("transaction_name"),
        service: row.get("service"),
        host: row.get("host"),
        trace_id: row.get("trace_id"),
    }
}

fn level_number(level: EventLevel) -> i64 {
    match level {
        EventLevel::Debug => 1,
        EventLevel::Info => 2,
        EventLevel::Warning => 3,
        EventLevel::Error => 4,
        EventLevel::Fatal => 5,
    }
}

fn level_of(number: i64) -> EventLevel {
    match number {
        1 => EventLevel::Debug,
        2 => EventLevel::Info,
        3 => EventLevel::Warning,
        5 => EventLevel::Fatal,
        _ => EventLevel::Error,
    }
}

const SUMMARY: &str = "SELECT event_id, issue_id, project_id, environment_id, time, level, release, message, \
    transaction_name, service, host, trace_id FROM events";

#[async_trait]
impl EventStore for SegmentEvents {
    async fn insert(
        &self,
        issue: IssueId,
        scope: Scope,
        event: &ErrorEvent,
        time: Timestamp,
        received: Timestamp,
    ) -> Result<(), PortError> {
        let day = received.day();
        let mut writer = self.writer.lock().await;
        if writer.as_ref().is_none_or(|(current, _)| *current != day) {
            if let Some((_, old)) = writer.take() {
                let _ = old.close().await;
            }
            *writer = Some((day, self.segments.open_writer(Kind::Events, day).await?));
        }
        let (_, connection) = writer.as_mut().expect("opened above");
        let message = event.primary().map_or_else(
            || event.message.clone().unwrap_or_default(),
            |e| {
                if e.value.is_empty() { e.kind.clone() } else { format!("{}: {}", e.kind, e.value) }
            },
        );
        sqlx::query(
            "INSERT INTO events (event_id, issue_id, time, received, project_id, environment_id, level, release, service, \
             host, trace_id, transaction_name, message, data) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&event.event_id)
        .bind(issue.get())
        .bind(time.unix_micros())
        .bind(received.unix_micros())
        .bind(scope.project.get())
        .bind(scope.environment.get())
        .bind(level_number(event.level))
        .bind(&event.release)
        .bind(event.tag("service"))
        .bind(event.tag("host"))
        .bind(&event.trace_id)
        .bind(&event.transaction)
        .bind(message)
        .bind(packed(&event.payload))
        .execute(&mut *connection)
        .await
        .map_err(port)?;
        drop(writer);
        self.segments.note_batch(Kind::Events, day, 1, time, time).await
    }

    async fn get(&self, event_id: &str) -> Result<Option<StoredEvent>, PortError> {
        for day in self.days().await? {
            let Some(pool) = self.segments.reader(Kind::Events, day).await? else { continue };
            let row = sqlx::query("SELECT * FROM events WHERE event_id = ? LIMIT 1")
                .bind(event_id)
                .fetch_optional(&pool)
                .await
                .map_err(port)?;
            if let Some(row) = row {
                let data: Vec<u8> = row.get("data");
                let payload = unpacked(&data).unwrap_or(Value::Null);
                return Ok(Some(StoredEvent {
                    issue: IssueId::new(row.get("issue_id")),
                    scope: Scope {
                        project: ProjectId::new(row.get("project_id")),
                        environment: EnvironmentId::new(row.get("environment_id")),
                    },
                    received: Timestamp::from_unix_micros(row.get("received")),
                    event: ErrorEvent::from_payload(payload, Some(event_id)),
                }));
            }
        }
        Ok(None)
    }

    async fn seen(&self, event_id: &str, since: Timestamp) -> Result<Option<IssueId>, PortError> {
        // Duplicates come soon after: the segments of `since` and the day after.
        for day in [since.day(), Day::from_days(since.day().days() + 1)] {
            let Some(pool) = self.segments.reader(Kind::Events, day).await? else { continue };
            let issue: Option<i64> = sqlx::query_scalar(
                "SELECT issue_id FROM events WHERE event_id = ? AND received >= ? LIMIT 1",
            )
            .bind(event_id)
            .bind(since.unix_micros())
            .fetch_optional(&pool)
            .await
            .map_err(port)?;
            if let Some(issue) = issue {
                return Ok(Some(IssueId::new(issue)));
            }
        }
        Ok(None)
    }

    async fn list(
        &self,
        issue: IssueId,
        before: Option<Timestamp>,
        limit: u32,
    ) -> Result<Vec<EventSummary>, PortError> {
        let mut found: Vec<EventSummary> = Vec::new();
        for day in self.days().await? {
            let Some(pool) = self.segments.reader(Kind::Events, day).await? else { continue };
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
                "{SUMMARY} WHERE issue_id = ? AND time < ? ORDER BY time DESC LIMIT ?"
            )))
            .bind(issue.get())
            .bind(before.map_or(i64::MAX, Timestamp::unix_micros))
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(port)?;
            found.extend(rows.iter().map(summary));
            if found.len() >= limit as usize * 2 {
                break;
            }
        }
        found.sort_by_key(|e| std::cmp::Reverse(e.time));
        found.truncate(limit as usize);
        Ok(found)
    }

    async fn by_trace(&self, trace_id: &str) -> Result<Vec<EventSummary>, PortError> {
        let mut found = Vec::new();
        for day in self.days().await? {
            let Some(pool) = self.segments.reader(Kind::Events, day).await? else { continue };
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
                "{SUMMARY} WHERE trace_id = ? ORDER BY time LIMIT 100"
            )))
            .bind(trace_id)
            .fetch_all(&pool)
            .await
            .map_err(port)?;
            found.extend(rows.iter().map(summary));
        }
        found.sort_by_key(|e| e.time);
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use logdock_kernel::redaction::RedactionRules;

    use super::*;
    use crate::sqlite::repository_tests::database;

    fn payload(release: &str) -> Value {
        let json = serde_json::json!({
            "timestamp": 1_794_993_302.5,
            "level": "error",
            "release": release,
            "transaction": "POST /orders",
            "tags": {"service": "web", "host": "h1"},
            "contexts": {"trace": {"trace_id": "t1"}},
            "exception": {"values": [{"type": "RuntimeException", "value": "boom", "stacktrace": {"frames": [
                {"filename": "/app/X.php", "function": "go", "lineno": 3, "in_app": true}
            ]}}]}
        });
        ErrorEvent::prepare(from_json(json), &RedactionRules::default()).0
    }

    #[tokio::test]
    async fn keeps_issues_their_counts_and_their_events() {
        let (db, dir) = database().await;
        sqlx::query("INSERT INTO projects (id, name, created_at) VALUES (1, 'orders', 0)")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, created_at) VALUES (2, 1, 'production', 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        let issues = SqliteIssues::new(db.clone());
        let scope = Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) };
        let at = Timestamp::from_unix_seconds(86_400 * 20_000 + 3_700);
        let new = NewIssue {
            scope,
            fingerprint: "f1".into(),
            title: "RuntimeException: boom".into(),
            culprit: Some("go in /app/X.php".into()),
            level: EventLevel::Error,
            seen: at,
            release: Some("r1".into()),
            event_id: "e1".into(),
        };
        let mut issue = issues.create(new.clone(), &payload("r1")).await.unwrap().unwrap();
        assert!(issues.create(new, &payload("r1")).await.unwrap().is_none(), "one per fingerprint");
        assert_eq!(issues.find(scope, "f1").await.unwrap().unwrap().id, issue.id);

        issue.observe(at, EventLevel::Fatal, Some("r2"), "e2");
        issue.status = IssueStatus::Muted { until: Some(at), remaining: Some(3) };
        issues.save(&issue, Some(&payload("r2"))).await.unwrap();
        let stored = issues.get(issue.id).await.unwrap().unwrap();
        assert_eq!(stored, issue);
        issue.status = IssueStatus::Resolved { at, in_next_release: Some("r2".into()) };
        issues.save(&issue, None).await.unwrap();
        assert_eq!(issues.get(issue.id).await.unwrap().unwrap().status, issue.status);
        let latest = issues.latest_event(issue.id).await.unwrap().unwrap();
        assert_eq!(latest.get("release").and_then(Value::as_str), Some("r2"), "kept when saved without one");

        issues.record(issue.id, at, Some("r1")).await.unwrap();
        issues.record(issue.id, at, Some("r2")).await.unwrap();
        issues.record(issue.id, at.saturating_add(std::time::Duration::from_secs(3600)), None).await.unwrap();
        let hours = issues
            .hours(
                issue.id,
                Timestamp::from_unix_seconds(0),
                at.saturating_add(std::time::Duration::from_secs(7200)),
            )
            .await
            .unwrap();
        assert_eq!(hours, [(at.hour(), 2), (at.hour() + 3600, 1)]);
        assert_eq!(issues.releases(issue.id).await.unwrap().len(), 2);

        let query = IssueQuery {
            limit: 10,
            text: Some("boom".into()),
            status: Some("resolved".into()),
            projects: vec![scope.project],
            environments: vec![scope.environment],
            ..Default::default()
        };
        let listed = issues.list(&query, Timestamp::from_unix_seconds(0)).await.unwrap();
        assert_eq!((listed.len(), listed[0].last_day), (1, 3));
        for sort in [IssueSort::Count, IssueSort::FirstSeen] {
            assert_eq!(
                issues.list(&IssueQuery { limit: 10, sort, ..Default::default() }, at).await.unwrap().len(),
                1
            );
        }
        assert!(
            issues
                .list(&IssueQuery { limit: 10, text: Some("100%_x".into()), ..Default::default() }, at)
                .await
                .unwrap()
                .is_empty()
        );

        let segments = Arc::new(Segments::new(dir.path().join("segments"), db.clone()));
        let events = SegmentEvents::new(Arc::clone(&segments));
        let event = ErrorEvent::from_payload(payload("r2"), Some("E2"));
        events.insert(issue.id, scope, &event, at, at).await.unwrap();
        let second = ErrorEvent::from_payload(payload("r2"), Some("e3"));
        events
            .insert(issue.id, scope, &second, at.saturating_add(std::time::Duration::from_secs(1)), at)
            .await
            .unwrap();
        let listed = events.list(issue.id, None, 10).await.unwrap();
        assert_eq!(listed.iter().map(|e| e.event_id.as_str()).collect::<Vec<_>>(), ["e3", "e2"]);
        assert_eq!(listed[0].message, "RuntimeException: boom");
        assert_eq!(listed[0].service.as_deref(), Some("web"));
        let older = events.list(issue.id, Some(listed[0].time), 10).await.unwrap();
        assert_eq!(older.len(), 1);
        let found = events.get("e2").await.unwrap().unwrap();
        assert_eq!(found.event.exceptions[0].kind, "RuntimeException");
        assert!(events.get("nope").await.unwrap().is_none());
        assert_eq!(events.by_trace("t1").await.unwrap().len(), 2);
        events.close().await;

        issues.delete(issue.id).await.unwrap();
        assert!(issues.get(issue.id).await.unwrap().is_none());
    }
}
