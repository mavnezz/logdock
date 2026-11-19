//! What the writers report about themselves, and the readiness probe built
//! on it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use logdock_kernel::PortError;
use logdock_retention::application::ports::StorageProbe;

use crate::sqlite::Database;

/// A writer counts as stalled when work has waited this long without a
/// commit.
const STALLED_AFTER: Duration = Duration::from_secs(30);

/// The writers' state, shared between them and the probe. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct WriterHealth {
    writers: Arc<Mutex<Vec<Writer>>>,
}

#[derive(Debug)]
struct Writer {
    name: &'static str,
    /// Since when work has been waiting, if any is.
    waiting_since: Option<Instant>,
}

impl WriterHealth {
    /// A writer named `name` reports through the handle.
    pub fn register(&self, name: &'static str) -> WriterHandle {
        let mut writers = self.writers.lock().unwrap_or_else(|e| e.into_inner());
        let index = writers.len();
        writers.push(Writer { name, waiting_since: None });
        WriterHandle { health: self.clone(), index }
    }

    /// Writers whose work has waited longer than `after`.
    pub fn stalled(&self, after: Duration) -> Vec<&'static str> {
        let writers = self.writers.lock().unwrap_or_else(|e| e.into_inner());
        writers
            .iter()
            .filter(|writer| writer.waiting_since.is_some_and(|since| since.elapsed() >= after))
            .map(|writer| writer.name)
            .collect()
    }

    fn update(&self, index: usize, waiting_since: Option<Instant>) {
        let mut writers = self.writers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(writer) = writers.get_mut(index) {
            writer.waiting_since = waiting_since;
        }
    }
}

/// How one writer reports: work arrived, work committed.
#[derive(Debug, Clone)]
pub struct WriterHandle {
    health: WriterHealth,
    index: usize,
}

impl WriterHandle {
    /// Work arrived; the first arrival since the last commit counts.
    pub fn waiting(&self) {
        let mut writers = self.health.writers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(writer) = writers.get_mut(self.index)
            && writer.waiting_since.is_none()
        {
            writer.waiting_since = Some(Instant::now());
        }
    }

    /// Everything that waited is committed.
    pub fn committed(&self) {
        self.health.update(self.index, None);
    }
}

/// [`StorageProbe`] on the main database, the data directory and the
/// writers' reports.
#[derive(Debug, Clone)]
pub struct DataDirProbe {
    db: Database,
    data_dir: PathBuf,
    writers: WriterHealth,
}

impl DataDirProbe {
    pub fn new(db: Database, data_dir: PathBuf, writers: WriterHealth) -> Self {
        Self { db, data_dir, writers }
    }
}

/// Free bytes of the file system that holds `path`, for unprivileged users.
pub fn free_bytes(path: &std::path::Path) -> std::io::Result<u64> {
    let stat = rustix::fs::statvfs(path)?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

#[async_trait]
impl StorageProbe for DataDirProbe {
    async fn database_ok(&self) -> bool {
        self.db.ping().await
    }

    async fn free_bytes(&self) -> Result<u64, PortError> {
        let dir = self.data_dir.clone();
        tokio::task::spawn_blocking(move || free_bytes(&dir))
            .await
            .map_err(|e| PortError::new("data directory", e))?
            .map_err(|e| PortError::new("data directory", e))
    }

    async fn stalled_writers(&self) -> Vec<&'static str> {
        self.writers.stalled(STALLED_AFTER)
    }

    async fn database_bytes(&self) -> Result<u64, PortError> {
        let path = self.db.path().display().to_string();
        Ok(["", "-wal", "-shm"]
            .iter()
            .filter_map(|suffix| std::fs::metadata(format!("{path}{suffix}")).ok())
            .map(|meta| meta.len())
            .sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writer_is_stalled_while_its_work_waits() {
        let health = WriterHealth::default();
        let logs = health.register("logs");
        let _events = health.register("events");
        assert!(health.stalled(Duration::ZERO).is_empty());
        logs.waiting();
        assert_eq!(health.stalled(Duration::ZERO), ["logs"]);
        assert!(health.stalled(Duration::from_secs(60)).is_empty());
        logs.committed();
        assert!(health.stalled(Duration::ZERO).is_empty());
    }

    #[tokio::test]
    async fn probes_the_database_and_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (db, _) = Database::open(&dir.path().join("logdock.db")).await.unwrap();
        let probe = DataDirProbe::new(db, dir.path().to_owned(), WriterHealth::default());
        assert!(probe.database_ok().await);
        assert!(probe.free_bytes().await.unwrap() > 0);
        assert!(probe.stalled_writers().await.is_empty());
        assert!(probe.database_bytes().await.unwrap() > 0);
    }
}
