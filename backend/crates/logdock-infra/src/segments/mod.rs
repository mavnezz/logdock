//! Day segments (docs/architecture.md, 5.1): one SQLite file per kind of
//! data and day of receipt, `segments/<kind>/<YYYY-MM-DD>.db`, and their
//! catalog in the main database.
//!
//! Writers hold one connection to today's segment; readers get small
//! read-only pools, kept for the segments asked for lately. Deleting a
//! segment deletes its file: that is retention and the storage limit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use logdock_kernel::{Day, PortError, Timestamp};
use logdock_retention::domain::Kind;
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

/// A segment as the catalog knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    pub kind: Kind,
    pub day: Day,
    pub rows: i64,
    pub min_time: Option<Timestamp>,
    pub max_time: Option<Timestamp>,
    pub closed: bool,
}

#[derive(Debug)]
pub struct Segments {
    root: PathBuf,
    db: Database,
    readers: Mutex<HashMap<(Kind, Day), (SqlitePool, u64)>>,
    uses: std::sync::atomic::AtomicU64,
}

pub(crate) fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("segments", error)
}

impl Segments {
    pub fn new(root: PathBuf, db: Database) -> Self {
        Self { root, db, readers: Mutex::new(HashMap::new()), uses: std::sync::atomic::AtomicU64::new(0) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn path(&self, kind: Kind, day: Day) -> PathBuf {
        self.root.join(kind.as_str()).join(format!("{day}.db"))
    }

    /// The writer's connection to a segment, which is created with its
    /// schema and entered into the catalog if it is new.
    pub async fn open_writer(&self, kind: Kind, day: Day) -> Result<SqliteConnection, PortError> {
        let path = self.path(kind, day);
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
        sqlx::query("INSERT INTO segments (kind, day) VALUES (?, ?) ON CONFLICT (kind, day) DO NOTHING")
            .bind(kind.as_str())
            .bind(day.days())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(connection)
    }

    /// A read-only pool on a segment; `None` if it does not exist.
    pub async fn reader(&self, kind: Kind, day: Day) -> Result<Option<SqlitePool>, PortError> {
        let tick = self.uses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some((pool, used)) =
            self.readers.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&(kind, day))
        {
            *used = tick;
            return Ok(Some(pool.clone()));
        }
        let path = self.path(kind, day);
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
            readers.insert((kind, day), (pool.clone(), tick));
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
            "SELECT day, rows, min_time, max_time, closed FROM segments \
             WHERE kind = ? AND rows > 0 AND max_time >= ? AND min_time < ? ORDER BY max_time DESC, day DESC",
        )
        .bind(kind.as_str())
        .bind(from.map_or(i64::MIN, Timestamp::unix_micros))
        .bind(to.map_or(i64::MAX, Timestamp::unix_micros))
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        Ok(rows.iter().map(|row| entry(kind, row)).collect())
    }

    /// Every segment the catalog knows.
    pub async fn all(&self) -> Result<Vec<CatalogEntry>, PortError> {
        let rows = sqlx::query(
            "SELECT kind, day, rows, min_time, max_time, closed FROM segments ORDER BY day, kind",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(port)?;
        Ok(rows.iter().filter_map(|row| Kind::parse(row.get("kind")).map(|kind| entry(kind, row))).collect())
    }

    /// Adds a committed batch to a segment's catalog entry.
    pub async fn note_batch(
        &self,
        kind: Kind,
        day: Day,
        rows: i64,
        min_time: Timestamp,
        max_time: Timestamp,
    ) -> Result<(), PortError> {
        sqlx::query(
            "UPDATE segments SET rows = rows + ?1, \
             min_time = min(coalesce(min_time, ?2), ?2), max_time = max(coalesce(max_time, ?3), ?3) \
             WHERE kind = ?4 AND day = ?5",
        )
        .bind(rows)
        .bind(min_time.unix_micros())
        .bind(max_time.unix_micros())
        .bind(kind.as_str())
        .bind(day.days())
        .execute(self.db.pool())
        .await
        .map_err(port)?;
        Ok(())
    }

    /// The size of a segment's files on disk.
    pub fn bytes(&self, kind: Kind, day: Day) -> u64 {
        let path = self.path(kind, day);
        ["", "-wal", "-shm"]
            .iter()
            .filter_map(|suffix| std::fs::metadata(format!("{}{suffix}", path.display())).ok())
            .map(|meta| meta.len())
            .sum()
    }

    /// Merges a finished day's index and marks it closed: it never changes
    /// again.
    pub async fn close(&self, kind: Kind, day: Day) -> Result<(), PortError> {
        let path = self.path(kind, day);
        if path.is_file() {
            let options = SqliteConnectOptions::new().filename(&path).busy_timeout(Duration::from_secs(30));
            let mut connection = SqliteConnection::connect_with(&options).await.map_err(port)?;
            if kind == Kind::Logs {
                sqlx::query("INSERT INTO logs_fts (logs_fts) VALUES ('optimize')")
                    .execute(&mut connection)
                    .await
                    .map_err(port)?;
            }
            sqlx::query("PRAGMA optimize").execute(&mut connection).await.map_err(port)?;
            sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&mut connection).await.map_err(port)?;
            connection.close().await.map_err(port)?;
        }
        sqlx::query("UPDATE segments SET closed = 1 WHERE kind = ? AND day = ?")
            .bind(kind.as_str())
            .bind(day.days())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    /// Deletes a segment: its readers, its files, its catalog entry.
    pub async fn delete(&self, kind: Kind, day: Day) -> Result<(), PortError> {
        let pool = self.readers.lock().unwrap_or_else(|e| e.into_inner()).remove(&(kind, day));
        if let Some((pool, _)) = pool {
            pool.close().await;
        }
        let path = self.path(kind, day);
        for suffix in ["", "-wal", "-shm"] {
            match std::fs::remove_file(format!("{}{suffix}", path.display())) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(port(error)),
            }
        }
        sqlx::query("DELETE FROM segments WHERE kind = ? AND day = ?")
            .bind(kind.as_str())
            .bind(day.days())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
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
                bytes: self.bytes(entry.kind, entry.day),
            })
            .collect())
    }

    async fn delete(&self, kind: Kind, day: Day) -> Result<(), PortError> {
        Segments::delete(self, kind, day).await
    }

    async fn close_before(&self, today: Day) -> Result<u32, PortError> {
        let mut closed = 0;
        for entry in self.all().await?.into_iter().filter(|e| !e.closed && e.day < today) {
            Segments::close(self, entry.kind, entry.day).await?;
            closed += 1;
        }
        Ok(closed)
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
        for statement in ["DELETE FROM log_rollups WHERE minute < ?"] {
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

    #[tokio::test]
    async fn creates_catalogs_reads_closes_and_deletes_segments() {
        let (db, dir) = database().await;
        let segments = Segments::new(dir.path().join("segments"), db);
        let day = Day::from_days(20_000);
        assert!(segments.reader(Kind::Logs, day).await.unwrap().is_none());
        let mut writer = segments.open_writer(Kind::Logs, day).await.unwrap();
        sqlx::query("INSERT INTO logs (time, received, project_id, environment_id, level, message, attributes) VALUES (5, 5, 1, 1, 3, 'hi', '{}')")
            .execute(&mut writer)
            .await
            .unwrap();
        segments
            .note_batch(Kind::Logs, day, 1, Timestamp::from_unix_micros(5), Timestamp::from_unix_micros(5))
            .await
            .unwrap();
        segments
            .note_batch(Kind::Logs, day, 2, Timestamp::from_unix_micros(3), Timestamp::from_unix_micros(4))
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

        let reader = segments.reader(Kind::Logs, day).await.unwrap().unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM logs").fetch_one(&reader).await.unwrap();
        assert_eq!(count, 1);
        assert!(segments.bytes(Kind::Logs, day) > 0);
        writer.close().await.unwrap();
        segments.close(Kind::Logs, day).await.unwrap();
        assert!(segments.all().await.unwrap()[0].closed);
        segments.delete(Kind::Logs, day).await.unwrap();
        assert!(!segments.path(Kind::Logs, day).exists());
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
            let writer = segments.open_writer(Kind::Logs, day).await.unwrap();
            writer.close().await.unwrap();
        }
        let listed = SegmentCatalog::segments(&segments).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|s| s.bytes > 0));
        assert_eq!(segments.close_before(Day::from_days(11)).await.unwrap(), 1);
        assert_eq!(segments.close_before(Day::from_days(11)).await.unwrap(), 0, "closed already");
        SegmentCatalog::delete(&segments, Kind::Logs, Day::from_days(10)).await.unwrap();
        assert_eq!(SegmentCatalog::segments(&segments).await.unwrap().len(), 1);

        sqlx::query("INSERT INTO log_rollups (project_id, environment_id, service, level, minute, count) VALUES (1, 1, '', 3, 60, 1), (1, 1, '', 3, 600, 1)")
            .execute(db.pool())
            .await
            .unwrap();
        let pruned =
            SqliteHousekeeping::new(db).prune_rollups(Timestamp::from_unix_seconds(300)).await.unwrap();
        assert_eq!(pruned, 1);
    }
}
