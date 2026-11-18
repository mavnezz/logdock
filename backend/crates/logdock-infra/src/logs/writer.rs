//! The one task that writes log entries: it takes what queued up since its
//! last commit, writes it to today's segment in one transaction, adds the
//! counts to the main database, and tells the live view.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use logdock_kernel::{Day, PortError, Timestamp};
use logdock_logs::application::ports::{LogFeed, LogSink, Submitted};
use logdock_logs::domain::{LogEntry, LogId};
use logdock_retention::domain::Kind;
use sqlx::SqliteConnection;
use sqlx::sqlite::SqliteConnection as _Connection;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::health::{WriterHandle, WriterHealth};
use crate::json::fields_to_json;
use crate::segments::{Segments, port};

#[derive(Debug, Clone, Copy)]
pub struct WriterConfig {
    /// Entries waiting at most; beyond, ingest is told to come back later.
    pub queue: usize,
    /// Entries in one transaction at most.
    pub batch: usize,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self { queue: 100_000, batch: 2_000 }
    }
}

struct Job {
    entries: Vec<LogEntry>,
    done: Option<oneshot::Sender<Result<(), String>>>,
}

/// [`LogSink`] and [`LogFeed`]: the handle to the writer task. Dropping the
/// last handle lets the task finish what is queued and end.
pub struct LogWriter {
    sender: mpsc::UnboundedSender<Job>,
    pending: Arc<AtomicUsize>,
    queue: usize,
    feed: broadcast::Sender<Arc<LogEntry>>,
}

/// The running writer task. [`WriterTask::finish`] lets it write what is
/// queued and end.
pub struct WriterTask {
    stop: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl WriterTask {
    pub async fn finish(self) {
        let _ = self.stop.send(true);
        if let Err(error) = self.handle.await {
            tracing::error!(%error, "the log writer failed");
        }
    }
}

impl LogWriter {
    pub fn start(segments: Arc<Segments>, health: &WriterHealth, config: WriterConfig) -> (Self, WriterTask) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let (feed, _) = broadcast::channel(1024);
        let (stop, stopped) = watch::channel(false);
        let task = Task {
            segments,
            health: health.register("logs"),
            receiver,
            stopped,
            pending: Arc::clone(&pending),
            feed: feed.clone(),
            batch: config.batch.max(1),
            current: None,
        };
        let handle = tokio::spawn(task.run());
        (Self { sender, pending, queue: config.queue, feed }, WriterTask { stop, handle })
    }
}

#[async_trait]
impl LogSink for LogWriter {
    async fn submit(&self, entries: Vec<LogEntry>, wait: bool) -> Result<Submitted, PortError> {
        let count = entries.len();
        let before = self.pending.fetch_add(count, Ordering::SeqCst);
        if before + count > self.queue && before > 0 {
            self.pending.fetch_sub(count, Ordering::SeqCst);
            return Ok(Submitted::Overloaded);
        }
        let (done, committed) = if wait {
            let (sender, receiver) = oneshot::channel();
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        if self.sender.send(Job { entries, done }).is_err() {
            self.pending.fetch_sub(count, Ordering::SeqCst);
            return Err(port("the log writer has stopped"));
        }
        if let Some(committed) = committed {
            committed.await.map_err(port)?.map_err(port)?;
        }
        Ok(Submitted::Accepted)
    }
}

impl LogFeed for LogWriter {
    fn subscribe(&self) -> broadcast::Receiver<Arc<LogEntry>> {
        self.feed.subscribe()
    }
}

struct Task {
    segments: Arc<Segments>,
    health: WriterHandle,
    receiver: mpsc::UnboundedReceiver<Job>,
    stopped: watch::Receiver<bool>,
    pending: Arc<AtomicUsize>,
    feed: broadcast::Sender<Arc<LogEntry>>,
    batch: usize,
    /// Today's segment and the connection to it.
    current: Option<(Day, SqliteConnection)>,
}

impl Task {
    /// The next job: waits for one until told to stop, then takes only
    /// what is queued.
    async fn next(&mut self) -> Option<Job> {
        if *self.stopped.borrow() {
            return self.receiver.try_recv().ok();
        }
        tokio::select! {
            job = self.receiver.recv() => job,
            _ = self.stopped.changed() => self.receiver.try_recv().ok(),
        }
    }

    async fn run(mut self) {
        while let Some(first) = self.next().await {
            self.health.waiting();
            let mut jobs = vec![first];
            let mut count = jobs[0].entries.len();
            while count < self.batch {
                match self.receiver.try_recv() {
                    Ok(job) => {
                        count += job.entries.len();
                        jobs.push(job);
                    }
                    Err(_) => break,
                }
            }
            let mut entries: Vec<LogEntry> = Vec::with_capacity(count);
            let mut done = Vec::new();
            for job in jobs {
                entries.extend(job.entries);
                done.extend(job.done);
            }
            let result = self.write(&mut entries).await;
            self.pending.fetch_sub(count, Ordering::SeqCst);
            self.health.committed();
            match result {
                Ok(()) => {
                    for sender in done {
                        let _ = sender.send(Ok(()));
                    }
                    for entry in entries {
                        let _ = self.feed.send(Arc::new(entry));
                    }
                }
                Err(error) => {
                    tracing::error!(%error, entries = count, "cannot write log entries");
                    for sender in done {
                        let _ = sender.send(Err(error.to_string()));
                    }
                }
            }
        }
        if let Some((_, connection)) = self.current.take() {
            use sqlx::Connection as _;
            let _ = connection.close().await;
        }
    }

    async fn connection(&mut self, day: Day) -> Result<&mut SqliteConnection, PortError> {
        if self.current.as_ref().is_none_or(|(current, _)| *current != day) {
            if let Some((old, connection)) = self.current.take() {
                use sqlx::Connection as _;
                let _ = connection.close().await;
                // The finished day's segment is closed in the background.
                let segments = Arc::clone(&self.segments);
                if old < day {
                    tokio::spawn(async move {
                        if let Err(error) = segments.close(Kind::Logs, old).await {
                            tracing::warn!(%error, day = %old, "cannot close the log segment");
                        }
                    });
                }
            }
            let connection = self.segments.open_writer(Kind::Logs, day).await?;
            self.current = Some((day, connection));
        }
        Ok(&mut self.current.as_mut().expect("opened above").1)
    }

    async fn write(&mut self, entries: &mut [LogEntry]) -> Result<(), PortError> {
        // By day of receipt: a batch at midnight goes into two segments.
        let mut days: BTreeMap<Day, Vec<usize>> = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            days.entry(entry.received.day()).or_default().push(index);
        }
        for (day, indices) in days {
            let connection = self.connection(day).await?;
            write_segment(connection, day, entries, &indices).await?;
            let times = indices.iter().map(|i| entries[*i].time);
            let (min, max) = times.fold(
                (Timestamp::from_unix_micros(i64::MAX), Timestamp::from_unix_micros(i64::MIN)),
                |(lo, hi), t| (lo.min(t), hi.max(t)),
            );
            self.segments.note_batch(Kind::Logs, day, indices.len() as i64, min, max).await?;
        }
        write_counts(&self.segments, entries).await
    }
}

async fn write_segment(
    connection: &mut _Connection,
    day: Day,
    entries: &mut [LogEntry],
    indices: &[usize],
) -> Result<(), PortError> {
    use sqlx::Connection as _;
    let mut tx = connection.begin().await.map_err(port)?;
    for index in indices {
        let entry = &mut entries[*index];
        let attributes = fields_to_json(&entry.attributes);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO logs (time, received, project_id, environment_id, level, service, host, container, release, \
             message, template, trace_id, span_id, user_id, attributes, redactions, truncated) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(entry.time.unix_micros())
        .bind(entry.received.unix_micros())
        .bind(entry.scope.project.get())
        .bind(entry.scope.environment.get())
        .bind(entry.level.number())
        .bind(&entry.service)
        .bind(&entry.host)
        .bind(&entry.container)
        .bind(&entry.release)
        .bind(&entry.message)
        .bind(&entry.template)
        .bind(&entry.trace_id)
        .bind(&entry.span_id)
        .bind(&entry.user_id)
        .bind(&attributes)
        .bind(i64::from(entry.redactions))
        .bind(entry.truncated)
        .fetch_one(&mut *tx)
        .await
        .map_err(port)?;
        sqlx::query("INSERT INTO logs_fts (rowid, message, attributes) VALUES (?, ?, ?)")
            .bind(id)
            .bind(&entry.message)
            .bind(&attributes)
            .execute(&mut *tx)
            .await
            .map_err(port)?;
        entry.id = Some(LogId { day, row: id });
    }
    tx.commit().await.map_err(port)
}

/// Adds the batch to the counts per minute and to the sources seen.
async fn write_counts(segments: &Segments, entries: &[LogEntry]) -> Result<(), PortError> {
    let mut counts: HashMap<(i64, i64, &str, i64, i64), i64> = HashMap::new();
    let mut sources: HashMap<(i64, i64, &str, &str), i64> = HashMap::new();
    for entry in entries {
        let service = entry.service.as_deref().unwrap_or("");
        let key = (
            entry.scope.project.get(),
            entry.scope.environment.get(),
            service,
            entry.level.number(),
            entry.time.minute(),
        );
        *counts.entry(key).or_default() += 1;
        let source = (
            entry.scope.project.get(),
            entry.scope.environment.get(),
            service,
            entry.host.as_deref().unwrap_or(""),
        );
        let seen = sources.entry(source).or_insert(i64::MIN);
        *seen = (*seen).max(entry.received.unix_micros());
    }
    let mut tx = segments.db().pool().begin().await.map_err(port)?;
    for ((project, environment, service, level, minute), count) in counts {
        sqlx::query(
            "INSERT INTO log_rollups (project_id, environment_id, service, level, minute, count) VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (project_id, environment_id, service, level, minute) DO UPDATE SET count = count + excluded.count",
        )
        .bind(project)
        .bind(environment)
        .bind(service)
        .bind(level)
        .bind(minute)
        .bind(count)
        .execute(&mut *tx)
        .await
        .map_err(port)?;
    }
    for ((project, environment, service, host), seen) in sources {
        sqlx::query(
            "INSERT INTO log_sources (project_id, environment_id, service, host, last_seen) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (project_id, environment_id, service, host) DO UPDATE SET last_seen = max(last_seen, excluded.last_seen)",
        )
        .bind(project)
        .bind(environment)
        .bind(service)
        .bind(host)
        .bind(seen)
        .execute(&mut *tx)
        .await
        .map_err(port)?;
    }
    tx.commit().await.map_err(port)
}
