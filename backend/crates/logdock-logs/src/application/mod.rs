//! Use cases of the Logs context and the ports they need.

pub mod ports;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use logdock_kernel::redaction::{RedactionRules, Redactor};
use logdock_kernel::{Clock, PortError, ProjectId, Scope, Timestamp};
use tokio::sync::broadcast;

use crate::domain::{
    Bucket, HistogramQuery, Level, LogEntry, LogId, LogLimits, LogPage, LogQuery, NewLogEntry, Source,
};
use ports::{LogFeed, LogSink, LogStats, LogStore, Submitted};

/// Adapters for every port of this context.
#[derive(Clone)]
pub struct LogPorts {
    pub sink: Arc<dyn LogSink>,
    pub store: Arc<dyn LogStore>,
    pub stats: Arc<dyn LogStats>,
    pub feed: Arc<dyn LogFeed>,
    pub clock: Arc<dyn Clock>,
}

/// What ingest did with a batch of entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ingested {
    pub accepted: u64,
    /// Entries with something cut short by the limits.
    pub truncated: u64,
    pub redactions: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("the writer is overloaded")]
    Overloaded,
    #[error(transparent)]
    Port(#[from] PortError),
}

/// All use cases of this context.
#[derive(Clone)]
pub struct LogUseCases {
    pub ingest: IngestLogs,
    pub search: SearchLogs,
    pub get: GetLog,
    pub histogram: LogHistogram,
    pub sources: ListSources,
    pub watch: WatchLogs,
}

impl LogUseCases {
    pub fn new(ports: LogPorts, limits: LogLimits) -> Self {
        let ports = Arc::new(ports);
        Self {
            ingest: IngestLogs { ports: Arc::clone(&ports), limits },
            search: SearchLogs { ports: Arc::clone(&ports) },
            get: GetLog { ports: Arc::clone(&ports) },
            histogram: LogHistogram { ports: Arc::clone(&ports) },
            sources: ListSources { ports: Arc::clone(&ports) },
            watch: WatchLogs { ports },
        }
    }
}

/// Command: keeps the entries a key sent: under its scope, within the
/// limits, redacted by the project's rules.
#[derive(Clone)]
pub struct IngestLogs {
    ports: Arc<LogPorts>,
    limits: LogLimits,
}

impl IngestLogs {
    /// With `wait`, answers once the entries are committed (forwarders
    /// retry until then); otherwise once they are queued (SDKs never retry).
    pub async fn execute(
        &self,
        scope: Scope,
        rules: &RedactionRules,
        entries: Vec<NewLogEntry>,
        wait: bool,
    ) -> Result<Ingested, IngestError> {
        let received = self.ports.clock.now();
        let latest =
            Timestamp::from_unix_seconds(received.unix_seconds() + self.limits.max_clock_skew_seconds);
        let mut ingested = Ingested::default();
        let prepared: Vec<LogEntry> = entries
            .into_iter()
            .map(|entry| {
                let mut redactor = Redactor::new(rules);
                let time = entry.time.filter(|t| *t <= latest).unwrap_or(received);
                let mut prepared = LogEntry {
                    id: None,
                    scope,
                    time,
                    received,
                    level: entry.level.unwrap_or(Level::Info),
                    message: redactor.text(&entry.message),
                    template: entry.template.map(|t| redactor.text(&t)),
                    attributes: redactor.attributes(logdock_kernel::Value::flatten(entry.attributes)),
                    trace_id: entry.trace_id,
                    span_id: entry.span_id,
                    service: entry.service,
                    host: entry.host,
                    container: entry.container,
                    release: entry.release,
                    user_id: entry.user_id,
                    redactions: 0,
                    truncated: false,
                };
                prepared.redactions = redactor.count();
                prepared.truncated = self.limits.apply(&mut prepared);
                ingested.truncated += u64::from(prepared.truncated);
                ingested.redactions += u64::from(prepared.redactions);
                prepared
            })
            .collect();
        ingested.accepted = prepared.len() as u64;
        if prepared.is_empty() {
            return Ok(ingested);
        }
        match self.ports.sink.submit(prepared, wait).await? {
            Submitted::Accepted => Ok(ingested),
            Submitted::Overloaded => Err(IngestError::Overloaded),
        }
    }
}

/// Query: a page of entries, newest first. `level:`, `service:`, `host:`
/// and `trace:` typed into the search become filters.
#[derive(Clone)]
pub struct SearchLogs {
    ports: Arc<LogPorts>,
}

impl SearchLogs {
    pub async fn execute(&self, mut query: LogQuery) -> Result<LogPage, PortError> {
        if let Some(level) = query.text.take(&["level"]).and_then(|l| Level::parse(&l)) {
            query.levels.push(level);
        }
        if let Some(service) = query.text.take(&["service"]) {
            query.services.push(service);
        }
        if let Some(host) = query.text.take(&["host"]) {
            query.hosts.push(host);
        }
        if let Some(trace) = query.text.take(&["trace", "trace_id"]) {
            query.trace_id = Some(trace);
        }
        query.limit = query.limit.clamp(1, LogQuery::MAX_LIMIT);
        self.ports.store.search(&query).await
    }
}

/// Query: one entry.
#[derive(Clone)]
pub struct GetLog {
    ports: Arc<LogPorts>,
}

impl GetLog {
    pub async fn execute(&self, id: LogId) -> Result<Option<LogEntry>, PortError> {
        self.ports.store.get(id).await
    }
}

/// Query: entries per level over time.
#[derive(Clone)]
pub struct LogHistogram {
    ports: Arc<LogPorts>,
}

impl LogHistogram {
    pub async fn execute(&self, mut query: HistogramQuery) -> Result<Vec<Bucket>, PortError> {
        if query.bucket_seconds <= 0 {
            query.bucket_seconds = HistogramQuery::bucket_for(query.from, query.to);
        }
        let buckets = self.ports.stats.histogram(&query).await?;
        // Every bucket of the range, also the empty ones, so the chart's
        // time axis is even.
        let size = query.bucket_seconds;
        let first = query.from.unix_seconds().div_euclid(size) * size;
        let last = query.to.unix_seconds();
        let mut filled = Vec::new();
        let mut found = buckets.into_iter().peekable();
        let mut start = first;
        while start < last && filled.len() < 1000 {
            let counts = match found.peek() {
                Some(bucket) if bucket.start == start => found.next().map(|b| b.counts).unwrap_or_default(),
                _ => [0; 6],
            };
            filled.push(Bucket { start, counts });
            start += size;
        }
        Ok(filled)
    }
}

/// Query: the services and hosts seen, for the filters.
#[derive(Clone)]
pub struct ListSources {
    ports: Arc<LogPorts>,
}

impl ListSources {
    pub async fn execute(&self, projects: &[ProjectId]) -> Result<Vec<Source>, PortError> {
        self.ports.stats.sources(projects).await
    }
}

/// Query: entries as they are written, for the live view.
#[derive(Clone)]
pub struct WatchLogs {
    ports: Arc<LogPorts>,
}

impl WatchLogs {
    pub fn execute(&self) -> broadcast::Receiver<Arc<LogEntry>> {
        self.ports.feed.subscribe()
    }
}
