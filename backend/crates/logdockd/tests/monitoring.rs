//! Feature tests: transactions and check-ins as sentry-laravel sent them
//! from the test app, through the real router into rollups, segments and
//! monitors.

mod common;

use std::io::Write;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{Browser, TestApp, project_with_key};
use serde_json::Value;

const FIXTURES: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../logdock-sentry/tests/fixtures/sentry-laravel-4.30.0");

/// The day the fixtures were captured.
const DAY: &str = "from=2026-11-19T00:00:00Z&to=2026-11-20T00:00:00Z";

fn envelope(project: i64, key: &str, name: &str) -> Request<Body> {
    let body = std::fs::read(format!("{FIXTURES}/{name}")).unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&body).unwrap();
    Request::builder()
        .method(Method::POST)
        .uri(format!("/api/{project}/envelope/"))
        .header(header::CONTENT_ENCODING, "gzip")
        .header("x-sentry-auth", format!("Sentry sentry_version=7, sentry_key={key}"))
        .body(Body::from(encoder.finish().unwrap()))
        .unwrap()
}

async fn send(app: &TestApp, project: i64, key: &str, names: &[&str]) {
    for name in names {
        let answer = app.request(envelope(project, key, name)).await;
        assert_eq!(answer.status, StatusCode::OK, "{name}: {}", answer.text());
    }
}

fn named<'a>(rows: &'a Value, name: &str) -> &'a Value {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {rows}"))
}

#[tokio::test]
async fn requests_jobs_and_what_they_spent_their_time_on() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    send(&app, project, &key, &["transaction-request.envelope", "transaction-job.envelope"]).await;

    let requests = admin.get(&format!("/api/v1/performance?kind=request&{DAY}")).await.json();
    let work = named(&requests, "GET /scenario/work");
    assert_eq!(work["count"], 1);
    assert_eq!(work["flagged"], 0);
    assert!(work["p95_ms"].as_f64().unwrap() > 50.0, "{work}");

    let jobs = admin.get(&format!("/api/v1/performance?kind=job&{DAY}")).await.json();
    assert_eq!(named(&jobs, "App\\Jobs\\ChargeOrder")["flagged"], 1, "the job failed");

    // The job's queue: it ran once, failed, and waited half a second.
    let queues = admin.get(&format!("/api/v1/queues?{DAY}")).await.json();
    let default = named(&queues, "default");
    assert_eq!((default["processed"].as_u64(), default["failed"].as_u64()), (Some(1), Some(1)), "{queues}");
    assert_eq!(default["retries"], 0);
    let waited = default["wait"]["average_ms"].as_f64().unwrap();
    assert!((529.0..531.0).contains(&waited), "{default}");
    assert!(default["processing"]["average_ms"].as_f64().unwrap() > 0.0);

    let queries = admin.get(&format!("/api/v1/performance?kind=query&sort=count&{DAY}")).await.json();
    named(&queries, "select ? as id, ? as total");
    named(&queries, "delete from \"jobs\" where \"id\" = ?");
    let outgoing = admin.get(&format!("/api/v1/performance?kind=outgoing&{DAY}")).await.json();
    assert_eq!(named(&outgoing, "GET logdock:8080")["count"], 1);
    let cache = admin.get(&format!("/api/v1/performance?kind=cache&{DAY}")).await.json();
    assert_eq!(named(&cache, "get exchange-rate")["flagged"], 1, "a miss");
    assert_eq!(named(&cache, "put exchange-rate")["flagged"], 0);

    let series = admin
        .get(&format!("/api/v1/performance/series?kind=request&name=GET%20/scenario/work&{DAY}"))
        .await
        .json();
    assert_eq!(series["bucket_seconds"], 1800);
    assert_eq!(series["total"]["count"], 1);
    assert_eq!(series["buckets"].as_array().unwrap().len(), 48);

    let samples = admin
        .get(&format!("/api/v1/performance/samples?kind=request&name=GET%20/scenario/work&{DAY}"))
        .await
        .json();
    let trace_id = samples[0]["trace_id"].as_str().unwrap();
    assert_eq!(samples[0]["http_status"], 200);
    let trace = admin.get(&format!("/api/v1/traces/{trace_id}")).await.json();
    let transaction = &trace["transactions"][0];
    assert_eq!(transaction["name"], "GET /scenario/work");
    assert_eq!(transaction["service"], "web");
    let spans: Vec<&str> =
        transaction["spans"].as_array().unwrap().iter().filter_map(|s| s["op"].as_str()).collect();
    assert!(spans.contains(&"http.client") && spans.contains(&"db.sql.query"), "{spans:?}");

    let bad = admin.get(&format!("/api/v1/performance?kind=nonsense&{DAY}")).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let backwards =
        admin.get("/api/v1/performance?kind=request&from=2026-11-20T00:00:00Z&to=2026-11-19T00:00:00Z").await;
    assert_eq!(backwards.status, StatusCode::BAD_REQUEST);
    let no_samples = admin.get(&format!("/api/v1/performance/samples?kind=query&name=x&{DAY}")).await;
    assert_eq!(no_samples.status, StatusCode::BAD_REQUEST);
    app.stop().await;
}

#[tokio::test]
async fn a_failing_scheduled_task_becomes_a_failing_monitor() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    send(&app, project, &key, &["check-in-started.envelope", "check-in-failed.envelope"]).await;

    let monitors = admin.get("/api/v1/monitors").await.json();
    let monitor = &monitors[0];
    assert_eq!(monitor["name"], "nightly-report");
    assert_eq!(monitor["health"], "failing");
    assert_eq!(monitor["schedule"], "* * * * *");
    assert_eq!(monitor["timezone"], "UTC");
    assert!(monitor["next_run"].is_string());
    let id = monitor["id"].as_i64().unwrap();
    let detail = admin.get(&format!("/api/v1/monitors/{id}")).await.json();
    let runs = detail["check_ins"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "start and end are one run");
    assert_eq!(runs[0]["status"], "error");
    assert_eq!(runs[0]["release"], "dev-1");
    assert!(runs[0]["duration_ms"].as_f64().is_some());

    let other = admin.get(&format!("/api/v1/monitors?project={}", project + 1)).await.json();
    assert!(other.as_array().unwrap().is_empty());
    assert_eq!(admin.delete(&format!("/api/v1/monitors/{id}")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(admin.get(&format!("/api/v1/monitors/{id}")).await.status, StatusCode::NOT_FOUND);
    let counts = admin.get(&format!("/api/v1/projects/{project}/ingest")).await.json();
    assert_eq!(counts[0]["accepted"], 2);
    assert_eq!(counts[0]["skipped"], 0);
    app.stop().await;
}
