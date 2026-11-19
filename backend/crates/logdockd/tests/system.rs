//! Feature tests: the assembled server's health and its OpenAPI document.

mod common;

use axum::http::StatusCode;
use common::TestApp;

#[tokio::test]
async fn is_live_and_ready_with_a_fresh_data_directory() {
    let app = TestApp::start().await;
    let live = app.get("/health/live").await;
    assert_eq!(live.status, StatusCode::OK);
    let ready = app.get("/health/ready").await;
    assert_eq!(ready.status, StatusCode::OK, "{}", ready.text());
    assert_eq!(ready.json()["status"], "ready");
    assert!(app.dir.path().join("data/logdock.db").is_file());
    app.stop().await;
}

#[tokio::test]
async fn is_not_ready_when_the_disk_is_too_full() {
    let app = TestApp::start_with(&[("LOGDOCK_MIN_FREE_DISK", "1000000TB")]).await;
    let ready = app.get("/health/ready").await;
    assert_eq!(ready.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(ready.json()["problems"][0], "low_disk");
    app.stop().await;
}

#[tokio::test]
async fn serves_its_openapi_document() {
    let app = TestApp::start().await;
    let document = app.get("/api/v1/openapi.json").await;
    assert_eq!(document.status, StatusCode::OK);
    assert_eq!(document.json()["info"]["title"], "Logdock API");
    app.stop().await;
}

#[tokio::test]
async fn reports_storage_by_kind() {
    let app =
        TestApp::start_with(&[("LOGDOCK_RETENTION_LOGS_DAYS", "14"), ("LOGDOCK_MAX_STORAGE", "1GiB")]).await;
    let admin = common::Browser::setup(&app, "ada@example.com").await;
    let (_, _, key) = common::project_with_key(&admin, "orders").await;
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/api/v1/ingest/logs")
        .header("authorization", format!("Bearer {key}"))
        .body(axum::body::Body::from(r#"{"message":"hello"}"#))
        .unwrap();
    assert_eq!(app.request(request).await.status, StatusCode::OK);
    let storage = admin.get("/api/v1/system/storage").await.json();
    let logs = &storage["kinds"][0];
    assert_eq!(logs["kind"], "logs");
    assert_eq!(logs["segments"], 1);
    assert_eq!(logs["retention_days"], 14);
    assert!(logs["bytes"].as_u64().unwrap() > 0);
    assert_eq!(storage["max_storage_bytes"], 1024u64 * 1024 * 1024);
    assert!(storage["database_bytes"].as_u64().unwrap() > 0);
    let info = admin.get("/api/v1/system/info").await.json();
    assert_eq!(info["public_url"], "http://logdock.test");
    app.stop().await;
}
