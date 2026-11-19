//! The Performance context's adapters: rollups in the main database,
//! transactions with their spans in the day segments of kind `traces`.
//!
//! Rollups are added up in memory and written by
//! [`SqliteRollups::flush`], every few seconds and before every read, so a
//! transaction costs one insert into its segment and no write to the main
//! database.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use logdock_kernel::{EnvironmentId, PortError, ProjectId, Scope, Timestamp, Value};
use logdock_performance::application::ports::{
    RollupQuery, RollupStore, SampleQuery, StoredTransaction, TransactionStore, TransactionSummary,
};
use logdock_performance::domain::{BUCKETS, Kind, Measurement, Rollup, Span, Transaction};
use logdock_retention::domain::Kind as SegmentKind;
use serde_json::{Value as Json, json};
use sqlx::sqlite::SqliteRow;
use sqlx::{Connection, QueryBuilder, Row, Sqlite, SqliteConnection};
use tokio::sync::Mutex;

use crate::json::{from_json, packed_json, to_json, unpacked_json};
use crate::segments::Segments;
use crate::sqlite::Database;

fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("performance", error)
}

type Key = (Kind, i64, i64, i64, String);

/// [`RollupStore`] on `perf_rollups`.
#[derive(Debug)]
pub struct SqliteRollups {
    db: Database,
    pending: std::sync::Mutex<HashMap<Key, Rollup>>,
}

impl SqliteRollups {
    pub fn new(db: Database) -> Self {
        Self { db, pending: std::sync::Mutex::new(HashMap::new()) }
    }

    /// Writes what was added since the last flush; what it could not write
    /// is kept for the next one.
    pub async fn flush(&self) -> Result<(), PortError> {
        let pending = std::mem::take(&mut *self.pending.lock().unwrap_or_else(|e| e.into_inner()));
        if pending.is_empty() {
            return Ok(());
        }
        let result = self.write(&pending).await;
        if result.is_err() {
            let mut kept = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            for (key, rollup) in pending {
                kept.entry(key).or_default().merge(&rollup);
            }
        }
        result
    }

    async fn write(&self, pending: &HashMap<Key, Rollup>) -> Result<(), PortError> {
        let mut tx = self.db.write().await.map_err(port)?;
        for ((kind, minute, project, environment, name), r) in pending {
            let (kind, minute, project, environment) = (*kind, *minute, *project, *environment);
            let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new(concat!(
                "INSERT INTO perf_rollups (kind, minute, project_id, environment_id, name, count, flagged, ",
                "total_us, max_us, b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11, b12, b13, b14) VALUES ("
            ));
            let mut values = sql.separated(", ");
            values.push_bind(kind.as_str());
            values.push_bind(minute);
            values.push_bind(project);
            values.push_bind(environment);
            values.push_bind(name.clone());
            values.push_bind(r.count as i64);
            values.push_bind(r.flagged as i64);
            values.push_bind(r.total_us.min(i64::MAX as u64) as i64);
            values.push_bind(r.max_us as i64);
            for count in r.buckets {
                values.push_bind(count as i64);
            }
            sql.push(concat!(
                ") ON CONFLICT (kind, minute, project_id, environment_id, name) DO UPDATE SET ",
                "count = count + excluded.count, flagged = flagged + excluded.flagged, ",
                "total_us = total_us + excluded.total_us, max_us = max(max_us, excluded.max_us), ",
                "b0 = b0 + excluded.b0, b1 = b1 + excluded.b1, b2 = b2 + excluded.b2, b3 = b3 + excluded.b3, ",
                "b4 = b4 + excluded.b4, b5 = b5 + excluded.b5, b6 = b6 + excluded.b6, b7 = b7 + excluded.b7, ",
                "b8 = b8 + excluded.b8, b9 = b9 + excluded.b9, b10 = b10 + excluded.b10, ",
                "b11 = b11 + excluded.b11, b12 = b12 + excluded.b12, b13 = b13 + excluded.b13, ",
                "b14 = b14 + excluded.b14"
            ));
            sql.build().execute(&mut *tx).await.map_err(port)?;
        }
        tx.commit().await.map_err(port)
    }

    fn filtered(sql: &mut QueryBuilder<Sqlite>, query: &RollupQuery) {
        sql.push(" WHERE kind = ").push_bind(query.kind.as_str());
        sql.push(" AND minute >= ").push_bind(query.from.unix_seconds().div_euclid(60) * 60);
        sql.push(" AND minute < ").push_bind(query.to.unix_seconds());
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
    }
}

fn rollup(row: &SqliteRow) -> Rollup {
    let mut buckets = [0u64; BUCKETS];
    for (i, bucket) in buckets.iter_mut().enumerate() {
        *bucket = row.get::<i64, _>(format!("b{i}").as_str()) as u64;
    }
    Rollup {
        count: row.get::<i64, _>("count") as u64,
        flagged: row.get::<i64, _>("flagged") as u64,
        total_us: row.get::<i64, _>("total_us") as u64,
        max_us: row.get::<i64, _>("max_us") as u64,
        buckets,
    }
}

fn sums() -> String {
    let buckets: Vec<String> = (0..BUCKETS).map(|i| format!("sum(b{i}) AS b{i}")).collect();
    format!(
        "sum(count) AS count, sum(flagged) AS flagged, sum(total_us) AS total_us, max(max_us) AS max_us, {}",
        buckets.join(", ")
    )
}

#[async_trait]
impl RollupStore for SqliteRollups {
    async fn add(
        &self,
        scope: Scope,
        minute: i64,
        measurements: &[Measurement],
        weight: u64,
    ) -> Result<(), PortError> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        for m in measurements {
            let key = (m.kind, minute, scope.project.get(), scope.environment.get(), m.name.clone());
            pending.entry(key).or_default().add(m.duration_us, m.flagged, weight);
        }
        Ok(())
    }

    async fn names(&self, query: &RollupQuery) -> Result<Vec<(String, Rollup)>, PortError> {
        self.flush().await?;
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT name, ");
        sql.push(sums()).push(" FROM perf_rollups");
        Self::filtered(&mut sql, query);
        sql.push(" GROUP BY name");
        let rows = sql.build().fetch_all(self.db.pool()).await.map_err(port)?;
        Ok(rows.iter().map(|row| (row.get("name"), rollup(row))).collect())
    }

    async fn series(
        &self,
        query: &RollupQuery,
        name: &str,
        bucket_seconds: i64,
    ) -> Result<Vec<(i64, Rollup)>, PortError> {
        self.flush().await?;
        let bucket_seconds = bucket_seconds.max(60);
        let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT (minute / ");
        sql.push_bind(bucket_seconds).push(") * ").push_bind(bucket_seconds).push(" AS bucket, ");
        sql.push(sums()).push(" FROM perf_rollups");
        Self::filtered(&mut sql, query);
        sql.push(" AND name = ").push_bind(name.to_owned());
        sql.push(" GROUP BY bucket ORDER BY bucket");
        let rows = sql.build().fetch_all(self.db.pool()).await.map_err(port)?;
        Ok(rows.iter().map(|row| (row.get("bucket"), rollup(row))).collect())
    }
}

/// [`TransactionStore`] over the trace segments.
#[derive(Debug)]
pub struct SegmentTransactions {
    segments: Arc<Segments>,
    writer: Mutex<Option<(logdock_kernel::Day, SqliteConnection)>>,
}

impl SegmentTransactions {
    pub fn new(segments: Arc<Segments>) -> Self {
        Self { segments, writer: Mutex::new(None) }
    }

    pub async fn close(&self) {
        if let Some((_, connection)) = self.writer.lock().await.take() {
            let _ = connection.close().await;
        }
    }
}

fn span_json(span: &Span) -> Json {
    json!({
        "span_id": span.span_id,
        "parent_span_id": span.parent_span_id,
        "op": span.op,
        "description": span.description,
        "start": span.start.unix_micros(),
        "duration_us": span.duration_us,
        "status": span.status,
        "data": to_json(&span.data),
    })
}

fn transaction_json(t: &Transaction) -> Json {
    json!({
        "trace_id": t.trace_id,
        "span_id": t.span_id,
        "parent_span_id": t.parent_span_id,
        "name": t.name,
        "op": t.op,
        "kind": t.kind.as_str(),
        "status": t.status,
        "http_method": t.http_method,
        "http_status": t.http_status,
        "start": t.start.unix_micros(),
        "duration_us": t.duration_us,
        "release": t.release,
        "environment": t.environment,
        "service": t.service,
        "host": t.host,
        "container": t.container,
        "spans": t.spans.iter().map(span_json).collect::<Vec<_>>(),
    })
}

fn string(json: &Json, key: &str) -> Option<String> {
    json.get(key).and_then(Json::as_str).map(str::to_owned)
}

fn micros(json: &Json, key: &str) -> i64 {
    json.get(key).and_then(Json::as_i64).unwrap_or_default()
}

fn transaction_of(json: &Json) -> Transaction {
    Transaction {
        trace_id: string(json, "trace_id").unwrap_or_default(),
        span_id: string(json, "span_id").unwrap_or_default(),
        parent_span_id: string(json, "parent_span_id"),
        name: string(json, "name").unwrap_or_default(),
        op: string(json, "op").unwrap_or_default(),
        kind: string(json, "kind").as_deref().and_then(Kind::parse).unwrap_or(Kind::Other),
        status: string(json, "status"),
        http_method: string(json, "http_method"),
        http_status: json.get("http_status").and_then(Json::as_i64),
        start: Timestamp::from_unix_micros(micros(json, "start")),
        duration_us: micros(json, "duration_us").max(0) as u64,
        release: string(json, "release"),
        environment: string(json, "environment"),
        service: string(json, "service"),
        host: string(json, "host"),
        container: string(json, "container"),
        spans: json
            .get("spans")
            .and_then(Json::as_array)
            .map(|spans| {
                spans
                    .iter()
                    .map(|s| Span {
                        span_id: string(s, "span_id").unwrap_or_default(),
                        parent_span_id: string(s, "parent_span_id"),
                        op: string(s, "op").unwrap_or_default(),
                        description: string(s, "description"),
                        start: Timestamp::from_unix_micros(micros(s, "start")),
                        duration_us: micros(s, "duration_us").max(0) as u64,
                        status: string(s, "status"),
                        data: s.get("data").cloned().map_or(Value::Null, from_json),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn summary(row: &SqliteRow) -> TransactionSummary {
    TransactionSummary {
        scope: Scope {
            project: ProjectId::new(row.get("project_id")),
            environment: EnvironmentId::new(row.get("environment_id")),
        },
        trace_id: row.get("trace_id"),
        span_id: row.get("span_id"),
        kind: Kind::parse(row.get::<&str, _>("kind")).unwrap_or(Kind::Other),
        name: row.get("name"),
        start: Timestamp::from_unix_micros(row.get("start")),
        duration_us: row.get::<i64, _>("duration_us").max(0) as u64,
        status: row.get("status"),
        http_status: row.get("http_status"),
        failed: row.get::<i64, _>("failed") != 0,
        release: row.get("release"),
        service: row.get("service"),
        host: row.get("host"),
    }
}

const SUMMARY: &str = "SELECT trace_id, span_id, kind, name, status, http_status, failed, start, duration_us, \
    project_id, environment_id, release, service, host FROM transactions";

#[async_trait]
impl TransactionStore for SegmentTransactions {
    async fn insert(&self, scope: Scope, t: &Transaction, received: Timestamp) -> Result<(), PortError> {
        let day = received.day();
        let mut writer = self.writer.lock().await;
        if writer.as_ref().is_none_or(|(current, _)| *current != day) {
            if let Some((_, old)) = writer.take() {
                let _ = old.close().await;
            }
            *writer = Some((day, self.segments.open_writer(SegmentKind::Traces, day).await?));
        }
        let (_, connection) = writer.as_mut().expect("opened above");
        sqlx::query(concat!(
            "INSERT INTO transactions (trace_id, span_id, kind, name, status, http_status, failed, start, ",
            "duration_us, received, project_id, environment_id, release, service, host, span_count, data) ",
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&t.trace_id)
        .bind(&t.span_id)
        .bind(t.kind.as_str())
        .bind(t.display_name())
        .bind(&t.status)
        .bind(t.http_status)
        .bind(i64::from(t.failed()))
        .bind(t.start.unix_micros())
        .bind(t.duration_us as i64)
        .bind(received.unix_micros())
        .bind(scope.project.get())
        .bind(scope.environment.get())
        .bind(&t.release)
        .bind(&t.service)
        .bind(&t.host)
        .bind(t.spans.len() as i64)
        .bind(packed_json(&transaction_json(t)))
        .execute(&mut *connection)
        .await
        .map_err(port)?;
        drop(writer);
        self.segments.note_batch(SegmentKind::Traces, day, 1, t.start, t.start).await
    }

    async fn samples(&self, query: &SampleQuery) -> Result<Vec<TransactionSummary>, PortError> {
        let mut found: Vec<TransactionSummary> = Vec::new();
        for segment in
            self.segments.overlapping(SegmentKind::Traces, Some(query.from), Some(query.to)).await?
        {
            let Some(pool) = self.segments.reader(SegmentKind::Traces, segment.day).await? else { continue };
            let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new(SUMMARY);
            sql.push(" WHERE kind = ").push_bind(query.kind.as_str());
            sql.push(" AND name = ").push_bind(query.name.clone());
            sql.push(" AND start >= ").push_bind(query.from.unix_micros());
            sql.push(" AND start < ").push_bind(query.to.unix_micros());
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
            sql.push(if query.slowest { " ORDER BY duration_us DESC" } else { " ORDER BY start DESC" });
            sql.push(" LIMIT ").push_bind(i64::from(query.limit));
            let rows = sql.build().fetch_all(&pool).await.map_err(port)?;
            found.extend(rows.iter().map(summary));
        }
        if query.slowest {
            found.sort_by_key(|s| std::cmp::Reverse(s.duration_us));
        } else {
            found.sort_by_key(|s| std::cmp::Reverse(s.start));
        }
        found.truncate(query.limit as usize);
        Ok(found)
    }

    async fn by_trace(&self, trace_id: &str) -> Result<Vec<StoredTransaction>, PortError> {
        let mut found = Vec::new();
        for segment in self.segments.overlapping(SegmentKind::Traces, None, None).await? {
            let Some(pool) = self.segments.reader(SegmentKind::Traces, segment.day).await? else { continue };
            let rows =
                sqlx::query("SELECT project_id, environment_id, data FROM transactions WHERE trace_id = ?")
                    .bind(trace_id)
                    .fetch_all(&pool)
                    .await
                    .map_err(port)?;
            for row in rows {
                let data: Vec<u8> = row.get("data");
                let Some(json) = unpacked_json(&data) else { continue };
                found.push(StoredTransaction {
                    scope: Scope {
                        project: ProjectId::new(row.get("project_id")),
                        environment: EnvironmentId::new(row.get("environment_id")),
                    },
                    transaction: transaction_of(&json),
                });
            }
        }
        found.sort_by_key(|s| s.transaction.start);
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::repository_tests::database;

    fn scope() -> Scope {
        Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) }
    }

    fn transaction(name: &str, start: i64, ms: u64, status: i64) -> Transaction {
        Transaction {
            trace_id: format!("trace-{name}-{start}"),
            span_id: "s1".into(),
            parent_span_id: None,
            name: name.into(),
            op: "http.server".into(),
            kind: Kind::Request,
            status: Some("ok".into()),
            http_method: Some("GET".into()),
            http_status: Some(status),
            start: Timestamp::from_unix_seconds(start),
            duration_us: ms * 1000,
            release: Some("1.0".into()),
            environment: Some("production".into()),
            service: Some("web".into()),
            host: Some("h1".into()),
            container: None,
            spans: vec![Span {
                span_id: "s2".into(),
                parent_span_id: Some("s1".into()),
                op: "db.sql.query".into(),
                description: Some("select 1".into()),
                start: Timestamp::from_unix_seconds(start),
                duration_us: 1500,
                status: None,
                data: Value::Object(vec![("db.system".into(), Value::String("mysql".into()))]),
            }],
        }
    }

    #[tokio::test]
    async fn adds_up_rollups_across_flushes() {
        let (db, _dir) = database().await;
        let rollups = SqliteRollups::new(db);
        let request = |ms: u64, failed: bool| Measurement {
            kind: Kind::Request,
            name: "GET /".into(),
            duration_us: ms * 1000,
            flagged: failed,
        };
        rollups.add(scope(), 1_800_000_000, &[request(10, false), request(20, true)], 1).await.unwrap();
        rollups.flush().await.unwrap();
        rollups.add(scope(), 1_800_000_000, &[request(30, false)], 2).await.unwrap();
        rollups.add(scope(), 1_800_000_600, &[request(5, false)], 1).await.unwrap();
        let query = RollupQuery {
            projects: vec![ProjectId::new(1)],
            environments: vec![EnvironmentId::new(2)],
            kind: Kind::Request,
            from: Timestamp::from_unix_seconds(1_800_000_000),
            to: Timestamp::from_unix_seconds(1_800_003_600),
        };
        let names = rollups.names(&query).await.unwrap();
        assert_eq!(names.len(), 1);
        let (name, total) = &names[0];
        assert_eq!(name, "GET /");
        assert_eq!((total.count, total.flagged, total.max_us), (5, 1, 30_000));
        assert_eq!(total.total_us, 10_000 + 20_000 + 60_000 + 5_000);
        let series = rollups.series(&query, "GET /", 300).await.unwrap();
        assert_eq!(
            series.iter().map(|(b, r)| (*b, r.count)).collect::<Vec<_>>(),
            [(1_800_000_000, 4), (1_800_000_600, 1)]
        );
        let other = RollupQuery { kind: Kind::Query, ..query.clone() };
        assert!(rollups.names(&other).await.unwrap().is_empty());
        let elsewhere = RollupQuery { projects: vec![ProjectId::new(9)], ..query };
        assert!(rollups.names(&elsewhere).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn keeps_transactions_with_their_spans_in_segments() {
        let (db, dir) = database().await;
        let segments = Arc::new(Segments::new(dir.path().join("segments"), db));
        let store = SegmentTransactions::new(Arc::clone(&segments));
        let at = 1_800_000_000;
        for (start, ms, status) in [(at, 120, 200), (at + 10, 900, 500), (at + 20, 40, 200)] {
            let t = transaction("/orders", start, ms, status);
            store.insert(scope(), &t, Timestamp::from_unix_seconds(start)).await.unwrap();
        }
        let query = SampleQuery {
            projects: vec![],
            environments: vec![EnvironmentId::new(2)],
            kind: Kind::Request,
            name: "GET /orders".into(),
            from: Timestamp::from_unix_seconds(at - 60),
            to: Timestamp::from_unix_seconds(at + 60),
            slowest: true,
            limit: 2,
        };
        let slowest = store.samples(&query).await.unwrap();
        assert_eq!(slowest.iter().map(|s| s.duration_us).collect::<Vec<_>>(), [900_000, 120_000]);
        assert!(slowest[0].failed);
        let newest = store.samples(&SampleQuery { slowest: false, limit: 10, ..query }).await.unwrap();
        assert_eq!(newest[0].start, Timestamp::from_unix_seconds(at + 20));

        let trace = store.by_trace(&format!("trace-/orders-{}", at + 10)).await.unwrap();
        assert_eq!(trace.len(), 1);
        let kept = &trace[0].transaction;
        assert_eq!(kept, &transaction("/orders", at + 10, 900, 500));
        store.close().await;
        segments.close_readers().await;
    }
}
