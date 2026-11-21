//! The Issues context's adapters: issues and their counts in the main
//! database, events in the day segments.

use std::sync::Arc;

use async_trait::async_trait;
use logdock_issues::application::ports::{
    ActiveIssue, ActivityQuery, ErrorActivity, EventStore, EventSummary, IssueQuery, IssueRepository,
    IssueSort, IssueSummary, ReleaseCount, ReleaseErrors, StoredEvent, TagCount, TagKey,
};
use logdock_issues::domain::{
    ErrorEvent, EventLevel, Issue, IssueCategory, IssueId, IssueStatus, NewIssue, USER, tags_of, values_kept,
};
use logdock_kernel::{Day, EnvironmentId, PortError, ProjectId, Scope, Timestamp, Value};
use logdock_retention::domain::Kind;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use tokio::sync::Mutex;

use crate::json::{from_json, packed, to_json, unpacked};
use crate::segments::{SegmentId, Segments, Writers};
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
        category: IssueCategory::parse(row.get("category")).unwrap_or_default(),
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

const COLUMNS: &str = "id, project_id, environment_id, fingerprint, category, title, culprit, level, status, resolved_at, \
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
            "INSERT INTO issues (project_id, environment_id, fingerprint, category, title, culprit, level, first_seen, \
             last_seen, count, first_release, last_release, last_event_id, last_event) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?) \
             ON CONFLICT (project_id, environment_id, fingerprint) DO NOTHING RETURNING id",
        )
        .bind(new.scope.project.get())
        .bind(new.scope.environment.get())
        .bind(&new.fingerprint)
        .bind(new.category.as_str())
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
        .push(") AS last_day, coalesce((SELECT k.distinct_values FROM issue_tag_keys k WHERE k.issue_id = i.id AND k.key = ")
        .push_bind(USER)
        .push("), 0) AS users FROM issues i WHERE 1 = 1");
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
        if let Some(category) = query.category {
            sql.push(" AND i.category = ").push_bind(category.as_str());
        }
        if let Some(release) = &query.first_release {
            sql.push(" AND i.first_release = ").push_bind(release.clone());
        }
        if let Some(text) = query.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            for word in text.split_whitespace() {
                let pattern = format!("%{}%", word.replace('%', "\\%").replace('_', "\\_"));
                sql.push(" AND (i.title LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\'");
                sql.push(" OR i.culprit LIKE ").push_bind(pattern).push(" ESCAPE '\\')");
            }
        }
        for tag in &query.tags {
            sql.push(if tag.negated { " AND NOT EXISTS" } else { " AND EXISTS" });
            sql.push(" (SELECT 1 FROM issue_tags t WHERE t.issue_id = i.id AND t.key = ")
                .push_bind(tag.key.clone());
            if tag.prefix {
                sql.push(" AND substr(t.value, 1, ").push_bind(tag.value.chars().count() as i64).push(") = ");
            } else {
                sql.push(" AND t.value = ");
            }
            sql.push_bind(tag.value.clone()).push(")");
        }
        sql.push(match query.sort {
            IssueSort::LastSeen => " ORDER BY i.last_seen DESC",
            IssueSort::FirstSeen => " ORDER BY i.first_seen DESC",
            IssueSort::Count => " ORDER BY i.count DESC",
            IssueSort::Users => " ORDER BY users DESC, i.last_seen DESC",
        });
        sql.push(" LIMIT ")
            .push_bind(i64::from(query.limit))
            .push(" OFFSET ")
            .push_bind(i64::from(query.offset));
        let rows = sql.build().fetch_all(self.db.pool()).await.map_err(port)?;
        Ok(rows
            .iter()
            .map(|row| IssueSummary {
                issue: issue(row),
                last_day: row.get::<i64, _>("last_day") as u64,
                users: row.get::<i64, _>("users") as u64,
            })
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

    async fn record(
        &self,
        id: IssueId,
        at: Timestamp,
        release: Option<&str>,
        tags: &[(String, String)],
    ) -> Result<(), PortError> {
        let mut tx = self.db.write().await.map_err(port)?;
        add_tags(&mut tx, id.get(), at, tags).await.map_err(port)?;
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

    async fn activity(&self, query: &ActivityQuery) -> Result<ErrorActivity, PortError> {
        let in_scope = |sql: &mut QueryBuilder<Sqlite>| {
            sql.push(" AND i.category = ").push_bind(IssueCategory::Error.as_str());
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
        };
        let hours = |sql: &mut QueryBuilder<Sqlite>| {
            sql.push(" FROM issue_hours h JOIN issues i ON i.id = h.issue_id WHERE h.hour >= ")
                .push_bind(query.from.hour())
                .push(" AND h.hour < ")
                .push_bind(query.to.unix_seconds());
        };
        let step = query.bucket_seconds.max(3600);
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT (h.hour / ");
        sql.push_bind(step).push(") * ").push_bind(step).push(" AS bucket, sum(h.count) AS events");
        hours(&mut sql);
        in_scope(&mut sql);
        sql.push(" GROUP BY bucket ORDER BY bucket");
        let rows = sql.build().fetch_all(self.db.pool()).await.map_err(port)?;
        let buckets =
            rows.iter().map(|row| (row.get("bucket"), row.get::<i64, _>("events") as u64)).collect();

        let counted = |column: &str| {
            let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT count(*) FROM issues i WHERE i.");
            sql.push(column).push(" >= ").push_bind(query.from.unix_micros());
            sql.push(" AND i.").push(column).push(" < ").push_bind(query.to.unix_micros());
            in_scope(&mut sql);
            sql
        };
        let new_issues: i64 =
            counted("first_seen").build_query_scalar().fetch_one(self.db.pool()).await.map_err(port)?;
        let regressed: i64 =
            counted("regressed_at").build_query_scalar().fetch_one(self.db.pool()).await.map_err(port)?;

        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT ");
        for column in COLUMNS.split(", ") {
            sql.push("i.").push(column.trim()).push(", ");
        }
        sql.push("sum(h.count) AS events, coalesce((SELECT k.distinct_values FROM issue_tag_keys k WHERE k.issue_id = i.id AND k.key = ")
            .push_bind(USER)
            .push("), 0) AS users");
        hours(&mut sql);
        in_scope(&mut sql);
        sql.push(" AND i.status != 'muted' GROUP BY i.id ORDER BY events DESC, i.last_seen DESC LIMIT ")
            .push_bind(i64::from(query.top));
        let rows = sql.build().fetch_all(self.db.pool()).await.map_err(port)?;
        let top = rows
            .iter()
            .map(|row| ActiveIssue {
                issue: issue(row),
                events: row.get::<i64, _>("events") as u64,
                users: row.get::<i64, _>("users") as u64,
            })
            .collect();
        Ok(ErrorActivity { buckets, new_issues: new_issues as u64, regressed: regressed as u64, top })
    }

    async fn tag_keys(&self, id: IssueId) -> Result<Vec<TagKey>, PortError> {
        let rows = sqlx::query("SELECT key, events, distinct_values FROM issue_tag_keys WHERE issue_id = ?")
            .bind(id.get())
            .fetch_all(self.db.pool())
            .await
            .map_err(port)?;
        Ok(rows
            .iter()
            .map(|row| TagKey {
                key: row.get("key"),
                events: row.get::<i64, _>("events") as u64,
                values: row.get::<i64, _>("distinct_values") as u32,
            })
            .collect())
    }

    async fn tag_values(&self, id: IssueId, key: &str, limit: u32) -> Result<Vec<TagCount>, PortError> {
        let rows = sqlx::query(
            "SELECT value, count FROM issue_tags WHERE issue_id = ? AND key = ? ORDER BY count DESC, value LIMIT ?",
        )
        .bind(id.get())
        .bind(key)
        .bind(i64::from(limit))
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        Ok(rows
            .iter()
            .map(|row| TagCount { value: row.get("value"), count: row.get::<i64, _>("count") as u64 })
            .collect())
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

    async fn errors_by_release(
        &self,
        projects: &[ProjectId],
        environments: &[EnvironmentId],
        limit: u32,
    ) -> Result<Vec<ReleaseErrors>, PortError> {
        // The scopes' filter, for the issues joined and those counted as new.
        fn scope(
            sql: &mut QueryBuilder<Sqlite>,
            alias: &str,
            projects: &[ProjectId],
            environments: &[EnvironmentId],
        ) {
            sql.push(format!(" AND {alias}.category = ")).push_bind(IssueCategory::Error.as_str());
            if !projects.is_empty() {
                sql.push(format!(" AND {alias}.project_id IN ("));
                let mut list = sql.separated(", ");
                for project in projects {
                    list.push_bind(project.get());
                }
                list.push_unseparated(")");
            }
            if !environments.is_empty() {
                sql.push(format!(" AND {alias}.environment_id IN ("));
                let mut list = sql.separated(", ");
                for environment in environments {
                    list.push_bind(environment.get());
                }
                list.push_unseparated(")");
            }
        }
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT r.release, sum(r.count) AS events, count(*) AS issues, min(r.first_seen) AS first_seen, \
             max(r.last_seen) AS last_seen, (SELECT count(*) FROM issues n WHERE n.first_release = r.release",
        );
        scope(&mut sql, "n", projects, environments);
        sql.push(") AS new_issues FROM issue_releases r JOIN issues i ON i.id = r.issue_id WHERE 1 = 1");
        scope(&mut sql, "i", projects, environments);
        sql.push(" GROUP BY r.release ORDER BY last_seen DESC LIMIT ").push_bind(i64::from(limit));
        let rows = sql.build().fetch_all(self.db.pool()).await.map_err(port)?;
        Ok(rows
            .iter()
            .map(|r| ReleaseErrors {
                release: r.get("release"),
                events: r.get::<i64, _>("events").max(0) as u64,
                issues: r.get::<i64, _>("issues").max(0) as u64,
                new_issues: r.get::<i64, _>("new_issues").max(0) as u64,
                first_seen: Timestamp::from_unix_micros(r.get("first_seen")),
                last_seen: Timestamp::from_unix_micros(r.get("last_seen")),
            })
            .collect())
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

/// Counts one event's tags for its issue. A value not kept yet is kept
/// while the issue has fewer than [`values_kept`] of the tag.
async fn add_tags(
    tx: &mut SqliteConnection,
    issue: i64,
    at: Timestamp,
    tags: &[(String, String)],
) -> Result<(), sqlx::Error> {
    for (key, value) in tags {
        let kept: i64 = sqlx::query_scalar(
            "INSERT INTO issue_tag_keys (issue_id, key, events, distinct_values) VALUES (?, ?, 1, 0) \
             ON CONFLICT (issue_id, key) DO UPDATE SET events = events + 1 RETURNING distinct_values",
        )
        .bind(issue)
        .bind(key)
        .fetch_one(&mut *tx)
        .await?;
        let counted = sqlx::query(
            "UPDATE issue_tags SET count = count + 1, first_seen = min(first_seen, ?1), \
             last_seen = max(last_seen, ?1) WHERE issue_id = ?2 AND key = ?3 AND value = ?4",
        )
        .bind(at.unix_micros())
        .bind(issue)
        .bind(key)
        .bind(value)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if counted == 0 && kept < i64::from(values_kept(key)) {
            sqlx::query(
                "INSERT INTO issue_tags (issue_id, key, value, count, first_seen, last_seen) VALUES (?1, ?2, ?3, 1, ?4, ?4)",
            )
            .bind(issue)
            .bind(key)
            .bind(value)
            .bind(at.unix_micros())
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE issue_tag_keys SET distinct_values = distinct_values + 1 WHERE issue_id = ? AND key = ?")
                .bind(issue)
                .bind(key)
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(())
}

/// Where [`backfill_tags`] got to, in the setting migration 0009 made.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Backfill {
    /// Events received before this (Unix microseconds) were not counted.
    since: i64,
    /// The segment's day and the last event's row in it.
    day: i64,
    id: i64,
}

const BACKFILL: &str = "issue_tags_backfill";
const BACKFILL_BATCH: i64 = 500;

/// Counts the tags of the events received before tags were counted, once:
/// segment by segment, oldest first, in batches whose place is saved with
/// them, so a restart goes on where it stopped. Returns the events counted.
pub async fn backfill_tags(db: &Database, segments: &Segments) -> Result<u64, PortError> {
    let state: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(BACKFILL)
        .fetch_optional(db.pool())
        .await
        .map_err(port)?;
    let Some(mut state) = state.and_then(|json| serde_json::from_str::<Backfill>(&json).ok()) else {
        return Ok(0);
    };
    let mut counted = 0;
    // Events from before were kept in the kind's own segments: stages had
    // no retention of their own yet.
    let mut days: Vec<Day> = segments
        .all()
        .await?
        .into_iter()
        .filter(|s| s.kind == Kind::Events && s.keep == 0)
        .map(|s| s.day)
        .collect();
    days.retain(|day| day.days() >= state.day && *day <= Timestamp::from_unix_micros(state.since).day());
    days.sort();
    for day in days {
        if day.days() > state.day {
            state.day = day.days();
            state.id = 0;
        }
        let Some(pool) = segments.reader(SegmentId::new(Kind::Events, day, 0)).await? else { continue };
        loop {
            let rows = sqlx::query(
                "SELECT id, issue_id, time, data FROM events WHERE id > ? AND received < ? ORDER BY id LIMIT ?",
            )
            .bind(state.id)
            .bind(state.since)
            .bind(BACKFILL_BATCH)
            .fetch_all(&pool)
            .await
            .map_err(port)?;
            let Some(last) = rows.last() else { break };
            state.id = last.get("id");
            let mut tx = db.write().await.map_err(port)?;
            for row in &rows {
                let issue: i64 = row.get("issue_id");
                let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM issues WHERE id = ?")
                    .bind(issue)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(port)?;
                let data: Vec<u8> = row.get("data");
                let (Some(_), Some(payload)) = (exists, unpacked(&data)) else { continue };
                let tags = tags_of(&ErrorEvent::from_payload(payload, None));
                add_tags(&mut tx, issue, Timestamp::from_unix_micros(row.get("time")), &tags)
                    .await
                    .map_err(port)?;
                counted += 1;
            }
            save_backfill(&mut tx, &state).await?;
            tx.commit().await.map_err(port)?;
            if (rows.len() as i64) < BACKFILL_BATCH {
                break;
            }
        }
    }
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(BACKFILL)
        .execute(db.pool())
        .await
        .map_err(port)?;
    Ok(counted)
}

async fn save_backfill(tx: &mut SqliteConnection, state: &Backfill) -> Result<(), PortError> {
    sqlx::query("UPDATE settings SET value = ? WHERE key = ?")
        .bind(serde_json::to_string(state).map_err(port)?)
        .bind(BACKFILL)
        .execute(&mut *tx)
        .await
        .map_err(port)?;
    Ok(())
}

/// [`EventStore`] over the events' day segments.
#[derive(Debug)]
pub struct SegmentEvents {
    segments: Arc<Segments>,
    /// Today's segments and the connections to them.
    writers: Mutex<Writers>,
}

impl SegmentEvents {
    pub fn new(segments: Arc<Segments>) -> Self {
        Self { segments, writers: Mutex::new(Writers::new(Kind::Events, false)) }
    }

    pub async fn close(&self) {
        self.writers.lock().await.close().await;
    }

    /// Every segment with events, newest first.
    async fn segments(&self) -> Result<Vec<SegmentId>, PortError> {
        Ok(self.segments.overlapping(Kind::Events, None, None).await?.iter().map(|s| s.id()).collect())
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
        let keep = self.segments.keep(Kind::Events, scope.environment).await?;
        let segment = SegmentId::new(Kind::Events, received.day(), keep);
        let mut writers = self.writers.lock().await;
        let connection = writers.get(&self.segments, segment).await?;
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
        drop(writers);
        self.segments.note_batch(segment, 1, time, time).await
    }

    async fn get(&self, event_id: &str) -> Result<Option<StoredEvent>, PortError> {
        for segment in self.segments().await? {
            let Some(pool) = self.segments.reader(segment).await? else { continue };
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
        let days = [since.day(), Day::from_days(since.day().days() + 1)];
        for segment in self.segments.on_days(Kind::Events, &days).await? {
            let Some(pool) = self.segments.reader(segment).await? else { continue };
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
        for segment in self.segments().await? {
            let Some(pool) = self.segments.reader(segment).await? else { continue };
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
        for segment in self.segments().await? {
            let Some(pool) = self.segments.reader(segment).await? else { continue };
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
    use logdock_issues::domain::{MAX_TAG_VALUES, TagFilter};
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
            category: IssueCategory::Error,
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

        let tags = |user: &str| {
            vec![("user".to_owned(), user.to_owned()), ("url".to_owned(), "https://shop/a b".to_owned())]
        };
        issues.record(issue.id, at, Some("r1"), &tags("id:1")).await.unwrap();
        issues.record(issue.id, at, Some("r2"), &tags("id:2")).await.unwrap();
        let later = at.saturating_add(std::time::Duration::from_secs(3600));
        issues.record(issue.id, later, None, &tags("id:1")).await.unwrap();
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

        // Tags: the users affected, the values, searching by them.
        let keys = issues.tag_keys(issue.id).await.unwrap();
        let user = keys.iter().find(|k| k.key == "user").unwrap();
        assert_eq!((user.events, user.values), (3, 2));
        let values = issues.tag_values(issue.id, "user", 10).await.unwrap();
        assert_eq!(
            values,
            [TagCount { value: "id:1".into(), count: 2 }, TagCount { value: "id:2".into(), count: 1 }]
        );
        assert_eq!(issues.tag_values(issue.id, "user", 1).await.unwrap().len(), 1);
        let by = |tags: Vec<TagFilter>| IssueQuery { limit: 10, tags, ..Default::default() };
        let tag = |key: &str, value: &str, prefix: bool, negated: bool| TagFilter {
            key: key.into(),
            value: value.into(),
            prefix,
            negated,
        };
        let found = issues.list(&by(vec![tag("user", "id:2", false, false)]), at).await.unwrap();
        assert_eq!((found.len(), found[0].users), (1, 2));
        assert_eq!(
            issues.list(&by(vec![tag("url", "https://shop/a", true, false)]), at).await.unwrap().len(),
            1
        );
        assert!(
            issues.list(&by(vec![tag("url", "https://shop/b", true, false)]), at).await.unwrap().is_empty()
        );
        assert!(issues.list(&by(vec![tag("user", "id:1", false, true)]), at).await.unwrap().is_empty());
        assert_eq!(issues.list(&by(vec![tag("user", "id:9", false, true)]), at).await.unwrap().len(), 1);
        let sorted = IssueQuery { limit: 10, sort: IssueSort::Users, ..Default::default() };
        assert_eq!(issues.list(&sorted, at).await.unwrap()[0].users, 2);

        // A performance issue: listed when asked for, left out of the errors.
        let n_plus_one = NewIssue {
            scope,
            fingerprint: "f2".into(),
            category: IssueCategory::Performance,
            title: "N+1 query: SELECT * FROM users WHERE id = ?".into(),
            culprit: Some("POST /orders".into()),
            level: EventLevel::Warning,
            seen: at,
            release: Some("r1".into()),
            event_id: "p1".into(),
        };
        let slow = issues.create(n_plus_one, &payload("r1")).await.unwrap().unwrap();
        assert_eq!(issues.get(slow.id).await.unwrap().unwrap().category, IssueCategory::Performance);
        issues.record(slow.id, at, Some("r1"), &[]).await.unwrap();
        let of = |category: Option<IssueCategory>| IssueQuery { limit: 10, category, ..Default::default() };
        let ids = |list: Vec<IssueSummary>| list.iter().map(|s| s.issue.id).collect::<Vec<_>>();
        assert_eq!(ids(issues.list(&of(Some(IssueCategory::Error)), at).await.unwrap()), [issue.id]);
        assert_eq!(ids(issues.list(&of(Some(IssueCategory::Performance)), at).await.unwrap()), [slow.id]);
        assert_eq!(issues.list(&of(None), at).await.unwrap().len(), 2);
        let releases = issues.errors_by_release(&[], &[], 10).await.unwrap();
        let r1 = releases.iter().find(|r| r.release == "r1").unwrap();
        assert_eq!((r1.events, r1.issues, r1.new_issues), (1, 1, 1), "errors only");

        // What the issues did in a range: events per bucket, the busiest.
        let activity = ActivityQuery {
            projects: vec![scope.project],
            environments: vec![scope.environment],
            from: Timestamp::from_unix_seconds(at.hour()),
            to: at.saturating_add(std::time::Duration::from_secs(7200)),
            bucket_seconds: 3600,
            top: 5,
        };
        let found = issues.activity(&activity).await.unwrap();
        assert_eq!(found.buckets, [(at.hour(), 2), (at.hour() + 3600, 1)]);
        assert_eq!((found.new_issues, found.regressed), (1, 0));
        assert_eq!((found.top.len(), found.top[0].events, found.top[0].users), (1, 3, 2));
        let daily =
            issues.activity(&ActivityQuery { bucket_seconds: 86_400, ..activity.clone() }).await.unwrap();
        assert_eq!(daily.buckets.iter().map(|(_, n)| n).sum::<u64>(), 3);
        let elsewhere = ActivityQuery { projects: vec![ProjectId::new(9)], ..activity };
        assert_eq!(issues.activity(&elsewhere).await.unwrap(), ErrorActivity::default());

        // Values beyond the most kept are counted for the tag only.
        let many: Vec<(String, String)> =
            (0..=MAX_TAG_VALUES).map(|n| ("route".to_owned(), format!("/r/{n}"))).collect();
        let mut tx = db.write().await.unwrap();
        add_tags(&mut tx, issue.id.get(), at, &many).await.unwrap();
        tx.commit().await.unwrap();
        let route = issues.tag_keys(issue.id).await.unwrap().into_iter().find(|k| k.key == "route").unwrap();
        assert_eq!((route.events, route.values), (u64::from(MAX_TAG_VALUES) + 1, MAX_TAG_VALUES));

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

        // The events from before tags were counted are counted once.
        let before = issues.tag_keys(issue.id).await.unwrap();
        let transactions =
            |keys: &[TagKey]| keys.iter().find(|k| k.key == "transaction").map_or(0, |k| k.events);
        assert_eq!(backfill_tags(&db, &segments).await.unwrap(), 2);
        let after = issues.tag_keys(issue.id).await.unwrap();
        assert_eq!(transactions(&after), transactions(&before) + 2);
        assert_eq!(backfill_tags(&db, &segments).await.unwrap(), 0, "once");

        issues.delete(issue.id).await.unwrap();
        assert!(issues.get(issue.id).await.unwrap().is_none());
    }
}
