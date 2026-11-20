//! Feature tests: performance problems in transactions (N+1 queries, slow
//! queries, requests over their threshold) become issues, through the real
//! router.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{Browser, TestApp, eventually, project_with_key};
use serde_json::{Value, json};

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()
}

/// A request as sentry-laravel sends it, with `queries` SELECTs in a row
/// of `ms` each under the route.
fn request(trace: &str, duration_ms: f64, queries: usize, ms: f64) -> Value {
    let start = now() - 5.0;
    let spans: Vec<Value> = (0..queries)
        .map(|n| {
            let from = start + 0.010 + n as f64 * (ms + 1.0) / 1000.0;
            json!({
                "span_id": format!("{:016x}", n + 1), "parent_span_id": "aaaaaaaaaaaaaaaa", "trace_id": trace,
                "op": "db.sql.query", "description": format!("select * from items where order_id = {n}"),
                "start_timestamp": from, "timestamp": from + ms / 1000.0, "data": {"db.system": "mysql"}
            })
        })
        .chain(std::iter::once(json!({
            "span_id": "aaaaaaaaaaaaaaaa", "parent_span_id": "bbbbbbbbbbbbbbbb", "trace_id": trace,
            "op": "http.route", "description": "OrderController@show",
            "start_timestamp": start + 0.005, "timestamp": start + duration_ms / 1000.0 - 0.005
        })))
        .collect();
    json!({
        "type": "transaction", "transaction": "/orders/{order}", "transaction_info": {"source": "route"},
        "start_timestamp": start, "timestamp": start + duration_ms / 1000.0,
        "environment": "production", "release": "4f2c1e9", "server_name": "orders-web-1",
        "tags": {"service": "web", "host": "docker-host-01"},
        "contexts": {"trace": {
            "trace_id": trace, "span_id": "bbbbbbbbbbbbbbbb", "op": "http.server", "status": "ok",
            "data": {"http.request.method": "GET", "http.response.status_code": 200}
        }},
        "spans": spans
    })
}

async fn send(app: &TestApp, project: i64, key: &str, transaction: &Value) {
    let body = format!(
        "{}\n{}\n{}\n",
        json!({"event_id": format!("{:032x}", rand_id()), "sent_at": "2026-11-20T09:15:03Z"}),
        json!({"type": "transaction", "content_type": "application/json"}),
        transaction
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/{project}/envelope/"))
        .header("x-sentry-auth", format!("Sentry sentry_version=7, sentry_key={key}"))
        .body(Body::from(body))
        .unwrap();
    let answer = app.request(request).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
}

fn rand_id() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

#[tokio::test]
async fn n_plus_one_queries_and_slow_requests_become_issues() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;

    // Defaults: requests over 2 s, N+1 queries, SELECTs over 1 s.
    let settings = admin.get(&format!("/api/v1/projects/{project}")).await.json()["settings"].clone();
    assert_eq!(
        settings["performance"],
        json!({"request_ms": 2000, "job_ms": null, "overrides": [], "n_plus_one": true, "slow_query_ms": 1000})
    );

    // Twelve of the same query in a row, 10 ms each.
    send(&app, project, &key, &request("0123456789abcdef0123456789abcdef", 200.0, 12, 10.0)).await;
    let issues = eventually(&admin, "/api/v1/issues", |i| i.as_array().is_some_and(|i| !i.is_empty())).await;
    assert_eq!(issues.as_array().unwrap().len(), 1, "{issues}");
    let issue = &issues[0];
    assert_eq!(issue["title"], "N+1 query: select * from items where order_id = ?");
    assert_eq!(issue["level"], "warning");
    assert_eq!(issue["culprit"], "GET /orders/{order}");
    let id = issue["id"].as_i64().unwrap();
    let detail = admin.get(&format!("/api/v1/issues/{id}")).await.json();
    let latest = &detail["latest"];
    assert_eq!(latest["contexts"]["performance"]["count"], 12);
    assert_eq!(latest["contexts"]["performance"]["total_ms"], 120.0);
    assert_eq!(latest["trace_id"], "0123456789abcdef0123456789abcdef");
    assert_eq!(latest["span_id"], "0000000000000001", "the first of the run");

    // The same problem within a minute is not reported again.
    send(&app, project, &key, &request("1123456789abcdef0123456789abcdef", 200.0, 12, 10.0)).await;
    let again = admin.get(&format!("/api/v1/issues/{id}")).await.json();
    assert_eq!(again["issue"]["count"], 1);

    // A threshold of the route's own.
    let mut changed = settings.clone();
    changed["performance"]["overrides"] = json!([{"name": "GET /orders/{order}", "ms": 100}]);
    changed["performance"]["n_plus_one"] = json!(false);
    let saved = admin.patch(&format!("/api/v1/projects/{project}"), json!({"settings": changed})).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert_eq!(saved.json()["settings"]["performance"]["overrides"][0]["ms"], 100);
    send(&app, project, &key, &request("2123456789abcdef0123456789abcdef", 300.0, 12, 10.0)).await;
    let issues = eventually(&admin, "/api/v1/issues", |i| i.as_array().is_some_and(|i| i.len() == 2)).await;
    let titles: Vec<&str> = issues.as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert!(titles.contains(&"Slow request: GET /orders/{order} over 100 ms"), "{titles:?}");
    app.stop().await;
}
