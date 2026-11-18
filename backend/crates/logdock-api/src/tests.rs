//! Tests of the router itself: health, static files, errors, OpenAPI.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use logdock_kernel::PortError;
use logdock_retention::application::ports::StorageProbe;
use logdock_retention::application::{RetentionPorts, RetentionUseCases};
use logdock_retention::domain::RetentionPolicy;
use serde_json::Value;
use tower::ServiceExt;

use super::*;

struct Probe {
    free: u64,
}

#[async_trait]
impl StorageProbe for Probe {
    async fn database_ok(&self) -> bool {
        true
    }

    async fn free_bytes(&self) -> Result<u64, PortError> {
        Ok(self.free)
    }

    async fn stalled_writers(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

fn app(free: u64, web_dir: Option<std::path::PathBuf>) -> Router {
    let policy = RetentionPolicy { min_free_bytes: 10, ..RetentionPolicy::default() };
    let retention = RetentionUseCases::new(RetentionPorts { probe: Arc::new(Probe { free }) }, policy);
    let config = ApiConfig { web_dir, ..ApiConfig::default() };
    let identity = logdock_identity::testing::TestIdentity::new().use_cases;
    let projects = logdock_projects::testing::TestProjects::new().use_cases;
    let logs = logdock_logs::testing::memory().0;
    router(ApiState::new(UseCases { identity, logs, projects, retention }, config))
}

async fn get(app: &Router, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, body)
}

fn json(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap()
}

#[tokio::test]
async fn answers_liveness_and_readiness() {
    let (status, headers, body) = get(&app(100, None), "/health/live").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["status"], "live");
    assert!(headers.contains_key("x-request-id"));
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

    let (status, _, body) = get(&app(100, None), "/health/ready").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["status"], "ready");

    let (status, _, body) = get(&app(5, None), "/health/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json(&body)["problems"][0], "low_disk");
}

#[tokio::test]
async fn unknown_api_paths_are_problems_not_pages() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<!doctype html>").unwrap();
    let app = app(100, Some(dir.path().to_owned()));
    for path in ["/api/v1/nothing", "/api", "/health/nothing"] {
        let (status, headers, body) = get(&app, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(headers[header::CONTENT_TYPE], "application/problem+json");
        assert_eq!(json(&body)["code"], "not_found");
    }
}

#[tokio::test]
async fn serves_the_web_interface_and_its_routes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<!doctype html><title>Logdock</title>").unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/app-1234.js"), "console.log(1)").unwrap();
    let app = app(100, Some(dir.path().to_owned()));

    let (status, headers, body) = get(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("Logdock"));
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY));

    let (status, headers, _) = get(&app, "/assets/app-1234.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));

    // A route of the interface's router gets its index.html.
    let (status, _, body) = get(&app, "/issues/42").await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("Logdock"));

    let post = Request::post("/issues").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(post).await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);

    let (status, _, _) = get(&self::app(100, None), "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn publishes_its_openapi_document() {
    let (status, _, body) = get(&app(100, None), "/api/v1/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    let document = json(&body);
    assert_eq!(document["info"]["title"], "Logdock API");
    assert!(document["paths"]["/health/ready"].is_object());
    assert_eq!(openapi().info.title, "Logdock API");
}
