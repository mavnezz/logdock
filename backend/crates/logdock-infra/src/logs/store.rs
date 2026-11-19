//! Finding entries again: the search over the day segments a query can
//! touch, newest first, and the counts kept in the main database.

use std::sync::Arc;

use async_trait::async_trait;
use logdock_kernel::{EnvironmentId, PortError, ProjectId, Timestamp};
use logdock_logs::application::ports::{LogStats, LogStore};
use logdock_logs::domain::{
    Bucket, Cursor, HistogramQuery, Level, LogEntry, LogId, LogPage, LogQuery, Source, Term,
};
use logdock_retention::domain::Kind;
use sqlx::{QueryBuilder, Row, Sqlite};

use super::entry;
use crate::segments::{Segments, port};

/// [`LogStore`] over the logs' day segments.
#[derive(Debug, Clone)]
pub struct SegmentLogStore {
    segments: Arc<Segments>,
}

impl SegmentLogStore {
    pub fn new(segments: Arc<Segments>) -> Self {
        Self { segments }
    }
}

/// The full-text part of a query as FTS5 understands it: every word and
/// phrase quoted, so nothing typed is taken for FTS5's own syntax.
pub(crate) fn match_expression(query: &LogQuery) -> Option<String> {
    let quote = |text: &str| format!("\"{}\"", text.replace('"', "\"\""));
    let mut parts: Vec<String> = query
        .text
        .text_terms()
        .map(|term| match term {
            Term::Word { text, prefix: true } => format!("{}*", quote(text)),
            Term::Word { text, .. } | Term::Phrase(text) => quote(text),
            Term::Attribute { .. } => unreachable!("text_terms() has no attributes"),
        })
        .collect();
    // An attribute's value must appear in the indexed attributes: the index
    // narrows the rows, json_extract then checks the key.
    for (_, value) in query.text.attributes() {
        if value.chars().any(char::is_alphanumeric) {
            parts.push(format!("attributes : {}", quote(value)));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn push_in<T>(sql: &mut QueryBuilder<Sqlite>, column: &str, values: &[T])
where
    T: for<'t> sqlx::Encode<'t, Sqlite> + sqlx::Type<Sqlite> + Clone,
{
    if values.is_empty() {
        return;
    }
    sql.push(format_args!(" AND {column} IN ("));
    let mut list = sql.separated(", ");
    for value in values {
        list.push_bind(value.clone());
    }
    list.push_unseparated(")");
}

/// A query's filters, prepared once for every segment it reads.
struct Prepared<'a> {
    query: &'a LogQuery,
    fts: Option<String>,
    projects: Vec<i64>,
    environments: Vec<i64>,
    levels: Vec<i64>,
}

impl SegmentLogStore {
    async fn search_segment(
        &self,
        day: logdock_kernel::Day,
        prepared: &Prepared<'_>,
        limit: u32,
    ) -> Result<Vec<LogEntry>, PortError> {
        let Prepared { query, fts, projects, environments, levels } = prepared;
        let Some(pool) = self.segments.reader(Kind::Logs, day).await? else { return Ok(Vec::new()) };
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT l.* FROM logs l");
        if let Some(fts) = fts {
            sql.push(" JOIN logs_fts ON logs_fts.rowid = l.id AND logs_fts MATCH ").push_bind(fts.clone());
        }
        sql.push(" WHERE 1 = 1");
        if let Some(from) = query.from {
            sql.push(" AND l.time >= ").push_bind(from.unix_micros());
        }
        if let Some(to) = query.to {
            sql.push(" AND l.time < ").push_bind(to.unix_micros());
        }
        if let Some(cursor) = query.before {
            let time = cursor.time.unix_micros();
            if day == cursor.id.day {
                sql.push(" AND (l.time < ").push_bind(time);
                sql.push(" OR (l.time = ")
                    .push_bind(time)
                    .push(" AND l.id < ")
                    .push_bind(cursor.id.row)
                    .push("))");
            } else if day < cursor.id.day {
                sql.push(" AND l.time <= ").push_bind(time);
            } else {
                sql.push(" AND l.time < ").push_bind(time);
            }
        }
        push_in(&mut sql, "l.project_id", projects);
        push_in(&mut sql, "l.environment_id", environments);
        push_in(&mut sql, "l.level", levels);
        push_in(&mut sql, "l.service", &query.services);
        push_in(&mut sql, "l.host", &query.hosts);
        if let Some(trace) = &query.trace_id {
            sql.push(" AND l.trace_id = ").push_bind(trace.clone());
        }
        for (key, value) in query.text.attributes() {
            // Keys are letters, digits, `_`, `.` and `-` (SearchText).
            sql.push(format_args!(" AND CAST(json_extract(l.attributes, '$.\"{key}\"') AS TEXT) = "));
            sql.push_bind(value.to_owned());
        }
        sql.push(" ORDER BY l.time DESC, l.id DESC LIMIT ").push_bind(i64::from(limit));
        let rows = sql.build().fetch_all(&pool).await.map_err(port)?;
        Ok(rows.iter().map(|row| entry(day, row)).collect())
    }
}

#[async_trait]
impl LogStore for SegmentLogStore {
    async fn search(&self, query: &LogQuery) -> Result<LogPage, PortError> {
        let limit = query.limit.max(1);
        let prepared = Prepared {
            query,
            fts: match_expression(query),
            projects: query.projects.iter().map(|p| p.get()).collect(),
            environments: query.environments.iter().map(|e| e.get()).collect(),
            levels: query.levels.iter().map(|l| l.number()).collect(),
        };
        let to = match (query.to, query.before) {
            (Some(to), Some(cursor)) => {
                Some(to.min(cursor.time.saturating_add(std::time::Duration::from_micros(1))))
            }
            (to, cursor) => to.or(cursor.map(|c| c.time.saturating_add(std::time::Duration::from_micros(1)))),
        };
        let segments = self.segments.overlapping(Kind::Logs, query.from, to).await?;
        let mut found: Vec<LogEntry> = Vec::new();
        for segment in segments {
            // Newest events first: once the page is full, a segment whose
            // newest event is older than the page's oldest adds nothing.
            if found.len() as u32 > limit
                && let (Some(max), Some(oldest)) =
                    (segment.max_time, found.get(limit as usize).map(|e| e.time))
                && max < oldest
            {
                break;
            }
            let rows = self.search_segment(segment.day, &prepared, limit + 1).await?;
            found.extend(rows);
            found.sort_by_key(|e| std::cmp::Reverse(order_key(e)));
            found.truncate(limit as usize + 1);
        }
        let more = found.len() > limit as usize;
        found.truncate(limit as usize);
        let next =
            if more { found.last().and_then(|e| e.id.map(|id| Cursor { time: e.time, id })) } else { None };
        Ok(LogPage { entries: found, next })
    }

    async fn get(&self, id: LogId) -> Result<Option<LogEntry>, PortError> {
        let Some(pool) = self.segments.reader(Kind::Logs, id.day).await? else { return Ok(None) };
        let row = sqlx::query("SELECT * FROM logs WHERE id = ?")
            .bind(id.row)
            .fetch_optional(&pool)
            .await
            .map_err(port)?;
        Ok(row.map(|row| entry(id.day, &row)))
    }
}

fn order_key(entry: &LogEntry) -> (Timestamp, i64, i64) {
    let id = entry.id.unwrap_or(LogId { day: logdock_kernel::Day::from_days(0), row: 0 });
    (entry.time, id.day.days(), id.row)
}

/// [`LogStats`] on the main database's rollups.
#[derive(Debug, Clone)]
pub struct SqliteLogStats {
    segments: Arc<Segments>,
}

impl SqliteLogStats {
    pub fn new(segments: Arc<Segments>) -> Self {
        Self { segments }
    }
}

#[async_trait]
impl LogStats for SqliteLogStats {
    async fn histogram(&self, query: &HistogramQuery) -> Result<Vec<Bucket>, PortError> {
        let size = query.bucket_seconds.max(60);
        let projects: Vec<i64> = query.projects.iter().map(|p| p.get()).collect();
        let environments: Vec<i64> = query.environments.iter().map(|e| e.get()).collect();
        let levels: Vec<i64> = query.levels.iter().map(|l| l.number()).collect();
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT (minute / ");
        sql.push_bind(size)
            .push(") * ")
            .push_bind(size)
            .push(" AS bucket, level, sum(count) AS n FROM log_rollups WHERE minute >= ");
        sql.push_bind(query.from.unix_seconds()).push(" AND minute < ").push_bind(query.to.unix_seconds());
        push_in(&mut sql, "project_id", &projects);
        push_in(&mut sql, "environment_id", &environments);
        push_in(&mut sql, "level", &levels);
        push_in(&mut sql, "service", &query.services);
        sql.push(" GROUP BY bucket, level ORDER BY bucket");
        let rows = sql.build().fetch_all(self.segments.db().pool()).await.map_err(port)?;
        let mut buckets: Vec<Bucket> = Vec::new();
        for row in rows {
            let start: i64 = row.get("bucket");
            let level = Level::from_number(row.get("level")).unwrap_or(Level::Info);
            let count: i64 = row.get("n");
            if buckets.last().is_none_or(|b| b.start != start) {
                buckets.push(Bucket { start, counts: [0; 6] });
            }
            if let Some(bucket) = buckets.last_mut() {
                bucket.counts[(level.number() - 1) as usize] += count as u64;
            }
        }
        Ok(buckets)
    }

    async fn sources(&self, projects: &[ProjectId]) -> Result<Vec<Source>, PortError> {
        let ids: Vec<i64> = projects.iter().map(|p| p.get()).collect();
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT project_id, environment_id, service, host, last_seen FROM log_sources WHERE 1 = 1",
        );
        push_in(&mut sql, "project_id", &ids);
        sql.push(" ORDER BY service, host");
        let rows = sql.build().fetch_all(self.segments.db().pool()).await.map_err(port)?;
        let text = |value: String| (!value.is_empty()).then_some(value);
        Ok(rows
            .iter()
            .map(|row| Source {
                project: ProjectId::new(row.get("project_id")),
                environment: EnvironmentId::new(row.get("environment_id")),
                service: text(row.get("service")),
                host: text(row.get("host")),
                last_seen: Timestamp::from_unix_micros(row.get("last_seen")),
            })
            .collect())
    }
}
