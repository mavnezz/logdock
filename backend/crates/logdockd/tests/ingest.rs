//! Feature tests: what SDKs and forwarders send, through the real router
//! into real segments, and found again.

mod common;

use std::io::Write;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{Browser, TestApp, eventually, project_with_key};
use serde_json::json;

/// A `log` item as sentry-php 4.x writes it (Serializer/EnvelopItems/LogsItem.php).
fn log_envelope(environment: &str, body: &str) -> String {
    let item = json!({"items": [{
        "timestamp": 1_794_993_302.123_456,
        "trace_id": "5b8efff798038103d269b633813fc60c",
        "level": "warn",
        "body": body,
        "attributes": {
            "sentry.release": {"type": "string", "value": "4f2c1e9"},
            "sentry.environment": {"type": "string", "value": environment},
            "server.address": {"type": "string", "value": "orders-horizon-1"},
            "service": {"type": "string", "value": "horizon"},
            "host": {"type": "string", "value": "docker-host-02"},
            "order_id": {"type": "integer", "value": 42},
            "password": {"type": "string", "value": "hunter2"}
        }
    }]});
    format!(
        "{}\n{}\n{}\n",
        json!({"event_id": "9ec79c33ec9942ab8353589fcb2e04dc", "sent_at": "2026-11-18T09:15:03Z", "dsn": "https://x@logdock.test/1"}),
        json!({"type": "log", "item_count": 1, "content_type": "application/vnd.sentry.items.log+json"}),
        item
    )
}

fn gzip(text: &str) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

fn envelope_request(project: i64, key: &str, body: Vec<u8>, gzipped: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/{project}/envelope/"))
        .header(header::CONTENT_TYPE, "application/x-sentry-envelope")
        .header(
            "x-sentry-auth",
            format!("Sentry sentry_version=7, sentry_client=sentry.php/4.37.1, sentry_key={key}"),
        );
    if gzipped {
        request = request.header(header::CONTENT_ENCODING, "gzip");
    }
    request.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn sentry_logs_arrive_redacted_under_the_keys_scope() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, production, key) = project_with_key(&admin, "orders").await;

    let body = gzip(&log_envelope("production", "Payment declined for order 42, password=hunter2"));
    let answer = app.request(envelope_request(project, &key, body, true)).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(answer.json()["id"], "9ec79c33ec9942ab8353589fcb2e04dc");

    let page = eventually(&admin, "/api/v1/logs?q=payment", |page| {
        page["entries"].as_array().is_some_and(|e| !e.is_empty())
    })
    .await;
    let entry = &page["entries"][0];
    assert_eq!(entry["project_id"], project);
    assert_eq!(entry["environment_id"], production);
    assert_eq!(entry["level"], "warn");
    assert_eq!(entry["service"], "horizon");
    assert_eq!(entry["host"], "docker-host-02");
    assert_eq!(entry["container"], "orders-horizon-1");
    assert_eq!(entry["release"], "4f2c1e9");
    assert_eq!(entry["trace_id"], "5b8efff798038103d269b633813fc60c");
    assert_eq!(entry["time"], "2026-11-18T09:15:02.123456Z");
    assert_eq!(entry["message"], "Payment declined for order 42, password=[redacted]");
    assert_eq!(entry["attributes"]["password"], "[redacted]");
    assert_eq!(entry["attributes"]["order_id"], 42);
    assert!(entry["attributes"].get("sentry.release").is_none(), "lifted into its field");

    let one = admin.get(&format!("/api/v1/logs/{}", entry["id"].as_str().unwrap())).await;
    assert_eq!(one.json()["message"], entry["message"]);
    let by_attribute = admin.get("/api/v1/logs?q=order_id:42&service=horizon&level=warn,error").await.json();
    assert_eq!(by_attribute["entries"].as_array().unwrap().len(), 1);
    let by_trace = admin.get("/api/v1/logs?trace=5b8efff798038103d269b633813fc60c").await.json();
    assert_eq!(by_trace["entries"].as_array().unwrap().len(), 1);
    let none = admin.get(&format!("/api/v1/logs?project={}", project + 1)).await.json();
    assert!(none["entries"].as_array().unwrap().is_empty());

    let histogram =
        admin.get("/api/v1/logs/histogram?from=2026-11-18T09:00:00Z&to=2026-11-18T10:00:00Z").await.json();
    let warn: u64 =
        histogram["buckets"].as_array().unwrap().iter().map(|b| b["warn"].as_u64().unwrap()).sum();
    assert_eq!(warn, 1);
    let sources = admin.get(&format!("/api/v1/logs/sources?project={project}")).await.json();
    assert_eq!(sources[0]["service"], "horizon");

    let counts = admin.get(&format!("/api/v1/projects/{project}/ingest")).await.json();
    assert_eq!(counts[0]["accepted"], 1);
    app.stop().await;
}

#[tokio::test]
async fn refuses_wrong_keys_stages_and_broken_envelopes() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    let (other, _, other_key) = project_with_key(&admin, "billing").await;

    let body = || log_envelope("production", "x").into_bytes();
    assert_eq!(
        app.request(envelope_request(project, "nonsense", body(), false)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.request(envelope_request(project, &other_key, body(), false)).await.status,
        StatusCode::FORBIDDEN
    );
    let unsigned = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/{other}/envelope/"))
        .body(Body::from("{}\n"))
        .unwrap();
    assert_eq!(app.request(unsigned).await.status, StatusCode::FORBIDDEN);
    let broken = app.request(envelope_request(project, &key, b"not an envelope".to_vec(), false)).await;
    assert_eq!(broken.status, StatusCode::BAD_REQUEST);
    let staging =
        app.request(envelope_request(project, &key, log_envelope("staging", "x").into_bytes(), false)).await;
    assert_eq!(staging.status, StatusCode::OK, "the envelope is taken, its entries are refused");
    // Other item types are skipped, not refused.
    let event = format!("{{}}\n{}\n{{}}\n", json!({"type": "client_report"}));
    assert_eq!(
        app.request(envelope_request(project, &key, event.into_bytes(), false)).await.status,
        StatusCode::OK
    );

    let counts = admin.get(&format!("/api/v1/projects/{project}/ingest")).await.json();
    let mut rejected = std::collections::BTreeMap::new();
    for count in counts.as_array().unwrap() {
        for (reason, n) in count["rejected"].as_object().unwrap() {
            *rejected.entry(reason.clone()).or_insert(0) += n.as_u64().unwrap();
        }
    }
    assert_eq!(rejected.get("auth"), Some(&2), "{counts}");
    assert_eq!(rejected.get("invalid"), Some(&1));
    assert_eq!(rejected.get("environment"), Some(&1));
    let skipped: u64 = counts.as_array().unwrap().iter().map(|c| c["skipped"].as_u64().unwrap()).sum();
    assert_eq!(skipped, 1);
    app.stop().await;
}

#[tokio::test]
async fn a_key_of_the_project_takes_every_stage_its_data_names() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let created = admin
        .post("/api/v1/projects", json!({"name": "orders", "environments": ["production"]}))
        .await
        .json();
    let project = created["id"].as_i64().unwrap();
    let new_key = admin.post(&format!("/api/v1/projects/{project}/keys"), json!({"label": "laravel"})).await;
    assert_eq!(new_key.status, StatusCode::CREATED, "{}", new_key.text());
    assert_eq!(new_key.json()["key"]["environment_id"], json!(null));
    let key = new_key.json()["secret"].as_str().unwrap().to_owned();

    // As Sentry's DSNs: one key, the stage from the data, added when new.
    for (stage, body) in [("qa", "from qa"), ("QA", "from QA"), ("production", "from production")] {
        let sent =
            app.request(envelope_request(project, &key, log_envelope(stage, body).into_bytes(), false)).await;
        assert_eq!(sent.status, StatusCode::OK, "{}", sent.text());
    }
    let lines = [r#"{"message":"names no stage"}"#, r#"{"message":"from dev","environment":"dev"}"#];
    assert_eq!(app.request(ndjson(&key, &lines.join("\n"))).await.json()["accepted"], 2);
    let bad = app.request(ndjson(&key, r#"{"message":"bad","environment":"feature/login"}"#)).await;
    assert_eq!(bad.json(), json!({"accepted": 0, "rejected": 1}));

    let stages = admin.get(&format!("/api/v1/projects/{project}")).await.json()["environments"].clone();
    let id_of = |name: &str| {
        stages.as_array().unwrap().iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("{stages}"))["id"]
            .as_i64()
            .unwrap()
    };
    assert_eq!(stages.as_array().unwrap().len(), 3, "{stages}");
    let page = eventually(&admin, &format!("/api/v1/logs?project={project}"), |page| {
        page["entries"].as_array().is_some_and(|e| e.len() == 5)
    })
    .await;
    let mut placed: Vec<(String, i64)> = page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["message"].as_str().unwrap().to_owned(), e["environment_id"].as_i64().unwrap()))
        .collect();
    placed.sort();
    assert_eq!(
        placed,
        [
            ("from QA".to_owned(), id_of("qa")),
            ("from dev".to_owned(), id_of("dev")),
            ("from production".to_owned(), id_of("production")),
            ("from qa".to_owned(), id_of("qa")),
            ("names no stage".to_owned(), id_of("production")),
        ]
    );
    app.stop().await;
}

#[tokio::test]
async fn holds_senders_to_their_size_and_rate() {
    let app = TestApp::start_with(&[
        ("LOGDOCK_INGEST_MAX_DECOMPRESSED", "2KB"),
        ("LOGDOCK_INGEST_RATE", "1"),
        ("LOGDOCK_INGEST_BURST", "1"),
    ])
    .await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    let large = gzip(&log_envelope("production", &"x".repeat(10_000)));
    let refused = app.request(envelope_request(project, &key, large, true)).await;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);

    let small = || log_envelope("production", "x").into_bytes();
    assert_eq!(app.request(envelope_request(project, &key, small(), false)).await.status, StatusCode::OK);
    let limited = app.request(envelope_request(project, &key, small(), false)).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers.contains_key(header::RETRY_AFTER));
    assert!(limited.headers["x-sentry-rate-limits"].to_str().unwrap().ends_with("::key"));
    app.stop().await;
}

fn ndjson(key: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/ingest/logs")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn forwarders_send_ndjson_and_find_it_committed() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (_, _, key) = project_with_key(&admin, "orders").await;
    let lines = [
        r#"{"timestamp":"2026-11-18T09:15:02Z","level":"error","message":"nginx upstream timed out","service":"nginx","host":"docker-host-01"}"#,
        r#"{"message":"Job failed","context":{"job":"SendMail"},"level":400,"level_name":"ERROR","channel":"production","datetime":"2026-11-18T09:15:03.000000+00:00","extra":{"service":"horizon"}}"#,
        "not json",
        r#"{"message":"wrong stage","environment":"staging"}"#,
    ];
    let answer = app.request(ndjson(&key, &lines.join("\n"))).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(answer.json(), json!({"accepted": 2, "rejected": 2}));
    // Committed before the answer: found at once.
    let page = admin.get("/api/v1/logs?level=error").await.json();
    let messages: Vec<&str> =
        page["entries"].as_array().unwrap().iter().map(|e| e["message"].as_str().unwrap()).collect();
    assert_eq!(messages, ["Job failed", "nginx upstream timed out"]);
    assert_eq!(page["entries"][0]["attributes"]["job"], "SendMail");

    assert_eq!(app.request(ndjson("wrong", "{}")).await.status, StatusCode::UNAUTHORIZED);
    let all_bad = app.request(ndjson(&key, "nope\n")).await;
    assert_eq!(all_bad.status, StatusCode::BAD_REQUEST);
    app.stop().await;
}

#[tokio::test]
async fn pages_through_many_entries_and_streams_new_ones() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (_, _, key) = project_with_key(&admin, "orders").await;
    let lines: Vec<String> = (0..25)
        .map(|i| format!(r#"{{"timestamp":"2026-11-18T09:{:02}:00Z","message":"entry {i}"}}"#, i))
        .collect();
    assert_eq!(app.request(ndjson(&key, &lines.join("\n"))).await.status, StatusCode::OK);
    let mut seen = Vec::new();
    let mut path = "/api/v1/logs?limit=10".to_owned();
    loop {
        let page = admin.get(&path).await.json();
        seen.extend(
            page["entries"].as_array().unwrap().iter().map(|e| e["message"].as_str().unwrap().to_owned()),
        );
        match page["next"].as_str() {
            Some(next) => path = format!("/api/v1/logs?limit=10&before={next}"),
            None => break,
        }
    }
    assert_eq!(seen.len(), 25);
    assert_eq!(seen[0], "entry 24");
    assert_eq!(seen[24], "entry 0");
    assert_eq!(admin.get("/api/v1/logs?before=garbage").await.status, StatusCode::BAD_REQUEST);
    assert_eq!(admin.get("/api/v1/logs?level=loud").await.status, StatusCode::BAD_REQUEST);
    assert_eq!(admin.get("/api/v1/logs/not-an-id").await.status, StatusCode::NOT_FOUND);

    // The live view: a stream that hears what is written after it opened.
    let request = Request::builder()
        .uri("/api/v1/logs/stream?q=live")
        .header(header::COOKIE, admin.cookie.clone().unwrap())
        .body(Body::empty())
        .unwrap();
    let response = tower::ServiceExt::oneshot(app.router.clone(), request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    let mut body = response.into_body().into_data_stream();
    app.request(ndjson(
        &key,
        r#"{"message":"not for the stream"}
{"message":"live entry"}"#,
    ))
    .await;
    let chunk =
        tokio::time::timeout(std::time::Duration::from_secs(5), futures_next(&mut body)).await.unwrap();
    let text = String::from_utf8_lossy(&chunk).into_owned();
    assert!(
        text.contains("event: entry") && text.contains("live entry") && !text.contains("not for the stream"),
        "{text}"
    );
    let anonymous = app.get("/api/v1/logs/stream").await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    app.stop().await;
}

async fn futures_next(body: &mut axum::body::BodyDataStream) -> Vec<u8> {
    use http_body_util::BodyExt as _;
    loop {
        let frame = body.frame().await.expect("the stream goes on").expect("no error");
        if let Ok(data) = frame.into_data()
            && !data.is_empty()
            && !data.starts_with(b":")
        {
            return data.to_vec();
        }
    }
}
