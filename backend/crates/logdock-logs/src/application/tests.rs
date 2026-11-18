use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::redaction::RedactionRules;
use logdock_kernel::{Clock, Day, EnvironmentId, PortError, ProjectId, Scope, Timestamp, Value};
use tokio::sync::broadcast;

use super::*;
use crate::domain::{Cursor, SearchText};

struct Fixed;

impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_seconds(1_000_000)
    }
}

#[derive(Default)]
struct Fake {
    written: Mutex<Vec<(Vec<LogEntry>, bool)>>,
    queries: Mutex<Vec<LogQuery>>,
    overloaded: bool,
    sender: Option<broadcast::Sender<Arc<LogEntry>>>,
}

#[async_trait]
impl LogSink for Fake {
    async fn submit(&self, entries: Vec<LogEntry>, wait: bool) -> Result<Submitted, PortError> {
        if self.overloaded {
            return Ok(Submitted::Overloaded);
        }
        self.written.lock().unwrap().push((entries, wait));
        Ok(Submitted::Accepted)
    }
}

#[async_trait]
impl LogStore for Fake {
    async fn search(&self, query: &LogQuery) -> Result<LogPage, PortError> {
        self.queries.lock().unwrap().push(query.clone());
        Ok(LogPage::default())
    }

    async fn get(&self, _id: LogId) -> Result<Option<LogEntry>, PortError> {
        Ok(None)
    }
}

#[async_trait]
impl LogStats for Fake {
    async fn histogram(&self, query: &HistogramQuery) -> Result<Vec<Bucket>, PortError> {
        let start = query.from.unix_seconds().div_euclid(query.bucket_seconds) * query.bucket_seconds;
        Ok(vec![Bucket { start: start + query.bucket_seconds, counts: [0, 0, 3, 0, 1, 0] }])
    }

    async fn sources(&self, _projects: &[ProjectId]) -> Result<Vec<Source>, PortError> {
        Ok(Vec::new())
    }
}

impl LogFeed for Fake {
    fn subscribe(&self) -> broadcast::Receiver<Arc<LogEntry>> {
        self.sender.as_ref().unwrap().subscribe()
    }
}

fn use_cases(fake: Arc<Fake>) -> LogUseCases {
    let ports = LogPorts {
        sink: fake.clone(),
        store: fake.clone(),
        stats: fake.clone(),
        feed: fake,
        clock: Arc::new(Fixed),
    };
    LogUseCases::new(ports, LogLimits::default())
}

fn scope() -> Scope {
    Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) }
}

#[tokio::test]
async fn ingest_keeps_entries_under_the_keys_scope_redacted_and_limited() {
    let fake = Arc::new(Fake::default());
    let logs = use_cases(fake.clone());
    let entries = vec![
        NewLogEntry {
            time: Some(Timestamp::from_unix_seconds(999_990)),
            level: Some(Level::Warn),
            message: "login with password=hunter2".into(),
            attributes: vec![
                ("token".into(), Value::String("abc".into())),
                ("user".into(), Value::Object(vec![("id".into(), Value::Int(7))])),
            ],
            ..NewLogEntry::default()
        },
        // From the future: gets the time of receipt.
        NewLogEntry {
            time: Some(Timestamp::from_unix_seconds(2_000_000)),
            message: "x".repeat(20_000),
            ..NewLogEntry::default()
        },
    ];
    let ingested = logs.ingest.execute(scope(), &RedactionRules::default(), entries, true).await.unwrap();
    assert_eq!(ingested, Ingested { accepted: 2, truncated: 1, redactions: 2 });
    let written = fake.written.lock().unwrap();
    let (entries, wait) = &written[0];
    assert!(*wait);
    assert_eq!(entries[0].scope, scope());
    assert_eq!(entries[0].message, "login with password=[redacted]");
    assert_eq!(entries[0].attributes[0].1.as_str(), Some("[redacted]"));
    assert_eq!(entries[0].attributes[1].0, "user.id", "flattened");
    assert_eq!(entries[0].redactions, 2);
    assert_eq!(entries[1].time, Timestamp::from_unix_seconds(1_000_000));
    assert_eq!(entries[1].level, Level::Info);
    assert!(entries[1].truncated);
}

#[tokio::test]
async fn ingest_says_when_the_writer_is_overloaded() {
    let logs = use_cases(Arc::new(Fake { overloaded: true, ..Fake::default() }));
    let entries = vec![NewLogEntry { message: "x".into(), ..NewLogEntry::default() }];
    let result = logs.ingest.execute(scope(), &RedactionRules::default(), entries, false).await;
    assert!(matches!(result, Err(IngestError::Overloaded)));
    let none = logs.ingest.execute(scope(), &RedactionRules::default(), Vec::new(), false).await.unwrap();
    assert_eq!(none.accepted, 0);
}

#[tokio::test]
async fn search_turns_typed_filters_into_filters_and_bounds_the_page() {
    let fake = Arc::new(Fake::default());
    let logs = use_cases(fake.clone());
    let query = LogQuery {
        text: SearchText::parse("level:error service:horizon host:h1 trace:abc payment"),
        limit: 10_000,
        before: Cursor::parse("1_1-1"),
        ..LogQuery::default()
    };
    logs.search.execute(query).await.unwrap();
    let seen = fake.queries.lock().unwrap()[0].clone();
    assert_eq!(seen.levels, [Level::Error]);
    assert_eq!(seen.services, ["horizon"]);
    assert_eq!(seen.hosts, ["h1"]);
    assert_eq!(seen.trace_id.as_deref(), Some("abc"));
    assert_eq!(seen.text.terms.len(), 1);
    assert_eq!(seen.limit, LogQuery::MAX_LIMIT);
    assert!(logs.get.execute(LogId { day: Day::from_days(1), row: 1 }).await.unwrap().is_none());
    assert!(logs.sources.execute(&[]).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_histogram_has_every_bucket_of_the_range() {
    let logs = use_cases(Arc::new(Fake::default()));
    let query = HistogramQuery {
        from: Timestamp::from_unix_seconds(0),
        to: Timestamp::from_unix_seconds(3600),
        ..HistogramQuery::default()
    };
    let buckets = logs.histogram.execute(query).await.unwrap();
    assert_eq!(buckets.len(), 60);
    assert_eq!(buckets[1].counts, [0, 0, 3, 0, 1, 0]);
    assert_eq!(buckets[0].counts, [0; 6]);
    assert_eq!(buckets[59].start, 3540);
}

#[tokio::test]
async fn the_live_view_hears_what_is_written() {
    let (sender, _) = broadcast::channel(4);
    let logs = use_cases(Arc::new(Fake { sender: Some(sender.clone()), ..Fake::default() }));
    let mut receiver = logs.watch.execute();
    let entry = LogEntry {
        id: None,
        scope: scope(),
        time: Timestamp::from_unix_seconds(1),
        received: Timestamp::from_unix_seconds(1),
        level: Level::Info,
        message: "hi".into(),
        template: None,
        attributes: Vec::new(),
        trace_id: None,
        span_id: None,
        service: None,
        host: None,
        container: None,
        release: None,
        user_id: None,
        redactions: 0,
        truncated: false,
    };
    sender.send(Arc::new(entry)).unwrap();
    assert_eq!(receiver.recv().await.unwrap().message, "hi");
}
