//! Feature tests: projects, stages, keys and settings.

mod common;

use axum::http::StatusCode;
use common::{Browser, PASSWORD, TestApp};
use serde_json::json;

#[tokio::test]
async fn administrators_create_projects_stages_and_keys() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let created = admin
        .post(
            "/api/v1/projects",
            json!({"name": "orders-backend", "environments": ["production", "staging"]}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let project = created.json();
    let id = project["id"].as_i64().unwrap();
    let production = project["environments"][0]["id"].as_i64().unwrap();
    assert_eq!(project["environments"][1]["name"], "staging");
    assert_eq!(project["settings"]["notify_new_issues"], true);

    let taken = admin.post("/api/v1/projects", json!({"name": "Orders-Backend"})).await;
    assert_eq!(taken.json()["code"], "name_taken");

    let key = admin
        .post(&format!("/api/v1/projects/{id}/keys"), json!({"environment_id": production, "label": "web"}))
        .await;
    assert_eq!(key.status, StatusCode::CREATED, "{}", key.text());
    let key = key.json();
    let secret = key["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 32);
    assert_eq!(key["dsn"], format!("http://{secret}@logdock.test/{id}"));
    let keys = admin.get(&format!("/api/v1/projects/{id}/keys")).await.json();
    assert_eq!(keys[0]["prefix"], &secret[..8]);
    assert!(keys[0].get("secret").is_none(), "the secret is shown once");

    let in_use = admin.delete(&format!("/api/v1/projects/{id}/environments/{production}")).await;
    assert_eq!(in_use.json()["code"], "environment_in_use");
    let key_id = keys[0]["id"].as_i64().unwrap();
    assert_eq!(
        admin.delete(&format!("/api/v1/projects/{id}/keys/{key_id}")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        admin.delete(&format!("/api/v1/projects/{id}/environments/{production}")).await.status,
        StatusCode::NO_CONTENT
    );
    let dev =
        admin.post(&format!("/api/v1/projects/{id}/environments"), json!({"name": "Development"})).await;
    assert_eq!(dev.json()["name"], "development");

    let counts = admin.get(&format!("/api/v1/projects/{id}/ingest?days=7")).await;
    assert_eq!(counts.status, StatusCode::OK);
    app.stop().await;
}

#[tokio::test]
async fn settings_are_checked_and_kept() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let id = admin.post("/api/v1/projects", json!({"name": "orders"})).await.json()["id"].as_i64().unwrap();
    let settings = json!({
        "notify": ["ops@example.com", " "],
        "notify_new_issues": true,
        "notify_regressions": false,
        "notify_monitors": true,
        "redaction": {"extra_keys": ["customer_pin"], "allowed_keys": null, "scrub_emails": true, "scrub_ip_addresses": false}
    });
    let changed = admin
        .patch(&format!("/api/v1/projects/{id}"), json!({"name": "orders-api", "settings": settings}))
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text());
    let project = admin.get(&format!("/api/v1/projects/{id}")).await.json();
    assert_eq!(project["name"], "orders-api");
    assert_eq!(project["settings"]["notify"], json!(["ops@example.com"]));
    assert_eq!(project["settings"]["redaction"]["scrub_emails"], true);

    let mut bad = project["settings"].clone();
    bad["notify"] = json!(["not an address"]);
    let refused = admin.patch(&format!("/api/v1/projects/{id}"), json!({"settings": bad})).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(admin.delete(&format!("/api/v1/projects/{id}")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(admin.get(&format!("/api/v1/projects/{id}")).await.status, StatusCode::NOT_FOUND);
    app.stop().await;
}

#[tokio::test]
async fn members_read_projects_but_change_nothing() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    admin
        .post(
            "/api/v1/accounts",
            json!({"email": "bob@example.com", "name": "Bob", "role": "member", "password": PASSWORD}),
        )
        .await;
    let project = admin
        .post("/api/v1/projects", json!({"name": "orders", "environments": ["production"]}))
        .await
        .json();
    let id = project["id"].as_i64().unwrap();
    let bob = Browser::login(&app, "bob@example.com", PASSWORD).await;
    assert_eq!(bob.get("/api/v1/projects").await.json().as_array().unwrap().len(), 1);
    assert_eq!(bob.get(&format!("/api/v1/projects/{id}/ingest")).await.status, StatusCode::OK);
    assert_eq!(bob.post("/api/v1/projects", json!({"name": "mine"})).await.status, StatusCode::FORBIDDEN);
    assert_eq!(bob.get(&format!("/api/v1/projects/{id}/keys")).await.status, StatusCode::FORBIDDEN);
    assert_eq!(bob.delete(&format!("/api/v1/projects/{id}")).await.status, StatusCode::FORBIDDEN);
    app.stop().await;
}
