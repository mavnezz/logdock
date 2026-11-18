//! The harness of the feature tests: Logdock assembled as in production,
//! with its data in a temporary directory, called through its router.

#![allow(dead_code)]

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use logdockd::app::App;
use logdockd::config::Config;
use serde_json::Value;
use tower::ServiceExt;

pub const SETUP_CODE: &str = "TEST-CODE";
pub const PASSWORD: &str = "correct horse battery";

pub struct TestApp {
    pub router: Router,
    pub app: Option<App>,
    pub dir: tempfile::TempDir,
}

pub struct Answer {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub fn config(dir: &std::path::Path, extra: &[(&str, &str)]) -> Config {
    let mut vars: Vec<(String, String)> = vec![
        ("LOGDOCK_DATA_DIR".into(), dir.join("data").display().to_string()),
        ("LOGDOCK_WEB_DIR".into(), String::new()),
        ("LOGDOCK_MIN_FREE_DISK".into(), "1".into()),
        ("LOGDOCK_PUBLIC_URL".into(), "http://logdock.test".into()),
        ("LOGDOCK_SETUP_CODE".into(), SETUP_CODE.into()),
    ];
    vars.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
    Config::from_lookup(|name| vars.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.clone())).unwrap()
}

impl TestApp {
    pub async fn start() -> Self {
        Self::start_with(&[]).await
    }

    pub async fn start_with(extra: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let app = App::build(&config(dir.path(), extra)).await.unwrap();
        Self { router: app.router.clone(), app: Some(app), dir }
    }

    pub async fn request(&self, request: Request<Body>) -> Answer {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes().to_vec();
        Answer { status, headers, body }
    }

    pub async fn get(&self, path: &str) -> Answer {
        self.request(Request::builder().method(Method::GET).uri(path).body(Body::empty()).unwrap()).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Answer {
        let request = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        self.request(request).await
    }

    pub async fn stop(mut self) {
        if let Some(app) = self.app.take() {
            app.shutdown(std::time::Duration::from_secs(5)).await;
        }
    }
}

/// A browser with a session: keeps the cookie and sends the CSRF token and
/// the origin with every change.
pub struct Browser<'a> {
    pub app: &'a TestApp,
    pub cookie: Option<String>,
    pub csrf: Option<String>,
}

impl<'a> Browser<'a> {
    pub fn new(app: &'a TestApp) -> Self {
        Self { app, cookie: None, csrf: None }
    }

    /// Runs the first setup as `email` and keeps the session.
    pub async fn setup(app: &'a TestApp, email: &str) -> Self {
        let mut browser = Self::new(app);
        let answer = browser
            .send(
                Method::POST,
                "/api/v1/setup",
                Some(serde_json::json!({
                    "code": SETUP_CODE, "email": email, "name": "Admin", "password": PASSWORD
                })),
            )
            .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
        browser.keep(&answer);
        browser
    }

    pub async fn login(app: &'a TestApp, email: &str, password: &str) -> Self {
        let mut browser = Self::new(app);
        let answer = browser
            .send(
                Method::POST,
                "/api/v1/auth/login",
                Some(serde_json::json!({"email": email, "password": password})),
            )
            .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
        browser.keep(&answer);
        browser
    }

    pub fn keep(&mut self, answer: &Answer) {
        if let Some(cookie) = answer.headers.get(header::SET_COOKIE) {
            let cookie = cookie.to_str().unwrap();
            self.cookie = Some(cookie.split(';').next().unwrap().to_owned());
        }
        if let Some(token) = answer.json()["csrf_token"].as_str() {
            self.csrf = Some(token.to_owned());
        }
    }

    pub async fn send(&self, method: Method, path: &str, body: Option<Value>) -> Answer {
        let mut request =
            Request::builder().method(method.clone()).uri(path).header(header::HOST, "logdock.test");
        if method != Method::GET {
            request = request.header(header::ORIGIN, "http://logdock.test");
        }
        if let Some(cookie) = &self.cookie {
            request = request.header(header::COOKIE, cookie);
        }
        if let Some(csrf) = &self.csrf {
            request = request.header("x-csrf-token", csrf);
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => request.body(Body::empty()).unwrap(),
        };
        self.app.request(request).await
    }

    pub async fn get(&self, path: &str) -> Answer {
        self.send(Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Answer {
        self.send(Method::POST, path, Some(body)).await
    }

    pub async fn patch(&self, path: &str, body: Value) -> Answer {
        self.send(Method::PATCH, path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Answer {
        self.send(Method::DELETE, path, None).await
    }
}
