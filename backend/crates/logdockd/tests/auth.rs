//! Feature tests: the first setup, sign-in, sessions, accounts and the
//! audit log through the real router and database.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{Browser, PASSWORD, TestApp};
use serde_json::json;

#[tokio::test]
async fn the_first_setup_creates_the_administrator_once() {
    let app = TestApp::start().await;
    let anonymous = Browser::new(&app);
    assert_eq!(anonymous.get("/api/v1/setup").await.json()["needed"], true);
    let wrong = anonymous
        .post(
            "/api/v1/setup",
            json!({"code": "WRONG-ONE", "email": "ada@example.com", "name": "Ada", "password": PASSWORD}),
        )
        .await;
    assert_eq!(
        (wrong.status, wrong.json()["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("invalid_setup_code"))
    );

    let admin = Browser::setup(&app, "ada@example.com").await;
    let session = admin.get("/api/v1/auth/session").await;
    assert_eq!(session.status, StatusCode::OK);
    assert_eq!(session.json()["account"]["role"], "admin");
    assert_eq!(anonymous.get("/api/v1/setup").await.json()["needed"], false);
    let again = anonymous
        .post("/api/v1/setup", json!({"code": common::SETUP_CODE, "email": "eve@example.com", "name": "Eve", "password": PASSWORD}))
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    app.stop().await;
}

#[tokio::test]
async fn signs_in_and_out_with_cookie_and_csrf_token() {
    let app = TestApp::start().await;
    Browser::setup(&app, "ada@example.com").await;
    let anonymous = Browser::new(&app);
    let wrong = anonymous
        .post("/api/v1/auth/login", json!({"email": "ada@example.com", "password": "nope nope nope"}))
        .await;
    assert_eq!(
        (wrong.status, wrong.json()["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("invalid_credentials"))
    );

    let ada = Browser::login(&app, "ADA@example.com", PASSWORD).await;
    let cookie = ada.cookie.clone().unwrap();
    assert!(cookie.starts_with("logdock_session="), "{cookie}");

    // A change without the CSRF token is refused, and so is one from
    // another site.
    let without_csrf = Browser { app: &app, cookie: Some(cookie.clone()), csrf: None };
    let refused = without_csrf.patch("/api/v1/me", json!({"name": "Ada L."})).await;
    assert_eq!(
        (refused.status, refused.json()["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("csrf_rejected"))
    );
    let profile = ada.patch("/api/v1/me", json!({"name": "Ada L.", "locale": "de"})).await;
    assert_eq!(profile.status, StatusCode::OK, "{}", profile.text());
    assert_eq!(profile.json()["locale"], "de");

    let logout = ada.send(Method::POST, "/api/v1/auth/logout", None).await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert_eq!(ada.get("/api/v1/auth/session").await.status, StatusCode::UNAUTHORIZED);
    app.stop().await;
}

#[tokio::test]
async fn refuses_to_sign_in_over_plain_http_with_secure_cookies() {
    let app = TestApp::start_with(&[("LOGDOCK_PUBLIC_URL", "https://logdock.test")]).await;
    let sign_in = |origin: &'static str| {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/setup")
            .header(header::HOST, "logdock.test")
            .header(header::ORIGIN, origin)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"code": common::SETUP_CODE, "email": "ada@example.com", "name": "Ada", "password": PASSWORD})
                    .to_string(),
            ))
            .unwrap();
        app.request(request)
    };

    // The browser would drop the secure cookie: the setup is refused and
    // can be done once the address is right.
    let refused = sign_in("http://logdock.test").await;
    assert_eq!(
        (refused.status, refused.json()["code"].as_str()),
        (StatusCode::CONFLICT, Some("insecure_connection"))
    );
    assert!(refused.headers.get(header::SET_COOKIE).is_none());
    assert_eq!(app.get("/api/v1/setup").await.json()["needed"], true);

    let done = sign_in("https://logdock.test").await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.text());
    let cookie = done.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with("__Host-logdock_session=") && cookie.contains("Secure"), "{cookie}");
    app.stop().await;
}

#[tokio::test]
async fn every_api_route_but_the_public_ones_needs_a_session() {
    let app = TestApp::start().await;
    let document = app.get("/api/v1/openapi.json").await.json();
    let public = ["/health/live", "/health/ready", "/api/v1/setup", "/api/v1/auth/login"];
    let anonymous = Browser::new(&app);
    let mut checked = 0;
    for (path, operations) in document["paths"].as_object().unwrap() {
        if public.contains(&path.as_str()) {
            continue;
        }
        let concrete =
            path.split('/').map(|s| if s.starts_with('{') { "1" } else { s }).collect::<Vec<_>>().join("/");
        for method in operations.as_object().unwrap().keys() {
            let method: Method = method.to_uppercase().parse().unwrap();
            let body = (method != Method::GET && method != Method::DELETE).then(|| json!({}));
            let answer = anonymous.send(method.clone(), &concrete, body).await;
            assert_eq!(answer.status, StatusCode::UNAUTHORIZED, "{method} {path} answered without a session");
            checked += 1;
        }
    }
    assert!(checked > 10, "only {checked} routes checked");
    app.stop().await;
}

#[tokio::test]
async fn administrators_manage_accounts_and_read_the_audit_log() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let created = admin
        .post(
            "/api/v1/accounts",
            json!({"email": "bob@example.com", "name": "Bob", "role": "member", "password": PASSWORD}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let bob_id = created.json()["id"].as_i64().unwrap();
    let taken = admin
        .post(
            "/api/v1/accounts",
            json!({"email": "bob@example.com", "name": "Bob", "role": "member", "password": PASSWORD}),
        )
        .await;
    assert_eq!(taken.json()["code"], "email_taken");

    let bob = Browser::login(&app, "bob@example.com", PASSWORD).await;
    assert_eq!(bob.get("/api/v1/accounts").await.status, StatusCode::FORBIDDEN);
    assert_eq!(bob.get("/api/v1/audit").await.status, StatusCode::FORBIDDEN);

    let own =
        admin.patch(&format!("/api/v1/accounts/{}", admin_id(&admin).await), json!({"role": "member"})).await;
    assert_eq!(own.json()["code"], "own_account");
    let disabled = admin.patch(&format!("/api/v1/accounts/{bob_id}"), json!({"disabled": true})).await;
    assert_eq!(disabled.json()["disabled"], true);
    assert_eq!(bob.get("/api/v1/auth/session").await.status, StatusCode::UNAUTHORIZED, "bob's session ended");

    let reset = admin
        .post(&format!("/api/v1/accounts/{bob_id}/password"), json!({"password": "a new password"}))
        .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT);
    assert_eq!(admin.delete(&format!("/api/v1/accounts/{bob_id}")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(admin.get("/api/v1/accounts").await.json().as_array().unwrap().len(), 1);

    let audit = admin.get("/api/v1/audit?limit=50").await.json();
    let actions: Vec<&str> =
        audit.as_array().unwrap().iter().map(|e| e["action"].as_str().unwrap()).collect();
    for action in
        ["setup", "account.create", "login", "account.update", "account.password_reset", "account.delete"]
    {
        assert!(actions.contains(&action), "{action} missing from {actions:?}");
    }
    app.stop().await;
}

async fn admin_id(browser: &Browser<'_>) -> i64 {
    browser.get("/api/v1/auth/session").await.json()["account"]["id"].as_i64().unwrap()
}

#[tokio::test]
async fn people_change_their_password() {
    let app = TestApp::start().await;
    let ada = Browser::setup(&app, "ada@example.com").await;
    let wrong =
        ada.post("/api/v1/me/password", json!({"current": "not it at all", "new": "a new password"})).await;
    assert_eq!(wrong.json()["code"], "wrong_password");
    let short = ada.post("/api/v1/me/password", json!({"current": PASSWORD, "new": "short"})).await;
    assert_eq!(short.status, StatusCode::UNPROCESSABLE_ENTITY);
    let changed =
        ada.post("/api/v1/me/password", json!({"current": PASSWORD, "new": "a new password"})).await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT);
    Browser::login(&app, "ada@example.com", "a new password").await;
    app.stop().await;
}
