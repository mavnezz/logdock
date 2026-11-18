//! What this context needs from the outside world.

use std::sync::Arc;

use async_trait::async_trait;
use logdock_kernel::{PortError, ProjectId};
use tokio::sync::broadcast;

use crate::domain::{Bucket, HistogramQuery, LogEntry, LogId, LogPage, LogQuery, Source};

/// Whether the writer took the entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submitted {
    Accepted,
    /// Its queue is full: the client should come back later.
    Overloaded,
}

/// Where entries go to be written.
#[async_trait]
pub trait LogSink: Send + Sync {
    /// Queues `entries`; with `wait`, answers once they are committed.
    async fn submit(&self, entries: Vec<LogEntry>, wait: bool) -> Result<Submitted, PortError>;
}

/// Where written entries are found again.
#[async_trait]
pub trait LogStore: Send + Sync {
    async fn search(&self, query: &LogQuery) -> Result<LogPage, PortError>;

    async fn get(&self, id: LogId) -> Result<Option<LogEntry>, PortError>;
}

/// Counts kept beside the entries.
#[async_trait]
pub trait LogStats: Send + Sync {
    async fn histogram(&self, query: &HistogramQuery) -> Result<Vec<Bucket>, PortError>;

    /// Services and hosts seen, for the filters.
    async fn sources(&self, projects: &[ProjectId]) -> Result<Vec<Source>, PortError>;
}

/// Entries as they are committed, for the live view.
pub trait LogFeed: Send + Sync {
    fn subscribe(&self) -> broadcast::Receiver<Arc<LogEntry>>;
}
