//! Day segments (docs/architecture.md, 5.1): one SQLite file per kind of
//! data and day of receipt, `segments/<kind>/<YYYY-MM-DD>.db`, and their
//! catalog in the main database. The data of stages that keep a kind for a
//! number of days of their own goes into segments of that retention,
//! `segments/<kind>/<YYYY-MM-DD>.<days>d.db`, which go when those days are
//! over (5.4).
//!
//! Writers hold one connection to each of today's segments; readers get
//! small read-only pools, kept for the segments asked for lately. Deleting
//! a segment deletes its file: that is retention and the storage limit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use logdock_kernel::{Day, EnvironmentId, PortError, ProjectId, Timestamp};
use logdock_retention::domain::{Kind, ScopeUsage, StageRetention};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
    SqliteSynchronous,
};
use sqlx::{Connection, Row};

use crate::sqlite::Database;

/// The schema of each kind's segments, applied when a segment is created.
fn schema(kind: Kind) -> &'static str {
    match kind {
        Kind::Logs => include_str!("logs.sql"),
        Kind::Events => include_str!("events.sql"),
        Kind::Traces => include_str!("traces.sql"),
    }
}

/// Reader pools kept open at most; older ones are closed.
const OPEN_READERS: usize = 48;

/// One segment: a kind, a day of receipt, and the retention of the stages
/// in it (`keep` days, 0 for the kind's own).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SegmentId {
    pub kind: Kind,
    pub day: Day,
    pub keep: u32,
}

impl SegmentId {
    pub fn new(kind: Kind, day: Day, keep: u32) -> Self {
        Self { kind, day, keep }
    }

    fn file_name(&self) -> String {
        if self.keep == 0 { format!("{}.db", self.day) } else { format!("{}.{}d.db", self.day, self.keep) }
    }
}

/// A segment as the catalog knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    pub kind: Kind,
    pub day: Day,
    pub keep: u32,
    pub rows: i64,
    pub min_time: Option<Timestamp>,
    pub max_time: Option<Timestamp>,
    pub closed: bool,
}

impl CatalogEntry {
    pub fn id(&self) -> SegmentId {
        SegmentId::new(self.kind, self.day, self.keep)
    }
}

/// How long the stages' retention is used before it is read again: a
/// changed setting applies within this time.
const KEEPS_FOR: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct Keeps {
    loaded: Option<Instant>,
    stages: HashMap<i64, StageRetention>,
}

#[derive(Debug)]
pub struct Segments {
    root: PathBuf,
    db: Database,
    readers: Mutex<HashMap<SegmentId, (SqlitePool, u64)>>,
    uses: std::sync::atomic::AtomicU64,
    keeps: tokio::sync::Mutex<Keeps>,
}

pub(crate) fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("segments", error)
}

impl Segments {
    pub fn new(root: PathBuf, db: Database) -> Self {
        Self {
            root,
            db,
            readers: Mutex::new(HashMap::new()),
            uses: std::sync::atomic::AtomicU64::new(0),
            keeps: tokio::sync::Mutex::new(Keeps::default()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn path(&self, id: SegmentId) -> PathBuf {
        self.root.join(id.kind.as_str()).join(id.file_name())
    }

    /// The segments a stage's data of `kind` goes into: those of the days
    /// the stage keeps it, or 0 for the kind's own.
    pub async fn keep(&self, kind: Kind, environment: EnvironmentId) -> Result<u32, PortError> {
        let mut keeps = self.keeps.lock().await;
        if keeps.loaded.is_none_or(|at| at.elapsed() > KEEPS_FOR) {
            let settings: Vec<String> = sqlx::query_scalar("SELECT settings FROM projects")
                .fetch_all(self.db.pool())
                .await
                .map_err(port)?;
            keeps.stages =
                settings.iter().flat_map(|json| crate::sqlite::projects::stage_retention(json)).collect();
            keeps.loaded = Some(Instant::now());
        }
        Ok(keeps.stages.get(&environment.get()).map_or(0, |stage| kind.kept_by(stage)))
    }

    /// Reads the stages' retention again at the next write, as after their
    /// settings changed.
    pub async fn forget_keeps(&self) {
        self.keeps.lock().await.loaded = None;
    }

    /// The writer's connection to a segment, which is created with its
    /// schema and entered into the catalog if it is new.
    pub async fn open_writer(&self, id: SegmentId) -> Result<SqliteConnection, PortError> {
        let (kind, day) = (id.kind, id.day);
        let path = self.path(id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(port)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10));
        let mut connection = SqliteConnection::connect_with(&options).await.map_err(port)?;
        sqlx::raw_sql(schema(kind)).execute(&mut connection).await.map_err(port)?;
        sqlx::query(
            "INSERT INTO segments (kind, day, keep) VALUES (?, ?, ?) ON CONFLICT (kind, day, keep) DO NOTHING",
        )
        .bind(kind.as_str())
        .bind(day.days())
        .bind(i64::from(id.keep))
        .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(connection)
    }

    /// A read-only pool on a segment; `None` if it does not exist.
    pub async fn reader(&self, id: SegmentId) -> Result<Option<SqlitePool>, PortError> {
        let tick = self.uses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some((pool, used)) = self.readers.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&id) {
            *used = tick;
            return Ok(Some(pool.clone()));
        }
        let path = self.path(id);
        if !path.is_file() {
            return Ok(None);
        }
        let options =
            SqliteConnectOptions::new().filename(&path).read_only(true).busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .idle_timeout(Duration::from_secs(300))
            .connect_with(options)
            .await
            .map_err(port)?;
        let evicted = {
            let mut readers = self.readers.lock().unwrap_or_else(|e| e.into_inner());
            readers.insert(id, (pool.clone(), tick));
            if readers.len() > OPEN_READERS {
                let oldest = readers.iter().min_by_key(|(_, (_, used))| *used).map(|(key, _)| *key);
                oldest.and_then(|key| readers.remove(&key)).map(|(pool, _)| pool)
            } else {
                None
            }
        };
        if let Some(pool) = evicted {
            pool.close().await;
        }
        Ok(Some(pool))
    }

    /// The catalog's segments of `kind` whose event times may overlap
    /// `[from, to)`, newest event first.
    pub async fn overlapping(
        &self,
        kind: Kind,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
    ) -> Result<Vec<CatalogEntry>, PortError> {
        let rows = sqlx::query(
            "SELECT kind, day, keep, rows, min_time, max_time, closed FROM segments \
             WHERE kind = ? AND rows > 0 AND max_time >= ? AND min_time < ? \
             ORDER BY max_time DESC, day DESC, keep",
        )
        .bind(kind.as_str())
        .bind(from.map_or(i64::MIN, Timestamp::unix_micros))
        .bind(to.map_or(i64::MAX, Timestamp::unix_micros))
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        Ok(rows.iter().map(|row| entry(kind, row)).collect())
    }

    /// The segments of `kind` on `days`, whatever their retention.
    pub async fn on_days(&self, kind: Kind, days: &[Day]) -> Result<Vec<SegmentId>, PortError> {
        Ok(self
            .all()
            .await?
            .iter()
            .filter(|e| e.kind == kind && days.contains(&e.day))
            .map(|e| e.id())
            .collect())
    }

    /// Every segment the catalog knows.
    pub async fn all(&self) -> Result<Vec<CatalogEntry>, PortError> {
        let rows = sqlx::query(
            "SELECT kind, day, keep, rows, min_time, max_time, closed FROM segments ORDER BY day, kind, keep",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        Ok(rows.iter().filter_map(|row| Kind::parse(row.get("kind")).map(|kind| entry(kind, row))).collect())
    }

    /// Adds a committed batch to a segment's catalog entry.
    pub async fn note_batch(
        &self,
        id: SegmentId,
        rows: i64,
        min_time: Timestamp,
        max_time: Timestamp,
    ) -> Result<(), PortError> {
        sqlx::query(
            "UPDATE segments SET rows = rows + ?1, \
             min_time = min(coalesce(min_time, ?2), ?2), max_time = max(coalesce(max_time, ?3), ?3) \
             WHERE kind = ?4 AND day = ?5 AND keep = ?6",
        )
        .bind(rows)
        .bind(min_time.unix_micros())
        .bind(max_time.unix_micros())
        .bind(id.kind.as_str())
        .bind(id.day.days())
        .bind(i64::from(id.keep))
        .execute(self.db.pool())
        .await
        .map_err(port)?;
        Ok(())
    }

    /// The size of a segment's files on disk.
    pub fn bytes(&self, id: SegmentId) -> u64 {
        let path = self.path(id);
        ["", "-wal", "-shm"]
            .iter()
            .filter_map(|suffix| std::fs::metadata(format!("{}{suffix}", path.display())).ok())
            .map(|meta| meta.len())
            .sum()
    }

    /// Merges a finished day's index and marks it closed: it never changes
    /// again.
    pub async fn close(&self, id: SegmentId) -> Result<(), PortError> {
        let path = self.path(id);
        if path.is_file() {
            let options = SqliteConnectOptions::new().filename(&path).busy_timeout(Duration::from_secs(30));
            let mut connection = SqliteConnection::connect_with(&options).await.map_err(port)?;
            if id.kind == Kind::Logs {
                sqlx::query("INSERT INTO logs_fts (logs_fts) VALUES ('optimize')")
                    .execute(&mut connection)
                    .await
                    .map_err(port)?;
            }
            sqlx::query("PRAGMA optimize").execute(&mut connection).await.map_err(port)?;
            sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&mut connection).await.map_err(port)?;
            connection.close().await.map_err(port)?;
        }
        sqlx::query("UPDATE segments SET closed = 1 WHERE kind = ? AND day = ? AND keep = ?")
            .bind(id.kind.as_str())
            .bind(id.day.days())
            .bind(i64::from(id.keep))
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    /// Deletes a segment: its readers, its files, its catalog entry and
    /// what was measured of it.
    pub async fn delete(&self, id: SegmentId) -> Result<(), PortError> {
        let pool = self.readers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        if let Some((pool, _)) = pool {
            pool.close().await;
        }
        let path = self.path(id);
        for suffix in ["", "-wal", "-shm"] {
            match std::fs::remove_file(format!("{}{suffix}", path.display())) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(port(error)),
            }
        }
        for table in ["segments", "segment_usage"] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DELETE FROM {table} WHERE kind = ? AND day = ? AND keep = ?"
            )))
            .bind(id.kind.as_str())
            .bind(id.day.days())
            .bind(i64::from(id.keep))
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        }
        Ok(())
    }

    /// Measures what each project's stage takes of a segment: its rows, and
    /// its share of the files by the size of its rows. Replaces what was
    /// measured of it before; a closed segment is marked as measured.
    pub async fn measure_one(&self, entry: &CatalogEntry) -> Result<(), PortError> {
        let id = entry.id();
        let Some(pool) = self.reader(id).await? else { return Ok(()) };
        let sql = match id.kind {
            Kind::Logs => {
                "SELECT project_id, environment_id, count(*) AS rows, \
                 sum(length(message) + coalesce(length(attributes), 0) + 64) AS size FROM logs GROUP BY 1, 2"
            }
            Kind::Events => {
                "SELECT project_id, environment_id, count(*) AS rows, \
                 sum(length(data) + length(message) + 64) AS size FROM events GROUP BY 1, 2"
            }
            Kind::Traces => {
                "SELECT project_id, environment_id, count(*) AS rows, \
                 sum(length(data) + length(name) + 64) AS size FROM transactions GROUP BY 1, 2"
            }
        };
        // A segment made but not written to yet has no table.
        let rows = match sqlx::query(sql).fetch_all(&pool).await {
            Ok(rows) => rows,
            Err(sqlx::Error::Database(error)) if error.message().contains("no such table") => Vec::new(),
            Err(error) => return Err(port(error)),
        };
        let file = self.bytes(id) as f64;
        let total: f64 = rows.iter().map(|r| r.get::<Option<i64>, _>("size").unwrap_or(0) as f64).sum();
        let mut tx = self.db.write().await.map_err(port)?;
        sqlx::query("DELETE FROM segment_usage WHERE kind = ? AND day = ? AND keep = ?")
            .bind(id.kind.as_str())
            .bind(id.day.days())
            .bind(i64::from(id.keep))
            .execute(&mut *tx)
            .await
            .map_err(port)?;
        for row in &rows {
            let size = row.get::<Option<i64>, _>("size").unwrap_or(0) as f64;
            let share = if total > 0.0 { size / total } else { 0.0 };
            sqlx::query(
                "INSERT INTO segment_usage (kind, day, keep, project_id, environment_id, rows, bytes) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id.kind.as_str())
            .bind(id.day.days())
            .bind(i64::from(id.keep))
            .bind(row.get::<i64, _>("project_id"))
            .bind(row.get::<i64, _>("environment_id"))
            .bind(row.get::<i64, _>("rows"))
            .bind((file * share).round() as i64)
            .execute(&mut *tx)
            .await
            .map_err(port)?;
        }
        sqlx::query("UPDATE segments SET measured = ? WHERE kind = ? AND day = ? AND keep = ?")
            .bind(entry.closed)
            .bind(id.kind.as_str())
            .bind(id.day.days())
            .bind(i64::from(id.keep))
            .execute(&mut *tx)
            .await
            .map_err(port)?;
        tx.commit().await.map_err(port)
    }

    /// Closes every reader, for shutdown.
    pub async fn close_readers(&self) {
        let pools: Vec<SqlitePool> = self
            .readers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, (pool, _))| pool)
            .collect();
        for pool in pools {
            pool.close().await;
        }
    }
}

#[async_trait::async_trait]
impl logdock_retention::application::ports::SegmentCatalog for Segments {
    async fn segments(&self) -> Result<Vec<logdock_retention::domain::Segment>, PortError> {
        Ok(self
            .all()
            .await?
            .into_iter()
            .map(|entry| logdock_retention::domain::Segment {
                kind: entry.kind,
                day: entry.day,
                keep: entry.keep,
                bytes: self.bytes(entry.id()),
            })
            .collect())
    }

    async fn delete(&self, segment: logdock_retention::domain::Segment) -> Result<(), PortError> {
        Segments::delete(self, SegmentId::new(segment.kind, segment.day, segment.keep)).await
    }

    async fn close_before(&self, today: Day) -> Result<u32, PortError> {
        let mut closed = 0;
        for entry in self.all().await?.into_iter().filter(|e| !e.closed && e.day < today) {
            Segments::close(self, entry.id()).await?;
            closed += 1;
        }
        Ok(closed)
    }

    async fn measure(&self) -> Result<u32, PortError> {
        let unmeasured: Vec<(String, i64, i64)> =
            sqlx::query_as("SELECT kind, day, keep FROM segments WHERE closed = 0 OR measured = 0")
                .fetch_all(self.db.pool())
                .await
                .map_err(port)?;
        let mut measured = 0;
        for entry in self.all().await? {
            let id = (entry.kind.as_str().to_owned(), entry.day.days(), i64::from(entry.keep));
            if unmeasured.contains(&id) {
                self.measure_one(&entry).await?;
                measured += 1;
            }
        }
        Ok(measured)
    }

    async fn usage(&self) -> Result<Vec<ScopeUsage>, PortError> {
        let rows = sqlx::query(
            "SELECT kind, project_id, environment_id, sum(rows) AS rows, sum(bytes) AS bytes FROM segment_usage \
             GROUP BY kind, project_id, environment_id ORDER BY bytes DESC",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                Some(ScopeUsage {
                    kind: Kind::parse(row.get("kind"))?,
                    project: ProjectId::new(row.get("project_id")),
                    environment: EnvironmentId::new(row.get("environment_id")),
                    rows: row.get::<i64, _>("rows") as u64,
                    bytes: row.get::<i64, _>("bytes") as u64,
                })
            })
            .collect())
    }
}

/// A writer's connections to today's segments of one kind, one per
/// retention. A new day closes the connections of the one before and, when
/// asked to, its segments in the background.
#[derive(Debug)]
pub struct Writers {
    kind: Kind,
    close_finished: bool,
    day: Option<Day>,
    open: HashMap<u32, SqliteConnection>,
}

impl Writers {
    pub fn new(kind: Kind, close_finished: bool) -> Self {
        Self { kind, close_finished, day: None, open: HashMap::new() }
    }

    /// The connection to segment `id`, of this kind and today. Asked for a
    /// later day, it closes the connections of the one before.
    pub async fn get(
        &mut self,
        segments: &Arc<Segments>,
        id: SegmentId,
    ) -> Result<&mut SqliteConnection, PortError> {
        if self.day != Some(id.day) {
            let old = self.day.replace(id.day);
            let finished: Vec<u32> = self.open.keys().copied().collect();
            self.close().await;
            if let Some(old) = old
                && old < id.day
                && self.close_finished
            {
                for keep in finished {
                    let (segments, done) = (Arc::clone(segments), SegmentId::new(self.kind, old, keep));
                    tokio::spawn(async move {
                        if let Err(error) = segments.close(done).await {
                            tracing::warn!(%error, day = %done.day, "cannot close the segment");
                        }
                    });
                }
            }
        }
        match self.open.entry(id.keep) {
            std::collections::hash_map::Entry::Occupied(open) => Ok(open.into_mut()),
            std::collections::hash_map::Entry::Vacant(new) => Ok(new.insert(segments.open_writer(id).await?)),
        }
    }

    /// Closes every connection, as at shutdown.
    pub async fn close(&mut self) {
        for (_, connection) in self.open.drain() {
            let _ = connection.close().await;
        }
    }
}

/// [`Housekeeping`](logdock_retention::application::ports::Housekeeping) on
/// the main database's counts.
#[derive(Debug, Clone)]
pub struct SqliteHousekeeping {
    db: Database,
}

impl SqliteHousekeeping {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl logdock_retention::application::ports::Housekeeping for SqliteHousekeeping {
    async fn prune_rollups(&self, before: Timestamp) -> Result<u64, PortError> {
        let mut pruned = 0;
        for statement in [
            "DELETE FROM log_rollups WHERE minute < ?",
            "DELETE FROM perf_rollups WHERE minute < ?",
            // Check-ins in microseconds, rollups in seconds.
            "DELETE FROM check_ins WHERE started < ? * 1000000",
        ] {
            let done = sqlx::query(statement)
                .bind(before.unix_seconds())
                .execute(self.db.pool())
                .await
                .map_err(port)?;
            pruned += done.rows_affected();
        }
        Ok(pruned)
    }
}

fn entry(kind: Kind, row: &sqlx::sqlite::SqliteRow) -> CatalogEntry {
    CatalogEntry {
        kind,
        day: Day::from_days(row.get("day")),
        keep: row.get::<i64, _>("keep") as u32,
        rows: row.get("rows"),
        min_time: row.get::<Option<i64>, _>("min_time").map(Timestamp::from_unix_micros),
        max_time: row.get::<Option<i64>, _>("max_time").map(Timestamp::from_unix_micros),
        closed: row.get("closed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::repository_tests::database;

    fn logs(day: Day) -> SegmentId {
        SegmentId::new(Kind::Logs, day, 0)
    }

    #[tokio::test]
    async fn creates_catalogs_reads_closes_and_deletes_segments() {
        let (db, dir) = database().await;
        let segments = Segments::new(dir.path().join("segments"), db);
        let day = Day::from_days(20_000);
        assert!(segments.reader(logs(day)).await.unwrap().is_none());
        let mut writer = segments.open_writer(logs(day)).await.unwrap();
        sqlx::query("INSERT INTO logs (time, received, project_id, environment_id, level, message, attributes) VALUES (5, 5, 1, 1, 3, 'hi', '{}')")
            .execute(&mut writer)
            .await
            .unwrap();
        segments
            .note_batch(logs(day), 1, Timestamp::from_unix_micros(5), Timestamp::from_unix_micros(5))
            .await
            .unwrap();
        segments
            .note_batch(logs(day), 2, Timestamp::from_unix_micros(3), Timestamp::from_unix_micros(4))
            .await
            .unwrap();
        let found =
            segments.overlapping(Kind::Logs, Some(Timestamp::from_unix_micros(4)), None).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].rows, found[0].min_time, found[0].max_time),
            (3, Some(Timestamp::from_unix_micros(3)), Some(Timestamp::from_unix_micros(5)))
        );
        assert!(
            segments
                .overlapping(Kind::Logs, Some(Timestamp::from_unix_micros(6)), None)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(segments.overlapping(Kind::Events, None, None).await.unwrap().is_empty());

        let reader = segments.reader(logs(day)).await.unwrap().unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM logs").fetch_one(&reader).await.unwrap();
        assert_eq!(count, 1);
        assert!(segments.bytes(logs(day)) > 0);
        writer.close().await.unwrap();
        segments.close(logs(day)).await.unwrap();
        assert!(segments.all().await.unwrap()[0].closed);
        segments.delete(logs(day)).await.unwrap();
        assert!(!segments.path(logs(day)).exists());
        assert!(segments.all().await.unwrap().is_empty());
        segments.close_readers().await;
        assert_eq!(segments.root(), dir.path().join("segments"));
    }

    #[tokio::test]
    async fn serves_as_the_retention_catalog() {
        use logdock_retention::application::ports::{Housekeeping, SegmentCatalog};

        let (db, dir) = database().await;
        let segments = Segments::new(dir.path().join("segments"), db.clone());
        for day in [Day::from_days(10), Day::from_days(11)] {
            let writer = segments.open_writer(logs(day)).await.unwrap();
            writer.close().await.unwrap();
        }
        let listed = SegmentCatalog::segments(&segments).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|s| s.bytes > 0 && s.keep == 0));
        assert_eq!(segments.close_before(Day::from_days(11)).await.unwrap(), 1);
        assert_eq!(segments.close_before(Day::from_days(11)).await.unwrap(), 0, "closed already");
        SegmentCatalog::delete(&segments, listed[0]).await.unwrap();
        assert_eq!(SegmentCatalog::segments(&segments).await.unwrap().len(), 1);

        sqlx::query("INSERT INTO log_rollups (project_id, environment_id, service, level, minute, count) VALUES (1, 1, '', 3, 60, 1), (1, 1, '', 3, 600, 1)")
            .execute(db.pool())
            .await
            .unwrap();
        let pruned =
            SqliteHousekeeping::new(db).prune_rollups(Timestamp::from_unix_seconds(300)).await.unwrap();
        assert_eq!(pruned, 1);
    }

    #[tokio::test]
    async fn keeps_a_stages_own_retention_in_segments_of_its_own() {
        use logdock_retention::application::ports::SegmentCatalog;

        let (db, dir) = database().await;
        // Stage 2 keeps its logs 3 days; stage 1 as the server says.
        let settings = r#"{"retention": [{"environment": 2, "logs_days": 3}]}"#;
        sqlx::query("INSERT INTO projects (id, name, settings, created_at) VALUES (1, 'orders', ?, 0)")
            .bind(settings)
            .execute(db.pool())
            .await
            .unwrap();
        let segments = Arc::new(Segments::new(dir.path().join("segments"), db.clone()));
        let (prod, dev) = (EnvironmentId::new(1), EnvironmentId::new(2));
        assert_eq!(segments.keep(Kind::Logs, prod).await.unwrap(), 0);
        assert_eq!(segments.keep(Kind::Logs, dev).await.unwrap(), 3);
        assert_eq!(segments.keep(Kind::Events, dev).await.unwrap(), 0, "events as the server says");

        // One writer, today's segments of both; a new day closes them.
        let day = Day::from_days(20_000);
        let mut writers = Writers::new(Kind::Logs, true);
        for (environment, message) in [(1, "x".repeat(300)), (2, "y".repeat(100)), (2, "z".repeat(100))] {
            let keep = segments.keep(Kind::Logs, EnvironmentId::new(environment)).await.unwrap();
            let id = SegmentId::new(Kind::Logs, day, keep);
            let connection = writers.get(&segments, id).await.unwrap();
            sqlx::query("INSERT INTO logs (time, received, project_id, environment_id, level, message, attributes) VALUES (5, 5, 1, ?, 3, ?, '{}')")
                .bind(environment)
                .bind(message)
                .execute(connection)
                .await
                .unwrap();
            segments
                .note_batch(id, 1, Timestamp::from_unix_micros(5), Timestamp::from_unix_micros(5))
                .await
                .unwrap();
        }
        let own = SegmentId::new(Kind::Logs, day, 3);
        assert!(segments.path(own).ends_with("logs/2024-10-04.3d.db"), "{}", segments.path(own).display());
        assert_eq!(segments.overlapping(Kind::Logs, None, None).await.unwrap().len(), 2);
        assert_eq!(segments.on_days(Kind::Logs, &[day]).await.unwrap(), [logs(day), own]);
        let tomorrow = SegmentId::new(Kind::Logs, Day::from_days(20_001), 0);
        writers.get(&segments, tomorrow).await.unwrap();
        writers.close().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let closed: Vec<bool> =
            segments.all().await.unwrap().iter().filter(|e| e.day == day).map(|e| e.closed).collect();
        assert_eq!(closed, [true, true], "yesterday's closed in the background");

        // What each stage takes: rows, and its share of each file.
        assert_eq!(SegmentCatalog::measure(segments.as_ref()).await.unwrap(), 3);
        assert_eq!(SegmentCatalog::measure(segments.as_ref()).await.unwrap(), 1, "only today's again");
        let usage = SegmentCatalog::usage(segments.as_ref()).await.unwrap();
        let of = |environment: i64| usage.iter().find(|u| u.environment.get() == environment).unwrap();
        assert_eq!((of(1).rows, of(2).rows), (1, 2));
        assert_eq!(of(1).bytes, segments.bytes(logs(day)));
        assert_eq!(of(2).bytes, segments.bytes(own));

        // Deleting a segment deletes what was measured of it.
        segments.delete(own).await.unwrap();
        let usage = SegmentCatalog::usage(segments.as_ref()).await.unwrap();
        assert!(usage.iter().all(|u| u.environment.get() == 1));
        segments.forget_keeps().await;
        assert_eq!(segments.keep(Kind::Logs, dev).await.unwrap(), 3);
    }
}
