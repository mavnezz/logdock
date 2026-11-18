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
