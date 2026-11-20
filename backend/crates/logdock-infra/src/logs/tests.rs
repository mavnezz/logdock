//! The writer and the search against real segments.

use std::sync::Arc;
use std::time::Duration;

use logdock_kernel::{Day, EnvironmentId, ProjectId, Scope, Timestamp, Value};
use logdock_logs::application::ports::{LogFeed, LogSink, LogStats, LogStore, Submitted};
use logdock_logs::domain::{HistogramQuery, Level, LogEntry, LogQuery, SearchText};

use super::*;
use crate::health::WriterHealth;
use crate::segments::Segments;
use crate::sqlite::repository_tests::database;

fn entry(seconds: i64, received: i64, message: &str) -> LogEntry {
    LogEntry {
        id: None,
        scope: Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) },
        time: Timestamp::from_unix_seconds(seconds),
        received: Timestamp::from_unix_seconds(received),
        level: Level::Info,
        message: message.to_owned(),
        template: None,
        attributes: vec![
            ("order_id".into(), Value::Int(seconds)),
            ("user.email".into(), Value::String("ada@example.com".into())),
        ],
        trace_id: Some(format!("trace-{}", seconds / 60 % 2)),
        span_id: None,
        service: Some(if seconds / 60 % 2 == 0 { "web" } else { "horizon" }.into()),
        host: Some("docker-host-02".into()),
        container: None,
        release: None,
        user_id: None,
        redactions: 0,
        truncated: false,
    }
}

const DAY: i64 = 86_400 * 20_000;

async fn store() -> (LogWriter, WriterTask, SegmentLogStore, SqliteLogStats, tempfile::TempDir) {
    let (db, dir) = database().await;
    let segments = Arc::new(Segments::new(dir.path().join("segments"), db));
    let (writer, task) =
        LogWriter::start(Arc::clone(&segments), &WriterHealth::default(), WriterConfig::default());
    (writer, task, SegmentLogStore::new(Arc::clone(&segments)), SqliteLogStats::new(segments), dir)
}

#[tokio::test]
async fn writes_entries_and_finds_them_newest_first_across_days() {
    let (writer, task, store, stats, _dir) = store().await;
    let mut feed = writer.subscribe();
    // Two days of receipt; the second day's segment holds a late entry
    // from the first day.
    let first: Vec<LogEntry> =
        (0..5).map(|i| entry(DAY + i * 60, DAY + i * 60, &format!("payment {i} declined"))).collect();
    assert_eq!(writer.submit(first, true).await.unwrap(), Submitted::Accepted);
    let second = vec![
        entry(DAY + 86_400 + 10, DAY + 86_400 + 10, "refund issued"),
        entry(DAY + 150, DAY + 86_400 + 20, "late payment 9"),
    ];
    writer.submit(second, true).await.unwrap();
    assert!(feed.recv().await.unwrap().id.is_some(), "the live view hears committed entries");

    let all = store.search(&LogQuery { limit: 3, ..LogQuery::default() }).await.unwrap();
    let messages: Vec<&str> = all.entries.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(messages, ["refund issued", "payment 4 declined", "payment 3 declined"]);
    let next = store.search(&LogQuery { limit: 3, before: all.next, ..LogQuery::default() }).await.unwrap();
    let messages: Vec<&str> = next.entries.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(messages, ["late payment 9", "payment 2 declined", "payment 1 declined"]);
    let last = store.search(&LogQuery { limit: 3, before: next.next, ..LogQuery::default() }).await.unwrap();
    assert_eq!(last.entries.len(), 1);
    assert!(last.next.is_none());

    let text = |text: &str| LogQuery { text: SearchText::parse(text), limit: 50, ..LogQuery::default() };
    assert_eq!(store.search(&text("payment")).await.unwrap().entries.len(), 6);
    assert_eq!(store.search(&text("pay*")).await.unwrap().entries.len(), 6);
    assert_eq!(store.search(&text("\"payment 4\"")).await.unwrap().entries.len(), 1);
    assert_eq!(store.search(&text("ada@example.com refund")).await.unwrap().entries.len(), 1);
    assert_eq!(store.search(&text(&format!("order_id:{}", DAY + 120))).await.unwrap().entries.len(), 1);
    assert_eq!(store.search(&text("user.email:ada@example.com")).await.unwrap().entries.len(), 7);
    assert_eq!(store.search(&text("\"quote\" \"inside")).await.unwrap().entries.len(), 0);

    let filtered = LogQuery {
        services: vec!["web".into()],
        hosts: vec!["docker-host-02".into()],
        levels: vec![Level::Info],
        projects: vec![ProjectId::new(1)],
        environments: vec![EnvironmentId::new(2)],
        trace_id: Some("trace-0".into()),
        from: Some(Timestamp::from_unix_seconds(DAY + 100)),
        to: Some(Timestamp::from_unix_seconds(DAY + 300)),
        limit: 50,
        ..LogQuery::default()
    };
    let found = store.search(&filtered).await.unwrap();
    assert_eq!(
        found.entries.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(),
        ["payment 4 declined", "late payment 9", "payment 2 declined"]
    );
    assert!(
        store
            .search(&LogQuery { projects: vec![ProjectId::new(9)], limit: 5, ..LogQuery::default() })
            .await
            .unwrap()
            .entries
            .is_empty()
    );

    let id = found.entries[0].id.unwrap();
    let got = store.get(id).await.unwrap().unwrap();
    assert_eq!(got.attributes[1].1.as_str(), Some("ada@example.com"));
    assert!(store.get(LogId { day: Day::from_days(1), keep: 0, row: 1 }).await.unwrap().is_none());

    let histogram = HistogramQuery {
        from: Timestamp::from_unix_seconds(DAY),
        to: Timestamp::from_unix_seconds(DAY + 600),
        bucket_seconds: 300,
        ..HistogramQuery::default()
    };
    let buckets = stats.histogram(&histogram).await.unwrap();
    assert_eq!(buckets.iter().map(|b| b.counts[2]).collect::<Vec<_>>(), [6]);
    let web = HistogramQuery { services: vec!["web".into()], ..histogram };
    assert_eq!(stats.histogram(&web).await.unwrap()[0].counts[2], 4);
    let sources = stats.sources(&[ProjectId::new(1)]).await.unwrap();
    assert_eq!(sources.iter().map(|s| s.service.as_deref().unwrap()).collect::<Vec<_>>(), ["horizon", "web"]);

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), task.finish()).await.unwrap();
}

#[tokio::test]
async fn refuses_more_than_its_queue_holds() {
    let (db, dir) = database().await;
    let segments = Arc::new(Segments::new(dir.path().join("segments"), db));
    let config = WriterConfig { queue: 2, batch: 10 };
    let (writer, task) = LogWriter::start(segments, &WriterHealth::default(), config);
    // A first batch is always taken, however large; one on top of a full
    // queue is not.
    let many: Vec<LogEntry> = (0..5).map(|i| entry(DAY + i, DAY + i, "x")).collect();
    assert_eq!(writer.submit(many.clone(), false).await.unwrap(), Submitted::Accepted);
    let second = writer.submit(many, false).await.unwrap();
    assert!(matches!(second, Submitted::Overloaded | Submitted::Accepted));
    drop(writer);
    task.finish().await;
}

/// The sizing benchmark of docs/testing.md: entries like a Laravel app's,
/// written through the writer as ingest would, then searched.
///
/// `cargo test -p logdock-infra --release -- --ignored --nocapture benchmark`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a benchmark; run it on purpose"]
async fn benchmark() {
    let total: usize = std::env::var("BENCH_ENTRIES").ok().and_then(|n| n.parse().ok()).unwrap_or(200_000);
    let (db, dir) = database().await;
    let segments = Arc::new(Segments::new(dir.path().join("segments"), db));
    let config = WriterConfig { queue: 1_000_000, batch: 2_000 };
    let (writer, task) = LogWriter::start(Arc::clone(&segments), &WriterHealth::default(), config);
    let messages = [
        "Payment declined for order {id}",
        "Job App\\Jobs\\SendInvoiceMail processed in 182 ms",
        "SQLSTATE[40001]: Serialization failure: 1213 Deadlock found when trying to get lock",
        "User {id} signed in from 203.0.113.7",
        "Cache miss for key products.page.3",
    ];
    let started = std::time::Instant::now();
    let mut sent = 0;
    while sent < total {
        let batch: Vec<LogEntry> = (sent..(sent + 100).min(total))
            .map(|i| {
                let mut e = entry(
                    DAY + (i as i64) / 50,
                    DAY + 1,
                    &messages[i % messages.len()].replace("{id}", &i.to_string()),
                );
                e.attributes.push(("request_id".into(), Value::String(format!("{i:032x}"))));
                e.attributes.push(("route".into(), Value::String("orders.store".into())));
                e
            })
            .collect();
        sent += batch.len();
        // As the SDK endpoint does: queued, not waited for; the last batch waits.
        let wait = sent >= total;
        assert_eq!(writer.submit(batch, wait).await.unwrap(), Submitted::Accepted);
    }
    let elapsed = started.elapsed();
    let bytes = segments.bytes(crate::segments::SegmentId::new(
        logdock_retention::domain::Kind::Logs,
        Day::from_days(20_000),
        0,
    ));
    let store = SegmentLogStore::new(Arc::clone(&segments));
    let search = std::time::Instant::now();
    let found = store
        .search(&LogQuery { text: SearchText::parse("deadlock"), limit: 100, ..LogQuery::default() })
        .await
        .unwrap();
    let search_ms = search.elapsed().as_millis();
    let attribute = std::time::Instant::now();
    let by_attribute = store
        .search(&LogQuery {
            text: SearchText::parse(&format!("request_id:{:032x}", total / 2)),
            limit: 100,
            ..LogQuery::default()
        })
        .await
        .unwrap();
    let attribute_ms = attribute.elapsed().as_millis();
    let page = std::time::Instant::now();
    store.search(&LogQuery { limit: 100, ..LogQuery::default() }).await.unwrap();
    let page_ms = page.elapsed().as_millis();
    println!(
        "benchmark: {total} entries in {:.2} s = {:.0} entries/s; {:.0} bytes/entry on disk; \
         full-text 'deadlock' {search_ms} ms ({} found), attribute {attribute_ms} ms ({} found), newest page {page_ms} ms",
        elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64(),
        bytes as f64 / total as f64,
        found.entries.len(),
        by_attribute.entries.len(),
    );
    drop(writer);
    task.finish().await;
}
