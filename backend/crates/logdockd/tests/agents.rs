//! Feature tests: API tokens and the MCP server, as a script or Claude
//! uses them, through the real router.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{Answer, Browser, TestApp, eventually, project_with_key};
use serde_json::{Value, json};

const TRACE: &str = "5b8efff798038103d269b633813fc60c";

/// A request with an API token, as a program sends it: no cookie, no CSRF
/// token, no origin.
async fn with_token(app: &TestApp, token: &str, method: Method, path: &str, body: Option<Value>) -> Answer {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json");
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    app.request(request.body(body).unwrap()).await
}

async fn new_token(admin: &Browser<'_>, name: &str, scope: &str) -> String {
    let created = admin.post("/api/v1/me/tokens", json!({"name": name, "scope": scope})).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let created = created.json();
    assert_eq!(created["mcp_url"], "http://logdock.test/mcp");
    assert_eq!(created["token"]["scope"], scope);
    created["secret"].as_str().unwrap().to_owned()
}

/// A log entry and an error event of one request, as sentry-php sends them.
async fn send_request_data(app: &TestApp, project: i64, key: &str) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let log = json!({"items": [{
        "timestamp": now, "trace_id": TRACE, "level": "warn", "body": "Payment provider slow for order 42",
        "attributes": {
            "sentry.environment": {"type": "string", "value": "production"},
            "service": {"type": "string", "value": "web"},
            "order_id": {"type": "integer", "value": 42}
        }
    }]});
    let event = json!({
        "event_id": "0f6ad8ed3b3a4cbd9b4d1f9e2a7c5b10", "timestamp": now, "platform": "php", "level": "error",
        "environment": "production", "release": "4f2c1e9", "transaction": "POST /orders/{order}/pay",
        "contexts": {"trace": {"trace_id": TRACE, "span_id": "d8f3c1a2b4e5f607"}},
        "exception": {"values": [{
            "type": "App\\Payments\\PaymentFailed", "value": "Payment for order 42 failed",
            "stacktrace": {"frames": [
                {"filename": "/vendor/laravel/framework/src/Illuminate/Pipeline/Pipeline.php", "lineno": 167, "in_app": false,
                 "function": "handle", "context_line": "return $next($request);"},
                {"filename": "/app/Payments/Gateway.php", "lineno": 31, "in_app": true, "function": "App\\Payments\\Gateway::charge",
                 "context_line": "throw new PaymentFailed($order);"}
            ]}
        }]}
    });
    let body = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        json!({"event_id": event["event_id"], "sent_at": "2026-11-20T09:15:03Z"}),
        json!({"type": "log", "item_count": 1, "content_type": "application/vnd.sentry.items.log+json"}),
        log,
        json!({"type": "event", "content_type": "application/json"}),
        event
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/{project}/envelope/"))
        .header("x-sentry-auth", format!("Sentry sentry_version=7, sentry_key={key}"))
        .body(Body::from(body))
        .unwrap();
    assert_eq!(app.request(request).await.status, StatusCode::OK);
}

#[tokio::test]
async fn api_tokens_read_for_their_account_and_change_issues_at_most() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    send_request_data(&app, project, &key).await;
    let issues = eventually(&admin, "/api/v1/issues", |i| i.as_array().is_some_and(|i| !i.is_empty())).await;
    let issue = issues[0]["id"].as_i64().unwrap();

    let read = new_token(&admin, "script", "read").await;
    assert!(read.starts_with("ldt_"));
    let projects = with_token(&app, &read, Method::GET, "/api/v1/projects", None).await;
    assert_eq!((projects.status, projects.json()[0]["name"].as_str()), (StatusCode::OK, Some("orders")));
    let logs = with_token(&app, &read, Method::GET, "/api/v1/logs?q=order_id:42", None).await;
    assert_eq!(logs.json()["entries"].as_array().unwrap().len(), 1);

    // It changes nothing, not even with an administrator's account.
    let resolve = json!({"action": "resolve"});
    for (method, path, body) in [
        (Method::PATCH, format!("/api/v1/issues/{issue}"), Some(resolve.clone())),
        (Method::PATCH, "/api/v1/me".to_owned(), Some(json!({"name": "Eve"}))),
        (Method::POST, "/api/v1/projects".to_owned(), Some(json!({"name": "shop"}))),
        (Method::POST, "/api/v1/me/tokens".to_owned(), Some(json!({"name": "more", "scope": "triage"}))),
        (Method::DELETE, format!("/api/v1/issues/{issue}"), None),
        (Method::POST, "/api/v1/auth/logout".to_owned(), None),
    ] {
        let refused = with_token(&app, &read, method.clone(), &path, body).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{method} {path}: {}", refused.text());
    }
    let session = with_token(&app, &read, Method::GET, "/api/v1/auth/session", None).await;
    assert_eq!(session.status, StatusCode::FORBIDDEN, "a token has no session");
    let accounts = with_token(&app, &read, Method::GET, "/api/v1/accounts", None).await;
    assert_eq!(accounts.status, StatusCode::FORBIDDEN, "a token reads data, not accounts");

    // A token with the triage scope changes issues' state, and only that.
    let triage = new_token(&admin, "fixer", "triage").await;
    let resolved =
        with_token(&app, &triage, Method::PATCH, &format!("/api/v1/issues/{issue}"), Some(resolve)).await;
    assert_eq!(resolved.status, StatusCode::OK, "{}", resolved.text());
    assert_eq!(resolved.json()["status"], "resolved");
    let delete = with_token(&app, &triage, Method::DELETE, &format!("/api/v1/issues/{issue}"), None).await;
    assert_eq!(delete.status, StatusCode::FORBIDDEN);
    let audit = admin.get("/api/v1/audit").await.json();
    assert!(audit.to_string().contains("ada@example.com (token fixer)"), "{audit}");

    // Listed and revoked in a session; revoked, it stops at once.
    let tokens = admin.get("/api/v1/me/tokens").await.json();
    let names: Vec<&str> = tokens.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["fixer", "script"]);
    assert!(tokens[1]["last_used_at"].is_string());
    let id = tokens[1]["id"].as_i64().unwrap();
    assert_eq!(admin.delete(&format!("/api/v1/me/tokens/{id}")).await.status, StatusCode::NO_CONTENT);
    let gone = with_token(&app, &read, Method::GET, "/api/v1/projects", None).await;
    assert_eq!(gone.status, StatusCode::UNAUTHORIZED);
    let unknown = with_token(&app, "ldt_0000", Method::GET, "/api/v1/projects", None).await;
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    app.stop().await;
}

async fn mcp(app: &TestApp, token: Option<&str>, message: Value) -> Answer {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    app.request(request.body(Body::from(message.to_string())).unwrap()).await
}

/// Calls a tool and reads its answer's JSON; `Err` with its text when the
/// tool failed.
async fn call(app: &TestApp, token: &str, tool: &str, arguments: Value) -> Result<Value, String> {
    let message = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": tool, "arguments": arguments}});
    let answer = mcp(app, Some(token), message).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let result = &answer.json()["result"];
    let text = result["content"][0]["text"].as_str().unwrap().to_owned();
    if result["isError"] == true { Err(text) } else { Ok(serde_json::from_str(&text).unwrap()) }
}

#[tokio::test]
async fn claude_reads_logs_issues_and_traces_through_mcp() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    send_request_data(&app, project, &key).await;
    eventually(&admin, "/api/v1/issues", |i| i.as_array().is_some_and(|i| !i.is_empty())).await;
    let read = new_token(&admin, "claude", "read").await;

    // Only with a token.
    let anonymous = mcp(&app, None, json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert!(anonymous.headers[header::WWW_AUTHENTICATE].to_str().unwrap().starts_with("Bearer"));
    assert_eq!(app.get("/mcp").await.status, StatusCode::METHOD_NOT_ALLOWED);

    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-code", "version": "2"}
    }});
    let answer = mcp(&app, Some(&read), init).await.json();
    assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(answer["result"]["serverInfo"]["name"], "logdock");
    assert!(answer["result"]["capabilities"]["tools"].is_object());
    let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(mcp(&app, Some(&read), initialized).await.status, StatusCode::ACCEPTED);
    let old = json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "2024-01-01"}});
    assert_eq!(mcp(&app, Some(&read), old).await.json()["result"]["protocolVersion"], "2025-06-18");

    let tools =
        mcp(&app, Some(&read), json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"})).await.json();
    let names: Vec<&str> =
        tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"search_logs") && names.contains(&"get_issue"), "{names:?}");
    assert!(!names.contains(&"change_issue"), "a read token has no tool that changes");

    let projects = call(&app, &read, "list_projects", json!({})).await.unwrap();
    assert_eq!(projects, json!([{"id": project, "name": "orders", "stages": ["production", "staging"]}]));

    let logs =
        call(&app, &read, "search_logs", json!({"query": "order_id:42", "project": "orders", "since": "1h"}))
            .await
            .unwrap();
    let entry = &logs["entries"][0];
    assert_eq!((entry["project"].as_str(), entry["stage"].as_str()), (Some("orders"), Some("production")));
    assert_eq!(entry["message"], "Payment provider slow for order 42");
    assert!(entry.get("project_id").is_none() && entry.get("received").is_none(), "{entry}");
    let none = call(&app, &read, "search_logs", json!({"stage": "staging"})).await.unwrap();
    assert!(none.get("entries").is_none(), "no entries in staging: {none}");

    let issues = call(&app, &read, "list_issues", json!({"stage": "production"})).await.unwrap();
    let id = issues[0]["id"].as_i64().unwrap();
    assert_eq!(issues[0]["title"], "PaymentFailed: Payment for order 42 failed");
    let issue = call(&app, &read, "get_issue", json!({"id": id})).await.unwrap();
    let frames = &issue["latest"]["exceptions"][0]["frames"];
    assert_eq!(frames[1]["context_line"], "throw new PaymentFailed($order);", "{issue}");
    assert!(frames[0].get("context_line").is_none(), "no code of libraries");
    assert!(issue.get("hours").is_none());
    let event_id = issue["latest"]["event_id"].as_str().unwrap().to_owned();
    assert_eq!(
        call(&app, &read, "get_event", json!({"event_id": event_id})).await.unwrap()["event"]["level"],
        "error"
    );

    let trace = call(&app, &read, "get_trace", json!({"trace_id": TRACE})).await.unwrap();
    assert_eq!(trace["logs"].as_array().unwrap().len(), 1, "{trace}");
    assert!(call(&app, &read, "performance", json!({"kind": "request", "since": "1d"})).await.is_ok());
    assert!(call(&app, &read, "list_monitors", json!({})).await.is_ok());
    assert!(call(&app, &read, "list_queues", json!({"since": "1d"})).await.is_ok());
    let releases = call(&app, &read, "list_releases", json!({"project": "orders"})).await.unwrap();
    assert_eq!(releases[0]["release"], "4f2c1e9", "{releases}");

    // Mistakes come back as the tool's failure, with what to do.
    let wrong = call(&app, &read, "search_logs", json!({"project": "shop"})).await.unwrap_err();
    assert!(wrong.contains("list_projects"), "{wrong}");
    assert!(call(&app, &read, "get_issue", json!({"id": 999})).await.unwrap_err().contains("404"));
    let unknown =
        mcp(&app, Some(&read), json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": "change_issue", "arguments": {}}}))
            .await;
    assert_eq!(unknown.json()["error"]["code"], -32602);
    let method =
        mcp(&app, Some(&read), json!({"jsonrpc": "2.0", "id": 10, "method": "resources/list"})).await;
    assert_eq!(method.json()["error"]["code"], -32601);
    let broken = app
        .request(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {read}"))
                .body(Body::from("{nope"))
                .unwrap(),
        )
        .await;
    assert_eq!(broken.json()["error"]["code"], -32700);

    // With the triage scope: one more tool.
    let triage = new_token(&admin, "claude-fix", "triage").await;
    let resolved = call(&app, &triage, "change_issue", json!({"id": id, "action": "resolve_next_release"}))
        .await
        .unwrap();
    assert_eq!(resolved["status"], "resolved", "{resolved}");
    assert_eq!(resolved["resolved_in_next_release"], "4f2c1e9");
    let batch = json!([
        {"jsonrpc": "2.0", "id": 1, "method": "ping"},
        {"jsonrpc": "2.0", "method": "notifications/cancelled"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}
    ]);
    let answers = mcp(&app, Some(&triage), batch).await.json();
    assert_eq!(answers.as_array().unwrap().len(), 2);
    assert!(answers[1]["result"]["tools"].to_string().contains("change_issue"));
    app.stop().await;
}
