//! Feature tests: releases, from the errors and requests that carry them,
//! through the real router.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{Browser, TestApp, eventually, project_with_key};
use serde_json::{Value, json};

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()
}

/// A request `ago` seconds back.
fn request(release: &str, ago: f64, ms: f64, status: i64) -> Value {
    let start = now() - ago;
    json!({
        "type": "transaction", "transaction": "/checkout", "transaction_info": {"source": "route"},
        "start_timestamp": start, "timestamp": start + ms / 1000.0,
        "environment": "production", "release": release,
        "contexts": {"trace": {
            "trace_id": format!("{:032x}", (ms as u128) << 8 | status as u128), "span_id": "bbbbbbbbbbbbbbbb",
            "op": "http.server", "status": "ok",
            "data": {"http.request.method": "POST", "http.response.status_code": status}
        }},
        "spans": []
    })
}

fn error(release: &str, ago: f64, kind: &str, id: u128) -> Value {
    json!({
        "event_id": format!("{id:032x}"), "timestamp": now() - ago, "platform": "php", "level": "error",
        "environment": "production", "release": release,
        "exception": {"values": [{"type": kind, "value": "it broke", "stacktrace": {"frames": [
            {"filename": "/app/Http/Controllers/CheckoutController.php", "lineno": 12, "in_app": true, "function": kind}
        ]}}]}
    })
}

async fn send(app: &TestApp, project: i64, key: &str, item: &Value) {
    let kind = item["type"].as_str().unwrap_or("event");
    let body = format!(
        "{}\n{}\n{}\n",
        json!({"event_id": item.get("event_id").cloned().unwrap_or(json!(null))}),
        json!({"type": kind, "content_type": "application/json"}),
        item
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

#[tokio::test]
async fn releases_show_their_errors_new_issues_and_requests() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;

    // 1.0 an hour ago.
    send(&app, project, &key, &error("1.0", 3_500.0, "TimeoutException", 1)).await;
    for ms in [100.0, 120.0] {
        send(&app, project, &key, &request("1.0", 3_600.0, ms, 200)).await;
    }
    // 1.1 brings the old error again, a new one, a slower request and a 500.
    send(&app, project, &key, &error("1.1", 20.0, "TimeoutException", 2)).await;
    send(&app, project, &key, &error("1.1", 20.0, "PaymentFailed", 3)).await;
    send(&app, project, &key, &request("1.1", 30.0, 900.0, 200)).await;
    send(&app, project, &key, &request("1.1", 30.0, 300.0, 500)).await;

    let path = format!("/api/v1/releases?project={project}");
    let releases = eventually(&admin, &path, |r| {
        r.as_array().is_some_and(|r| r.len() == 2 && r[0]["requests"]["count"] == 2)
    })
    .await;
    let newest = &releases[0];
    assert_eq!(newest["release"], "1.1", "{releases}");
    assert_eq!((newest["events"].as_u64(), newest["issues"].as_u64()), (Some(2), Some(2)));
    assert_eq!(newest["new_issues"], 1, "PaymentFailed first appeared in 1.1");
    assert_eq!(newest["requests"]["flagged"], 1, "the 500");
    let older = &releases[1];
    assert_eq!((older["release"].as_str(), older["new_issues"].as_u64()), (Some("1.0"), Some(1)));
    assert_eq!(older["requests"]["flagged"], 0);
    assert!(older["requests"]["p95_ms"].as_f64() < newest["requests"]["p95_ms"].as_f64());

    // The issues new in a release.
    let new_in = admin.get("/api/v1/issues?status=all&release=1.1").await.json();
    let titles: Vec<&str> = new_in.as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["PaymentFailed: it broke"]);

    let elsewhere = admin.get("/api/v1/releases?project=999").await.json();
    assert_eq!(elsewhere, json!([]));
    app.stop().await;
}
