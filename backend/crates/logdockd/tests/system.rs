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

fn log_line(key: &str, stage: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri("/api/v1/ingest/logs")
        .header("authorization", format!("Bearer {key}"))
        .body(axum::body::Body::from(format!(r#"{{"message":"hello {stage}","environment":"{stage}"}}"#)))
        .unwrap()
}

#[tokio::test]
async fn a_stage_keeps_its_data_as_long_as_it_chose() {
    let app = TestApp::start_with(&[("LOGDOCK_RETENTION_LOGS_DAYS", "14")]).await;
    let admin = common::Browser::setup(&app, "ada@example.com").await;
    let (project, production, _) = common::project_with_key(&admin, "orders").await;
    // A key of the whole project: its data names its stage.
    let key = admin.post(&format!("/api/v1/projects/{project}/keys"), serde_json::json!({})).await.json();
    let key = key["secret"].as_str().unwrap().to_owned();
    let mut detail = admin.get(&format!("/api/v1/projects/{project}")).await.json();
    let staging = detail["environments"].as_array().unwrap().iter().find(|e| e["name"] == "staging").unwrap()
        ["id"]
        .as_i64()
        .unwrap();

    // Out of range: refused.
    let mut settings = detail["settings"].clone();
    settings["retention"] = serde_json::json!([{"environment_id": staging, "logs_days": 0}]);
    let refused =
        admin.patch(&format!("/api/v1/projects/{project}"), serde_json::json!({"settings": settings})).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.text());
    settings["retention"] =
        serde_json::json!([{"environment_id": staging, "logs_days": 3, "traces_days": 1}]);
    let saved =
        admin.patch(&format!("/api/v1/projects/{project}"), serde_json::json!({"settings": settings})).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    detail = saved.json();
    assert_eq!(detail["settings"]["retention"][0]["logs_days"], 3);
    assert_eq!(detail["settings"]["retention"][0]["events_days"], serde_json::Value::Null);

    // Staging's entries go into segments of their own; production's do not.
    for stage in ["production", "staging", "staging"] {
        assert_eq!(app.request(log_line(&key, stage)).await.status, StatusCode::OK);
    }
    common::eventually(&admin, "/api/v1/logs", |l| l["entries"].as_array().is_some_and(|e| e.len() == 3))
        .await;
    let entries = admin.get("/api/v1/logs").await.json();
    let ids: Vec<&str> =
        entries["entries"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids.iter().filter(|id| id.contains(".3-")).count(), 2, "{ids:?}");
    let own = ids.iter().find(|id| id.contains(".3-")).unwrap();
    assert_eq!(admin.get(&format!("/api/v1/logs/{own}")).await.json()["message"], "hello staging");

    assert_eq!(app.app.as_ref().unwrap().measure_storage().await.unwrap(), 2);
    let storage = admin.get("/api/v1/system/storage").await.json();
    let segments = storage["segments"].as_array().unwrap();
    let days: Vec<(u64, bool)> = segments
        .iter()
        .map(|s| (s["retention_days"].as_u64().unwrap(), s["own_retention"].as_bool().unwrap()))
        .collect();
    assert_eq!(days, [(14, false), (3, true)], "{storage}");
    let stages = storage["stages"].as_array().unwrap();
    let rows = |environment: i64| {
        stages.iter().find(|s| s["environment_id"] == environment).unwrap()["rows"].clone()
    };
    assert_eq!((rows(production), rows(staging)), (serde_json::json!(1), serde_json::json!(2)));
    assert!(stages.iter().all(|s| s["bytes"].as_u64().unwrap() > 0 && s["kind"] == "logs"));
    assert_eq!(storage["forecast"], serde_json::Value::Null, "no full day yet");
    app.stop().await;
}
