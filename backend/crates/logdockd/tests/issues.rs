//! Feature tests: error events as sentry-php and sentry-laravel send them,
//! grouped into issues, triaged, and found with their trace.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{Browser, TestApp, eventually, project_with_key};
use serde_json::{Value, json};

const TRACE: &str = "771a43a4192642f0b136d5159a501700";

fn now_seconds() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()
}

/// An `event` item as sentry-php 4.x writes it for an exception that
/// sentry-laravel reported (Serializer/EnvelopItems/EventItem.php).
fn error_event(event_id: &str, environment: &str, release: &str, line: i64) -> Value {
    json!({
        "event_id": event_id,
        "timestamp": now_seconds(),
        "platform": "php",
        "sdk": {"name": "sentry.php.laravel", "version": "4.15.0"},
        "logger": "laravel",
        "transaction": "POST /orders/{order}/pay",
        "server_name": "orders-web-1",
        "release": release,
        "environment": environment,
        "level": "error",
        "tags": {"service": "web", "host": "docker-host-01"},
        "user": {"id": "17", "email": "customer@example.com", "ip_address": "203.0.113.9"},
        "request": {
            "url": "https://orders.example.com/orders/42/pay",
            "method": "POST",
            "query_string": "retry=1",
            "headers": {"Host": ["orders.example.com"], "User-Agent": ["Mozilla/5.0"], "Cookie": ["laravel_session=abc"], "Authorization": ["Bearer secret-token"]},
            "cookies": {"laravel_session": "abc"},
            "data": {"card": "4111111111111111", "password": "hunter2"}
        },
        "contexts": {
            "os": {"name": "Linux", "version": "6.8.0"},
            "runtime": {"name": "php", "version": "8.4.15"},
            "trace": {"trace_id": TRACE, "span_id": "d8f3c1a2b4e5f607", "op": "http.server"}
        },
        "breadcrumbs": {"values": [
            {"type": "default", "category": "db.sql.query", "level": "info", "timestamp": now_seconds() - 1.0,
             "message": "select * from orders where id = ?", "data": {"bindings": [42]}},
            {"type": "default", "category": "log", "level": "warning", "timestamp": now_seconds() - 0.5,
             "message": "Payment provider slow"}
        ]},
        "exception": {"values": [
            {
                "type": "GuzzleHttp\\Exception\\ConnectException",
                "value": "cURL error 28: Operation timed out after 5001 milliseconds",
                "mechanism": {"type": "generic", "handled": true},
                "stacktrace": {"frames": [
                    {"filename": "/public/index.php", "abs_path": "/var/www/html/public/index.php", "lineno": 17, "in_app": true, "function": null},
                    {"filename": "/vendor/laravel/framework/src/Illuminate/Pipeline/Pipeline.php", "abs_path": "/var/www/html/vendor/laravel/framework/src/Illuminate/Pipeline/Pipeline.php", "lineno": 167, "in_app": false, "function": "Illuminate\\Pipeline\\Pipeline::Illuminate\\Pipeline\\{closure}"},
                    {"filename": "/app/Payments/Gateway.php", "abs_path": "/var/www/html/app/Payments/Gateway.php", "lineno": line, "in_app": true,
                     "function": "App\\Payments\\Gateway::charge", "pre_context": ["    {"], "context_line": "        $response = $this->http->post($url);", "post_context": ["    }"]}
                ]}
            },
            {
                "type": "App\\Payments\\PaymentFailed",
                "value": "Payment for order 42 failed",
                "mechanism": {"type": "generic", "handled": false},
                "stacktrace": {"frames": [
                    {"filename": "/app/Http/Controllers/PaymentController.php", "abs_path": "/var/www/html/app/Http/Controllers/PaymentController.php", "lineno": 31, "in_app": true, "function": "App\\Http\\Controllers\\PaymentController::pay"}
                ]}
            }
        ]}
    })
}

fn envelope(event: &Value) -> String {
    format!(
        "{}\n{}\n{}\n",
        json!({"event_id": event["event_id"], "sent_at": "2026-11-18T09:15:03Z", "sdk": {"name": "sentry.php.laravel", "version": "4.15.0"}}),
        json!({"type": "event", "content_type": "application/json"}),
        event
    )
}

fn envelope_request(project: i64, key: &str, body: String) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(format!("/api/{project}/envelope/"))
        .header(header::CONTENT_TYPE, "application/x-sentry-envelope")
        .header(
            "x-sentry-auth",
            format!("Sentry sentry_version=7, sentry_client=sentry.php.laravel/4.15.0, sentry_key={key}"),
        )
        .body(Body::from(body))
        .unwrap()
}

async fn send(app: &TestApp, project: i64, key: &str, event: &Value) {
    let answer = app.request(envelope_request(project, key, envelope(event))).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(answer.json()["id"], event["event_id"]);
}

#[tokio::test]
async fn errors_become_issues_with_their_events_and_trace() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, production, key) = project_with_key(&admin, "orders").await;

    send(&app, project, &key, &error_event("a1b2c3d4e5f60718293a4b5c6d7e8f90", "production", "4f2c1e9", 52))
        .await;
    // The same error from another line of the same function is the same issue.
    send(
        &app,
        project,
        &key,
        &error_event("b1b2c3d4-e5f6-0718-293a-4b5c6d7e8f91", "production", "4f2c1e9", 58),
    )
    .await;

    let issues = admin.get("/api/v1/issues").await.json();
    assert_eq!(issues.as_array().unwrap().len(), 1, "{issues}");
    let issue = &issues[0];
    assert_eq!(issue["title"], "PaymentFailed: Payment for order 42 failed");
    assert_eq!(
        issue["culprit"],
        "App\\Http\\Controllers\\PaymentController::pay in /app/Http/Controllers/PaymentController.php"
    );
    assert_eq!(issue["project_id"], project);
    assert_eq!(issue["environment_id"], production);
    assert_eq!(issue["status"], "unresolved");
    assert_eq!(issue["level"], "error");
    assert_eq!(issue["count"], 2);
    assert_eq!(issue["last_day"], 2);
    assert_eq!(issue["first_release"], "4f2c1e9");
    let id = issue["id"].as_i64().unwrap();

    let detail = admin.get(&format!("/api/v1/issues/{id}")).await.json();
    assert_eq!(detail["hours"].as_array().unwrap().len(), 14 * 24);
    let counted: u64 = detail["hours"].as_array().unwrap().iter().map(|h| h["count"].as_u64().unwrap()).sum();
    assert_eq!(counted, 2);
    assert_eq!(detail["releases"][0]["release"], "4f2c1e9");
    assert_eq!(detail["releases"][0]["count"], 2);
    let latest = &detail["latest"];
    assert_eq!(latest["event_id"], "b1b2c3d4e5f60718293a4b5c6d7e8f91");
    assert_eq!(latest["server_name"], "orders-web-1");
    assert_eq!(latest["transaction"], "POST /orders/{order}/pay");
    assert_eq!(latest["trace_id"], TRACE);
    assert_eq!(latest["exceptions"].as_array().unwrap().len(), 2);
    let frames = latest["exceptions"][0]["frames"].as_array().unwrap();
    assert_eq!(frames[2]["function"], "App\\Payments\\Gateway::charge");
    assert_eq!(frames[2]["lineno"], 58);
    assert_eq!(frames[2]["in_app"], true);
    assert_eq!(latest["breadcrumbs"][0]["category"], "db.sql.query");
    assert_eq!(latest["contexts"]["runtime"]["version"], "8.4.15");

    // What must not be kept: cookies, the body, credentials in headers.
    let kept = latest.to_string();
    for secret in ["laravel_session=abc", "hunter2", "4111111111111111", "secret-token"] {
        assert!(!kept.contains(secret), "{secret} was kept: {kept}");
    }
    let headers: Vec<&str> =
        latest["request"]["headers"].as_array().unwrap().iter().map(|h| h[0].as_str().unwrap()).collect();
    assert!(headers.contains(&"User-Agent"), "{headers:?}");
    assert!(!headers.contains(&"Cookie"));

    let events = admin.get(&format!("/api/v1/issues/{id}/events")).await.json();
    assert_eq!(events.as_array().unwrap().len(), 2);
    assert_eq!(events[0]["service"], "web");
    assert_eq!(events[0]["host"], "docker-host-01");
    let event = admin.get("/api/v1/events/A1B2C3D4-E5F6-0718-293A-4B5C6D7E8F90").await.json();
    assert_eq!(event["issue_id"], id);
    assert_eq!(event["event"]["exceptions"][0]["frames"][2]["lineno"], 52);
    assert_eq!(
        admin.get("/api/v1/events/00000000000000000000000000000000").await.status,
        StatusCode::NOT_FOUND
    );

    // A log entry of the same request shows up with the error in its trace.
    let line = json!({"message": "Charging order 42", "level": "info", "trace_id": TRACE, "environment": "production"});
    let ndjson = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/ingest/logs")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::from(format!("{line}\n")))
        .unwrap();
    assert_eq!(app.request(ndjson).await.status, StatusCode::OK);
    let trace = eventually(&admin, &format!("/api/v1/traces/{TRACE}"), |t| {
        t["logs"].as_array().is_some_and(|l| !l.is_empty())
    })
    .await;
    assert_eq!(trace["logs"][0]["message"], "Charging order 42");
    assert_eq!(trace["events"].as_array().unwrap().len(), 2);
    app.stop().await;
}

#[tokio::test]
async fn resolved_issues_come_back_and_muted_ones_stay_quiet() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    let mut n = 0;
    let mut next = |release: &str| {
        n += 1;
        error_event(&format!("{n:032x}"), "production", release, 52)
    };

    send(&app, project, &key, &next("1.0.0")).await;
    let id = admin.get("/api/v1/issues").await.json()[0]["id"].as_i64().unwrap();
    let path = format!("/api/v1/issues/{id}");

    let resolved = admin.patch(&path, json!({"action": "resolve_next_release"})).await;
    assert_eq!(resolved.status, StatusCode::OK, "{}", resolved.text());
    assert_eq!(resolved.json()["status"], "resolved");
    assert_eq!(resolved.json()["resolved_in_next_release"], "1.0.0");
    assert!(admin.get("/api/v1/issues").await.json().as_array().unwrap().is_empty(), "unresolved only");
    assert_eq!(admin.get("/api/v1/issues?status=resolved").await.json().as_array().unwrap().len(), 1);

    // The old release is still out there: no regression.
    send(&app, project, &key, &next("1.0.0")).await;
    assert_eq!(admin.get(&path).await.json()["issue"]["status"], "resolved");
    // The fix did not work: the next release brings it back.
    send(&app, project, &key, &next("1.0.1")).await;
    let issue = admin.get(&path).await.json()["issue"].clone();
    assert_eq!(issue["status"], "unresolved");
    assert!(issue["regressed_at"].is_string());
    assert_eq!(issue["last_release"], "1.0.1");

    let muted = admin.patch(&path, json!({"action": "mute", "mute_events": 2})).await.json();
    assert_eq!(muted["status"], "muted");
    assert_eq!(muted["muted_remaining"], 2);
    send(&app, project, &key, &next("1.0.1")).await;
    assert_eq!(admin.get(&path).await.json()["issue"]["status"], "muted");
    send(&app, project, &key, &next("1.0.1")).await;
    assert_eq!(
        admin.get(&path).await.json()["issue"]["status"],
        "unresolved",
        "muted until the second event"
    );
    assert_eq!(admin.get(&path).await.json()["issue"]["count"], 5);

    let reopened = admin.patch(&path, json!({"action": "reopen"})).await;
    assert_eq!(reopened.json()["status"], "unresolved");
    let bad = admin.patch(&path, json!({"action": "explode"})).await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(admin.get("/api/v1/issues?status=nonsense").await.status, StatusCode::BAD_REQUEST);
    assert_eq!(admin.get("/api/v1/issues?sort=count&q=payment").await.json().as_array().unwrap().len(), 1);

    let audit = admin.get("/api/v1/audit").await.json();
    let actions: Vec<&str> = audit.as_array().unwrap().iter().filter_map(|e| e["action"].as_str()).collect();
    assert!(actions.contains(&"issue.resolve_next_release"), "{actions:?}");

    assert_eq!(admin.delete(&path).await.status, StatusCode::NO_CONTENT);
    assert_eq!(admin.get(&path).await.status, StatusCode::NOT_FOUND);
    app.stop().await;
}

#[tokio::test]
async fn events_are_held_to_their_keys_stage() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;

    send(&app, project, &key, &error_event("c1b2c3d4e5f60718293a4b5c6d7e8f90", "staging", "1", 52)).await;
    assert!(admin.get("/api/v1/issues?status=all").await.json().as_array().unwrap().is_empty());
    let broken = format!("{{}}\n{}\n[1, 2]\n", json!({"type": "event"}));
    assert_eq!(app.request(envelope_request(project, &key, broken)).await.status, StatusCode::OK);
    let without_id = format!("{{}}\n{}\n{}\n", json!({"type": "event"}), json!({"message": "no id"}));
    assert_eq!(app.request(envelope_request(project, &key, without_id)).await.status, StatusCode::OK);

    let counts = admin.get(&format!("/api/v1/projects/{project}/ingest")).await.json();
    let rejected = &counts[0]["rejected"];
    assert_eq!(rejected["environment"], 1, "{counts}");
    assert_eq!(rejected["invalid"], 2);
    app.stop().await;
}

/// A stand-in SMTP server that takes every message: what it got, one text
/// per message.
async fn smtp_server() -> (u16, tokio::sync::mpsc::UnboundedReceiver<String>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, received) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let sender = sender.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"220 test ESMTP\r\n").await.unwrap();
                let mut message: Option<String> = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(text) = message.as_mut() {
                        if line == "." {
                            let _ = sender.send(message.take().unwrap_or_default());
                            write.write_all(b"250 queued\r\n").await.unwrap();
                        } else {
                            text.push_str(&line);
                            text.push('\n');
                        }
                        continue;
                    }
                    let command = line.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
                    let reply: &[u8] = match command.as_str() {
                        "EHLO" | "HELO" => b"250 test\r\n",
                        "DATA" => {
                            message = Some(String::new());
                            b"354 go on\r\n"
                        }
                        "QUIT" => {
                            let _ = write.write_all(b"221 bye\r\n").await;
                            break;
                        }
                        _ => b"250 ok\r\n",
                    };
                    write.write_all(reply).await.unwrap();
                }
            });
        }
    });
    (port, received)
}

async fn notify(admin: &Browser<'_>, project: i64, addresses: &[&str]) {
    let settings = json!({"settings": {
        "notify": addresses,
        "notify_new_issues": true,
        "notify_regressions": true,
        "notify_monitors": false,
        "redaction": {"extra_keys": [], "allowed_keys": null, "scrub_emails": false, "scrub_ip_addresses": false}
    }});
    let answer = admin.patch(&format!("/api/v1/projects/{project}"), settings).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
}

async fn next_mail(received: &mut tokio::sync::mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(10), received.recv()).await.unwrap().unwrap()
}

#[tokio::test]
async fn new_and_returning_issues_are_mailed_to_the_projects_addresses() {
    let (port, mut received) = smtp_server().await;
    let url = format!("smtp://127.0.0.1:{port}");
    let app =
        TestApp::start_with(&[("LOGDOCK_SMTP_URL", &url), ("LOGDOCK_MAIL_FROM", "logdock@example.com")])
            .await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    notify(&admin, project, &["ops@example.com"]).await;

    let sent = admin.post(&format!("/api/v1/projects/{project}/test-mail"), json!({})).await;
    assert_eq!(sent.json()["recipients"], 1);
    assert!(next_mail(&mut received).await.contains("Test e-mail for orders"));

    send(&app, project, &key, &error_event("d1b2c3d4e5f60718293a4b5c6d7e8f90", "production", "1.0.0", 52))
        .await;
    let mail = next_mail(&mut received).await;
    assert!(mail.contains("Subject: [Logdock] New issue in orders / production"), "{mail}");
    assert!(mail.contains("http://logdock.test/issues/"), "{mail}");
    assert!(mail.contains("POST /orders/{order}/pay"));

    // The same issue again says nothing; once resolved, it does.
    send(&app, project, &key, &error_event("d2b2c3d4e5f60718293a4b5c6d7e8f90", "production", "1.0.0", 52))
        .await;
    let id = admin.get("/api/v1/issues").await.json()[0]["id"].as_i64().unwrap();
    admin.patch(&format!("/api/v1/issues/{id}"), json!({"action": "resolve"})).await;
    send(&app, project, &key, &error_event("d3b2c3d4e5f60718293a4b5c6d7e8f90", "production", "1.0.1", 52))
        .await;
    let mail = next_mail(&mut received).await;
    assert!(mail.contains("Issue came back in orders / production"), "{mail}");
    assert!(received.try_recv().is_err(), "no mail for the second event");
    app.stop().await;
}

#[tokio::test]
async fn the_test_mail_needs_a_server_and_addresses() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, _) = project_with_key(&admin, "orders").await;
    let answer = admin.post(&format!("/api/v1/projects/{project}/test-mail"), json!({})).await;
    assert_eq!(answer.status, StatusCode::CONFLICT);
    assert_eq!(answer.json()["code"], "mail_not_configured");
    app.stop().await;

    // Nothing listens on the discard port.
    let app = TestApp::start_with(&[("LOGDOCK_SMTP_URL", "smtp://127.0.0.1:9")]).await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, _) = project_with_key(&admin, "orders").await;
    let answer = admin.post(&format!("/api/v1/projects/{project}/test-mail"), json!({})).await;
    assert_eq!(answer.json()["code"], "no_recipients");
    let missing = admin.post("/api/v1/projects/999/test-mail", json!({})).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    notify(&admin, project, &["ops@example.com"]).await;
    let failed = admin.post(&format!("/api/v1/projects/{project}/test-mail"), json!({})).await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY);
    assert_eq!(failed.json()["code"], "mail_failed");
    assert!(failed.json()["detail"].as_str().is_some_and(|d| !d.is_empty()));
    app.stop().await;
}

const FIXTURES: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../logdock-sentry/tests/fixtures/sentry-laravel-4.30.0");

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
}

/// What sentry-laravel sent from the test app, sent again as it came:
/// gzipped, with its own envelope headers.
#[tokio::test]
async fn takes_what_sentry_laravel_really_sends() {
    let app = TestApp::start().await;
    let admin = Browser::setup(&app, "ada@example.com").await;
    let (project, _, key) = project_with_key(&admin, "orders").await;
    let post = |body: String| {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, body.as_bytes()).unwrap();
        let mut request = envelope_request(project, &key, String::new());
        *request.body_mut() = Body::from(encoder.finish().unwrap());
        request.headers_mut().insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        request
    };

    let request_event = fixture("event-request.envelope");
    for body in [request_event.clone(), fixture("event-job.envelope"), fixture("logs.envelope")] {
        let answer = app.request(post(body)).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    }
    // An SDK that sends an event twice: it is kept once.
    assert_eq!(app.request(post(request_event.clone())).await.status, StatusCode::OK);
    // The same error after an edit above the route: PHP 8.4 names the
    // closure by its new line, the issue stays the same.
    let edited = request_event
        .replace("web.php:22}", "web.php:30}")
        .replace("626aba60e1bd4f41a112a8b52df2112f", "726aba60e1bd4f41a112a8b52df2112f");
    assert_eq!(app.request(post(edited)).await.status, StatusCode::OK);

    let issues = admin.get("/api/v1/issues?sort=first_seen").await.json();
    assert_eq!(issues.as_array().unwrap().len(), 2, "{issues}");
    let request =
        issues.as_array().unwrap().iter().find(|i| i["title"].as_str().unwrap().contains("42")).unwrap();
    assert_eq!(request["title"], "PaymentFailed: Payment for order 42 failed");
    assert_eq!(request["count"], 2, "the duplicate is not counted, the edited one is");
    assert!(request["culprit"].as_str().unwrap().contains("/routes/web.php"), "{}", request["culprit"]);
    let detail = admin.get(&format!("/api/v1/issues/{}", request["id"])).await.json();
    let latest = &detail["latest"];
    assert_eq!(latest["exceptions"][0]["type"], "RuntimeException");
    assert_eq!(latest["exceptions"][1]["handled"], false);
    assert_eq!(latest["transaction"], "/scenario/fail");
    assert_eq!(latest["release"], "dev-1");
    assert_eq!(latest["request"]["method"], "GET");
    let headers = latest["request"]["headers"].as_array().unwrap();
    assert!(headers.iter().any(|h| h[0] == "user-agent" && h[1] == "node"), "{headers:?}");
    let vendor =
        latest["exceptions"][1]["frames"].as_array().unwrap().iter().find(|f| f["in_app"] == false).unwrap();
    assert!(vendor["pre_context"].as_array().unwrap().is_empty(), "no code around library lines");

    let job =
        issues.as_array().unwrap().iter().find(|i| i["title"].as_str().unwrap().contains("43")).unwrap();
    let job = admin.get(&format!("/api/v1/issues/{}", job["id"])).await.json();
    let tags: Vec<(String, String)> = serde_json::from_value(job["latest"]["tags"].clone()).unwrap();
    assert!(tags.contains(&("service".into(), "horizon".into())), "{tags:?}");
    assert!(job["latest"]["request"].is_null());

    let logs = eventually(&admin, "/api/v1/logs?from=2026-01-01T00:00:00Z", |page| {
        page["entries"].as_array().is_some_and(|e| e.len() == 2)
    })
    .await;
    let slow = &logs["entries"][0];
    assert_eq!(slow["message"], "Payment provider slow");
    assert_eq!(slow["attributes"]["password"], "[redacted]");
    assert!(slow["span_id"].is_null(), "sentry-php's \"null\" is no span: {}", slow["span_id"]);
    assert_eq!(slow["container"], "99f03f3d15d9");
    app.stop().await;
}
