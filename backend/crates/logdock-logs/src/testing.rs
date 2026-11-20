//! In-memory implementations of every port, for tests.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::{Clock, Day, PortError, ProjectId, Timestamp};
use tokio::sync::broadcast;

use crate::application::ports::{LogFeed, LogSink, LogStats, LogStore, Submitted};
use crate::application::{LogPorts, LogUseCases};
use crate::domain::{Bucket, HistogramQuery, LogEntry, LogId, LogLimits, LogPage, LogQuery, Source};

/// Keeps entries in a list and searches it with [`LogQuery::matches`].
pub struct MemoryLogs {
    pub entries: Mutex<Vec<LogEntry>>,
    feed: broadcast::Sender<Arc<LogEntry>>,
}

impl Default for MemoryLogs {
    fn default() -> Self {
        Self { entries: Mutex::new(Vec::new()), feed: broadcast::channel(64).0 }
    }
}

#[async_trait]
impl LogSink for MemoryLogs {
    async fn submit(&self, entries: Vec<LogEntry>, _wait: bool) -> Result<Submitted, PortError> {
        let mut stored = self.entries.lock().unwrap();
        for mut entry in entries {
            entry.id = Some(LogId { day: entry.received.day(), keep: 0, row: stored.len() as i64 + 1 });
            let _ = self.feed.send(Arc::new(entry.clone()));
            stored.push(entry);
        }
        Ok(Submitted::Accepted)
    }
}

#[async_trait]
impl LogStore for MemoryLogs {
    async fn search(&self, query: &LogQuery) -> Result<LogPage, PortError> {
        let mut found: Vec<LogEntry> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| query.matches(e))
            .filter(|e| query.before.is_none_or(|c| e.id.is_some_and(|id| c.is_before(e.time, id))))
            .cloned()
            .collect();
        found.sort_by_key(|e| std::cmp::Reverse((e.time, e.id)));
        found.truncate(query.limit.max(1) as usize);
        Ok(LogPage { entries: found, next: None })
    }

    async fn get(&self, id: LogId) -> Result<Option<LogEntry>, PortError> {
        Ok(self.entries.lock().unwrap().iter().find(|e| e.id == Some(id)).cloned())
    }
}

#[async_trait]
impl LogStats for MemoryLogs {
    async fn histogram(&self, _query: &HistogramQuery) -> Result<Vec<Bucket>, PortError> {
        Ok(Vec::new())
    }

    async fn sources(&self, _projects: &[ProjectId]) -> Result<Vec<Source>, PortError> {
        Ok(Vec::new())
    }
}

impl LogFeed for MemoryLogs {
    fn subscribe(&self) -> broadcast::Receiver<Arc<LogEntry>> {
        self.feed.subscribe()
    }
}

struct Epoch;

impl Clock for Epoch {
    fn now(&self) -> Timestamp {
        Day::from_days(20_000).start()
    }
}

/// The context with every port in memory.
pub fn memory() -> (LogUseCases, Arc<MemoryLogs>) {
    let logs = Arc::new(MemoryLogs::default());
    let ports = LogPorts {
        sink: logs.clone(),
        store: logs.clone(),
        stats: logs.clone(),
        feed: logs.clone(),
        clock: Arc::new(Epoch),
    };
    (LogUseCases::new(ports, LogLimits::default()), logs)
}
