//! What ingest accepted and refused, per project, key and day. Requests add
//! to counts in memory; [`SqliteIngestCounter::flush`] writes them, every
//! few seconds and before every read, so ingest costs no write of its own.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;
use logdock_kernel::{Day, PortError, ProjectId, Timestamp};
use logdock_projects::application::ports::IngestCounter;
use logdock_projects::domain::{IngestCount, KeyId, Rejection, Tally};
use sqlx::Row;

use super::Database;

#[derive(Debug, Default, Clone)]
struct Pending {
    accepted: u64,
    skipped: u64,
    rejected: BTreeMap<Rejection, u64>,
    last: Option<(Rejection, String, Timestamp)>,
}

impl Pending {
    fn merge(&mut self, other: Pending) {
        self.accepted += other.accepted;
        self.skipped += other.skipped;
        for (reason, count) in other.rejected {
            *self.rejected.entry(reason).or_default() += count;
        }
        if other.last.is_some() {
            self.last = other.last;
        }
    }
}

#[derive(Debug)]
pub struct SqliteIngestCounter {
    db: Database,
    pending: Mutex<HashMap<(i64, i64, i64), Pending>>,
}

impl SqliteIngestCounter {
    pub fn new(db: Database) -> Self {
        Self { db, pending: Mutex::new(HashMap::new()) }
    }

    /// Writes what was counted since the last flush; what it could not
    /// write is kept for the next one.
    pub async fn flush(&self) -> Result<(), PortError> {
        let pending = std::mem::take(&mut *self.pending.lock().unwrap_or_else(|e| e.into_inner()));
        if pending.is_empty() {
            return Ok(());
        }
        let result = self.write(&pending).await;
        if result.is_err() {
            let mut kept = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            for (key, counts) in pending {
                kept.entry(key).or_default().merge(counts);
            }
        }
        result
    }

    async fn write(&self, pending: &HashMap<(i64, i64, i64), Pending>) -> Result<(), PortError> {
        let port = |e| PortError::new("ingest counts", e);
        let mut tx = self.db.write().await.map_err(port)?;
        for (&(project, key, day), counts) in pending {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT rejected FROM ingest_counts WHERE project_id = ? AND key_id = ? AND day = ?",
            )
            .bind(project)
            .bind(key)
            .bind(day)
            .fetch_optional(&mut *tx)
            .await
            .map_err(port)?;
            let mut rejected: BTreeMap<String, u64> =
                existing.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
            for (reason, count) in &counts.rejected {
                *rejected.entry(reason.as_str().to_owned()).or_default() += count;
            }
            let rejected = serde_json::to_string(&rejected).unwrap_or_else(|_| "{}".to_owned());
            let (last_reason, last_detail, last_at) = match counts.last.clone() {
                Some((reason, detail, at)) => (Some(reason.as_str()), Some(detail), Some(at.unix_micros())),
                None => (None, None, None),
            };
            // A project deleted meanwhile takes its counts with it.
            sqlx::query(
                "INSERT INTO ingest_counts (project_id, key_id, day, accepted, skipped, rejected, last_reason, last_detail, last_at) \
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9 WHERE EXISTS (SELECT 1 FROM projects WHERE id = ?1) \
                 ON CONFLICT (project_id, key_id, day) DO UPDATE SET \
                   accepted = accepted + excluded.accepted, skipped = skipped + excluded.skipped, \
                   rejected = excluded.rejected, \
                   last_reason = coalesce(excluded.last_reason, last_reason), \
                   last_detail = coalesce(excluded.last_detail, last_detail), \
                   last_at = coalesce(excluded.last_at, last_at)",
            )
            .bind(project)
            .bind(key)
            .bind(day)
            .bind(counts.accepted as i64)
            .bind(counts.skipped as i64)
            .bind(rejected)
            .bind(last_reason)
            .bind(last_detail)
            .bind(last_at)
            .execute(&mut *tx)
            .await
            .map_err(port)?;
        }
        tx.commit().await.map_err(port)
    }
}

#[async_trait]
impl IngestCounter for SqliteIngestCounter {
    async fn record(&self, project: ProjectId, key: Option<KeyId>, tally: &Tally, at: Timestamp) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let entry = pending.entry((project.get(), key.map_or(0, KeyId::get), at.day().days())).or_default();
        entry.accepted += tally.accepted;
        entry.skipped += tally.skipped;
        for (reason, count) in &tally.rejected {
            *entry.rejected.entry(*reason).or_default() += count;
        }
        if let (Some((reason, _)), Some(detail)) = (tally.rejected.last(), &tally.detail) {
            entry.last = Some((*reason, detail.chars().take(300).collect(), at));
        }
    }

    async fn counts(&self, project: ProjectId, since: Day) -> Result<Vec<IngestCount>, PortError> {
        self.flush().await?;
        let rows = sqlx::query(
            "SELECT * FROM ingest_counts WHERE project_id = ? AND day >= ? ORDER BY day DESC, key_id",
        )
        .bind(project.get())
        .bind(since.days())
        .fetch_all(self.db.pool())
        .await
        .map_err(|e| PortError::new("ingest counts", e))?;
        Ok(rows
            .iter()
            .map(|row| {
                let key: i64 = row.get("key_id");
                let rejected: String = row.get("rejected");
                let rejected: BTreeMap<String, u64> = serde_json::from_str(&rejected).unwrap_or_default();
                let reason: Option<String> = row.get("last_reason");
                let last = match (
                    reason.as_deref().and_then(Rejection::parse),
                    row.get::<Option<i64>, _>("last_at"),
                ) {
                    (Some(reason), Some(at)) => Some((
                        reason,
                        row.get::<Option<String>, _>("last_detail").unwrap_or_default(),
                        Timestamp::from_unix_micros(at),
                    )),
                    _ => None,
                };
                IngestCount {
                    day: Day::from_days(row.get("day")),
                    key: (key != 0).then(|| KeyId::new(key)),
                    accepted: row.get::<i64, _>("accepted") as u64,
                    skipped: row.get::<i64, _>("skipped") as u64,
                    rejected: rejected
                        .into_iter()
                        .filter_map(|(r, n)| Some((Rejection::parse(&r)?, n)))
                        .collect(),
                    last_rejection: last,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::sqlite::repository_tests::database;

    async fn with_project(db: &Database) {
        sqlx::query("INSERT INTO projects (id, name, created_at) VALUES (1, 'orders', 0)")
            .execute(db.pool())
            .await
            .unwrap();
    }

    fn at() -> Timestamp {
        Timestamp::from_unix_seconds(1_800_000_000)
    }

    /// Another connection writes while the flush runs: the flush waits for
    /// it, instead of failing with SQLITE_BUSY_SNAPSHOT as a transaction that
    /// reads before it writes does.
    #[tokio::test]
    async fn waits_for_another_writer() {
        let (db, _dir) = database().await;
        with_project(&db).await;
        let counter = SqliteIngestCounter::new(db.clone());
        counter.record(ProjectId::new(1), None, &Tally::accepted(3), at()).await;
        counter.flush().await.unwrap();
        counter.record(ProjectId::new(1), None, &Tally::accepted(4), at()).await;

        let mut other = db.write().await.unwrap();
        sqlx::query("UPDATE projects SET name = 'orders-backend' WHERE id = 1")
            .execute(&mut *other)
            .await
            .unwrap();
        let flush = tokio::spawn(async move { counter.flush().await.map(|()| counter) });
        tokio::time::sleep(Duration::from_millis(200)).await;
        other.commit().await.unwrap();
        let counter = flush.await.unwrap().expect("the flush waited for the lock");
        let counts = counter.counts(ProjectId::new(1), at().day()).await.unwrap();
        assert_eq!(counts[0].accepted, 7);
    }

    #[tokio::test]
    async fn keeps_what_it_could_not_write() {
        let (db, _dir) = database().await;
        with_project(&db).await;
        let counter = SqliteIngestCounter::new(db.clone());
        counter.record(ProjectId::new(1), None, &Tally::accepted(2), at()).await;
        counter
            .record(ProjectId::new(1), None, &Tally::rejected(Rejection::Rate, 1, "slow down"), at())
            .await;
        db.close().await;
        assert!(counter.flush().await.is_err());
        let kept = counter.pending.lock().unwrap();
        let counts = kept.get(&(1, 0, at().day().days())).unwrap();
        assert_eq!((counts.accepted, counts.rejected.get(&Rejection::Rate)), (2, Some(&1)));
    }
}
