//! Logdock as an MCP server (Model Context Protocol, Streamable HTTP
//! transport): `POST /mcp` with an API token, for Claude and other agents.
//!
//! The tools read through the same API as the interface, with the caller's
//! token: they see and may do exactly what the token may, and answer with
//! the API's JSON, trimmed of what an agent does not need. The server keeps
//! no state between requests and offers no stream (`GET /mcp` is 405).

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use logdock_identity::application::AuthContext;
use logdock_identity::domain::Permission;
use serde_json::{Map, Value, json};
use tower::ServiceExt;

use crate::API_V1;
use crate::auth::{check_same_origin, token_caller};
use crate::dto::{now, time};
use crate::state::ApiState;

/// The protocol versions this server speaks, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// A tool's answer beyond this many characters is cut.
const MAX_TEXT: usize = 200_000;

const INSTRUCTIONS: &str = "Logdock holds the logs, errors and performance data of Laravel apps: \
projects (one per app), each with stages (production, staging, …). Start with list_projects. \
search_logs searches log entries (words, \"phrases\", key:value attributes, trace:<id>); \
list_issues and get_issue show errors grouped into issues, with stack traces and the request; \
get_trace shows everything that carries one trace ID, across containers (the request, the jobs it \
queued, their logs and errors); performance shows where requests, jobs, queries and outgoing \
requests spend their time; list_monitors shows scheduled tasks. Times are UTC; `since` takes \
15m, 6h, 2d or 1w.";

pub(crate) fn router(api: Router) -> Router<ApiState> {
    Router::new().route(
        "/mcp",
        post(move |State(state): State<ApiState>, headers: HeaderMap, body: Bytes| {
            let api = api.clone();
            async move { handle(state, api, headers, body).await }
        })
        .get(not_offered)
        .delete(not_offered),
    )
}

/// No stream from the server, no sessions to end.
async fn not_offered() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

fn unauthorized() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({"code": "unauthenticated", "detail": "send an API token: Authorization: Bearer ldt_…"})),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer realm=\"Logdock\""));
    response
}

async fn handle(state: ApiState, api: Router, headers: HeaderMap, body: Bytes) -> Response {
    // Against DNS rebinding: a browser's request from another site.
    if check_same_origin(&headers).is_err() {
        return (StatusCode::FORBIDDEN, Json(json!({"code": "csrf_rejected"}))).into_response();
    }
    let auth = match token_caller(&headers, &state).await {
        Some(Ok(auth)) => auth,
        _ => return unauthorized(),
    };
    let Some(token) = headers.get(header::AUTHORIZATION).cloned() else { return unauthorized() };
    let server = Server { api, token, auth };
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(error) => {
            return Json(failure(Value::Null, -32700, &format!("parse error: {error}"))).into_response();
        }
    };
    match message {
        Value::Array(batch) => {
            let mut answers = Vec::new();
            for message in batch {
                answers.extend(server.answer(message).await);
            }
            if answers.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                Json(Value::Array(answers)).into_response()
            }
        }
        message => match server.answer(message).await {
            Some(answer) => Json(answer).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One request's caller and the API it reads through.
struct Server {
    api: Router,
    token: HeaderValue,
    auth: AuthContext,
}

impl Server {
    /// The answer to one JSON-RPC message; `None` for notifications and
    /// for answers the client sends.
    async fn answer(&self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return id
                .filter(|_| message.get("result").is_none() && message.get("error").is_none())
                .map(|id| failure(id, -32600, "invalid request: no method"));
        };
        let id = id?;
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": self.tools()})),
            "tools/call" => self.call(&params).await,
            other => Err((-32601, format!("method {other:?} is not offered"))),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => failure(id, code, &message),
        })
    }

    fn may_triage(&self) -> bool {
        self.auth.can(Permission::TriageIssues)
    }

    fn tools(&self) -> Vec<Value> {
        let mut tools = read_tools();
        if self.may_triage() {
            tools.push(change_issue_tool());
        }
        tools
    }

    async fn call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if !self.tools().iter().any(|tool| tool["name"] == name) {
            return Err((-32602, format!("unknown tool {name:?}")));
        }
        let (text, failed) = match self.run(name, &args).await {
            Ok(value) => (value.to_string(), false),
            Err(message) => (message, true),
        };
        let text = if text.len() > MAX_TEXT {
            let mut end = MAX_TEXT;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}… [cut at {MAX_TEXT} characters: ask for less, e.g. a lower limit]", &text[..end])
        } else {
            text
        };
        Ok(json!({"content": [{"type": "text", "text": text}], "isError": failed}))
    }

    async fn run(&self, tool: &str, args: &Value) -> Result<Value, String> {
        match tool {
            "list_projects" => {
                let projects = self.projects().await?;
                Ok(Value::Array(
                    projects
                        .iter()
                        .map(|p| json!({"id": p.id, "name": p.name, "stages": p.stages.iter().map(|(_, n)| n).collect::<Vec<_>>()}))
                        .collect(),
                ))
            }
            "search_logs" => {
                let projects = self.projects().await?;
                let mut query = scope(args, &projects)?;
                query.extend(range(args)?);
                for (arg, param) in
                    [("query", "q"), ("level", "level"), ("service", "service"), ("host", "host")]
                {
                    if let Some(value) = text(args, arg) {
                        query.push((param, value));
                    }
                }
                if let Some(trace) = text(args, "trace_id") {
                    query.push(("trace", trace));
                }
                if let Some(before) = text(args, "before") {
                    query.push(("before", before));
                }
                query.push(("limit", limit(args, 50, 200)));
                let mut page = self.get("/logs", &query).await?;
                name_scopes(&mut page, &projects);
                Ok(compact(page))
            }
            "list_issues" => {
                let projects = self.projects().await?;
                let mut query = scope(args, &projects)?;
                for (arg, param) in [("status", "status"), ("query", "q"), ("sort", "sort")] {
                    if let Some(value) = text(args, arg) {
                        query.push((param, value));
                    }
                }
                query.push(("limit", limit(args, 25, 100)));
                if let Some(offset) = args.get("offset").and_then(Value::as_u64) {
                    query.push(("offset", offset.to_string()));
                }
                let mut issues = self.get("/issues", &query).await?;
                name_scopes(&mut issues, &projects);
                Ok(compact(issues))
            }
            "get_issue" => {
                let id = number(args, "id")?;
                let projects = self.projects().await?;
                let mut detail = self.get(&format!("/issues/{id}"), &[]).await?;
                if let Some(detail) = detail.as_object_mut() {
                    // 84 counts of four hours each: the issue says first and last seen.
                    detail.remove("hours");
                }
                if let Some(latest) = detail.get_mut("latest") {
                    trim_event(latest);
                }
                name_scopes(&mut detail, &projects);
                Ok(compact(detail))
            }
            "get_event" => {
                let event_id = required(args, "event_id")?;
                let projects = self.projects().await?;
                let mut stored = self.get(&format!("/events/{}", encode(&event_id)), &[]).await?;
                if let Some(event) = stored.get_mut("event") {
                    trim_event(event);
                }
                name_scopes(&mut stored, &projects);
                Ok(compact(stored))
            }
            "get_trace" => {
                let trace_id = required(args, "trace_id")?;
                let projects = self.projects().await?;
                let mut trace = self.get(&format!("/traces/{}", encode(&trace_id)), &[]).await?;
                name_scopes(&mut trace, &projects);
                Ok(compact(trace))
            }
            "performance" => {
                let projects = self.projects().await?;
                let mut query = scope(args, &projects)?;
                query.extend(range(args)?);
                query.push(("kind", text(args, "kind").unwrap_or_else(|| "request".into())));
                if let Some(sort) = text(args, "sort") {
                    query.push(("sort", sort));
                }
                query.push(("limit", limit(args, 20, 100)));
                Ok(compact(self.get("/performance", &query).await?))
            }
            "list_monitors" => {
                let projects = self.projects().await?;
                let query = scope(args, &projects)?;
                let mut monitors = self.get("/monitors", &query).await?;
                name_scopes(&mut monitors, &projects);
                Ok(compact(monitors))
            }
            "change_issue" => {
                let id = number(args, "id")?;
                let action = required(args, "action")?;
                let mut body = json!({"action": action});
                for key in ["mute_minutes", "mute_events"] {
                    if let Some(value) = args.get(key).filter(|v| v.is_u64()) {
                        body[key] = value.clone();
                    }
                }
                let projects = self.projects().await?;
                let mut issue = self.send(Method::PATCH, &format!("/issues/{id}"), &[], Some(body)).await?;
                name_scopes(&mut issue, &projects);
                Ok(compact(issue))
            }
            other => Err(format!("unknown tool {other:?}")),
        }
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
        self.send(Method::GET, path, query, None).await
    }

    /// Asks the API as the caller, with their token.
    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value, String> {
        let mut uri = format!("{API_V1}{path}");
        for (n, (key, value)) in query.iter().enumerate() {
            uri.push(if n == 0 { '?' } else { '&' });
            uri.push_str(key);
            uri.push('=');
            uri.push_str(&encode(value));
        }
        let mut request =
            Request::builder().method(method).uri(uri).header(header::AUTHORIZATION, self.token.clone());
        if body.is_some() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .map_err(|e| e.to_string())?;
        let response = self.api.clone().oneshot(request).await.map_err(|e| e.to_string())?;
        let status = response.status();
        let bytes =
            axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024).await.map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(value);
        }
        let why =
            value.get("detail").or_else(|| value.get("code")).and_then(Value::as_str).unwrap_or("no detail");
        Err(format!("Logdock answered {status}: {why}"))
    }

    async fn projects(&self) -> Result<Vec<Project>, String> {
        let projects = self.get("/projects", &[]).await?;
        Ok(projects
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Project {
                id: p["id"].as_i64().unwrap_or_default(),
                name: p["name"].as_str().unwrap_or_default().to_owned(),
                stages: p["environments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        (
                            e["id"].as_i64().unwrap_or_default(),
                            e["name"].as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect(),
            })
            .collect())
    }
}

struct Project {
    id: i64,
    name: String,
    stages: Vec<(i64, String)>,
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&PROTOCOL_VERSIONS[1]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "logdock", "title": "Logdock", "version": logdock_kernel::VERSION},
        "instructions": INSTRUCTIONS,
    })
}

fn text(args: &Value, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn required(args: &Value, key: &str) -> Result<String, String> {
    text(args, key).ok_or_else(|| format!("{key} is missing"))
}

fn number(args: &Value, key: &str) -> Result<i64, String> {
    let value = required(args, key)?;
    value.parse().map_err(|_| format!("{key} {value:?} is no number"))
}

fn limit(args: &Value, default: u64, max: u64) -> String {
    args.get("limit").and_then(Value::as_u64).unwrap_or(default).clamp(1, max).to_string()
}

/// `project` and `stage` (names or IDs) as the API's ID filters. A stage
/// without a project means that stage in every project.
fn scope(args: &Value, projects: &[Project]) -> Result<Vec<(&'static str, String)>, String> {
    let mut query = Vec::new();
    let project = match text(args, "project") {
        Some(wanted) => {
            let found = projects
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(&wanted) || p.id.to_string() == wanted)
                .ok_or_else(|| format!("no project {wanted:?}; list_projects names them"))?;
            query.push(("project", found.id.to_string()));
            Some(found)
        }
        None => None,
    };
    if let Some(stage) = text(args, "stage") {
        let ids: Vec<String> = projects
            .iter()
            .filter(|p| project.is_none_or(|only| only.id == p.id))
            .flat_map(|p| &p.stages)
            .filter(|(id, name)| name.eq_ignore_ascii_case(&stage) || id.to_string() == stage)
            .map(|(id, _)| id.to_string())
            .collect();
        if ids.is_empty() {
            return Err(format!("no stage {stage:?}; list_projects names them"));
        }
        query.push(("environment", ids.join(",")));
    }
    Ok(query)
}

/// `since` (15m, 6h, 2d, 1w) or `from` and `to` (RFC 3339).
fn range(args: &Value) -> Result<Vec<(&'static str, String)>, String> {
    let mut query = Vec::new();
    if let Some(since) = text(args, "since") {
        let ago = duration(&since).ok_or_else(|| format!("since {since:?}: say 15m, 6h, 2d or 1w"))?;
        query.push(("from", time(now().saturating_sub(ago))));
    }
    for key in ["from", "to"] {
        if let Some(value) = text(args, key) {
            query.push((key, value));
        }
    }
    Ok(query)
}

fn duration(text: &str) -> Option<Duration> {
    let (number, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = number.parse().ok()?;
    let seconds = match unit.trim() {
        "s" => 1,
        "m" | "min" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 7 * 86_400,
        _ => return None,
    };
    Some(Duration::from_secs(n.checked_mul(seconds)?))
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Puts project and stage names in place of their IDs, wherever both are.
fn name_scopes(value: &mut Value, projects: &[Project]) {
    match value {
        Value::Object(object) => {
            if let (Some(project), Some(stage)) = (
                object.get("project_id").and_then(Value::as_i64),
                object.get("environment_id").and_then(Value::as_i64),
            ) && let Some(found) = projects.iter().find(|p| p.id == project)
            {
                let stage = found.stages.iter().find(|(id, _)| *id == stage).map(|(_, name)| name.clone());
                object.shift_remove("project_id");
                object.shift_remove("environment_id");
                object.insert("project".into(), Value::String(found.name.clone()));
                object.insert("stage".into(), stage.map_or(Value::Null, Value::String));
            }
            object.values_mut().for_each(|v| name_scopes(v, projects));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| name_scopes(v, projects)),
        _ => {}
    }
}

/// An event as an agent reads it: the frames of libraries without their
/// code, the last breadcrumbs only.
fn trim_event(event: &mut Value) {
    let Some(event) = event.as_object_mut() else { return };
    let trim_frames = |frames: &mut Value| {
        for frame in frames.as_array_mut().into_iter().flatten() {
            if frame["in_app"] == false
                && let Some(frame) = frame.as_object_mut()
            {
                for key in ["pre_context", "post_context", "context_line", "abs_path", "colno"] {
                    frame.remove(key);
                }
            }
        }
    };
    if let Some(frames) = event.get_mut("frames") {
        trim_frames(frames);
    }
    for exception in event.get_mut("exceptions").and_then(Value::as_array_mut).into_iter().flatten() {
        if let Some(frames) = exception.get_mut("frames") {
            trim_frames(frames);
        }
    }
    if let Some(breadcrumbs) = event.get_mut("breadcrumbs").and_then(Value::as_array_mut) {
        let keep = 30;
        if breadcrumbs.len() > keep {
            breadcrumbs.drain(..breadcrumbs.len() - keep);
        }
    }
}

/// Without nulls, empty lists and objects, and bookkeeping an agent has no
/// use for.
fn compact(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut kept = Map::new();
            for (key, value) in object {
                if matches!(key.as_str(), "received" | "redactions" | "span_id" | "template")
                    || (key == "truncated" && value == false)
                {
                    continue;
                }
                let value = compact(value);
                let empty = match &value {
                    Value::Null => true,
                    Value::Array(items) => items.is_empty(),
                    Value::Object(object) => object.is_empty(),
                    _ => false,
                };
                if !empty {
                    kept.insert(key, value);
                }
            }
            Value::Object(kept)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(compact).collect()),
        other => other,
    }
}

fn scope_properties() -> Map<String, Value> {
    let mut properties = Map::new();
    properties.insert(
        "project".into(),
        json!({"type": "string", "description": "A project's name or ID; all when left out."}),
    );
    properties.insert(
        "stage".into(),
        json!({"type": "string", "description": "A stage's name, such as production; all when left out."}),
    );
    properties
}

fn range_properties(properties: &mut Map<String, Value>) {
    properties.insert(
        "since".into(),
        json!({"type": "string", "description": "How far back: 15m, 6h, 2d or 1w. Or use from and to."}),
    );
    properties
        .insert("from".into(), json!({"type": "string", "description": "From this time on, RFC 3339."}));
    properties.insert("to".into(), json!({"type": "string", "description": "Before this time, RFC 3339."}));
}

fn tool(
    name: &str,
    title: &str,
    description: &str,
    properties: Map<String, Value>,
    required: &[&str],
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
        "annotations": {"readOnlyHint": true, "openWorldHint": false},
    })
}

fn read_tools() -> Vec<Value> {
    let mut logs = scope_properties();
    range_properties(&mut logs);
    logs.insert(
        "query".into(),
        json!({"type": "string", "description": "Words, \"phrases\", prefix*, key:value for attributes (order_id:42), level:, service:, host:."}),
    );
    logs.insert(
        "level".into(),
        json!({"type": "string", "description": "Levels, comma-separated: debug, info, warn, error."}),
    );
    logs.insert(
        "service".into(),
        json!({"type": "string", "description": "The container's service: web, horizon, scheduler, …"}),
    );
    logs.insert("host".into(), json!({"type": "string", "description": "The Docker host."}));
    logs.insert(
        "trace_id".into(),
        json!({"type": "string", "description": "Only the entries of this trace."}),
    );
    logs.insert(
        "limit".into(),
        json!({"type": "integer", "description": "Entries, newest first: 50 by default, at most 200."}),
    );
    logs.insert(
        "before".into(),
        json!({"type": "string", "description": "The `next` of the previous answer, for older entries."}),
    );

    let mut issues = scope_properties();
    issues.insert("status".into(), json!({"type": "string", "enum": ["unresolved", "resolved", "muted", "all"], "description": "unresolved by default."}));
    issues.insert("query".into(), json!({"type": "string", "description": "Words in the title or culprit."}));
    issues.insert("sort".into(), json!({"type": "string", "enum": ["last_seen", "first_seen", "count"]}));
    issues.insert("limit".into(), json!({"type": "integer", "description": "25 by default, at most 100."}));
    issues.insert("offset".into(), json!({"type": "integer"}));

    let mut performance = scope_properties();
    range_properties(&mut performance);
    performance.insert(
        "kind".into(),
        json!({"type": "string", "enum": ["request", "job", "command", "other", "query", "outgoing", "cache"], "description": "request by default; query are SQL statements, outgoing HTTP requests by host."}),
    );
    performance.insert(
        "sort".into(),
        json!({"type": "string", "enum": ["total_time", "count", "slowest", "failures"]}),
    );
    performance
        .insert("limit".into(), json!({"type": "integer", "description": "20 by default, at most 100."}));

    let mut id = Map::new();
    id.insert("id".into(), json!({"type": "integer", "description": "The issue's ID."}));
    let mut event = Map::new();
    event.insert("event_id".into(), json!({"type": "string"}));
    let mut trace = Map::new();
    trace.insert("trace_id".into(), json!({"type": "string", "description": "32 hex characters, from a log entry, an event or a transaction."}));

    vec![
        tool(
            "list_projects",
            "Projects and stages",
            "The projects (one per app) and their stages; the names the other tools take.",
            Map::new(),
            &[],
        ),
        tool(
            "search_logs",
            "Search log entries",
            "Log entries of all apps and containers, newest first, with their attributes and trace ID. Narrow by project, stage, time, level, service and words.",
            logs,
            &[],
        ),
        tool(
            "list_issues",
            "List issues",
            "Errors grouped into issues: title, culprit, how often, first and last seen, status and releases.",
            issues,
            &[],
        ),
        tool(
            "get_issue",
            "Show an issue",
            "One issue with its latest event: the exception chain, stack frames (the app's own with their code), the request, tags, breadcrumbs and trace ID.",
            id,
            &["id"],
        ),
        tool(
            "get_event",
            "Show an event",
            "One error event by its event ID, as get_issue shows the latest.",
            event,
            &["event_id"],
        ),
        tool(
            "get_trace",
            "Show a trace",
            "Everything that carries one trace ID, across containers: transactions with their spans (queries, outgoing requests, cache), log entries and error events.",
            trace,
            &["trace_id"],
        ),
        tool(
            "performance",
            "Where time goes",
            "Requests, jobs, commands, SQL statements, outgoing requests or cache operations over a range (24 hours by default): count, failures, p50, p95, p99 and total time.",
            performance,
            &[],
        ),
        tool(
            "list_monitors",
            "Scheduled tasks",
            "Scheduled tasks by their check-ins: health (ok, failing, missed), last run and next run.",
            scope_properties(),
            &[],
        ),
    ]
}

fn change_issue_tool() -> Value {
    let mut properties = Map::new();
    properties.insert("id".into(), json!({"type": "integer", "description": "The issue's ID."}));
    properties.insert(
        "action".into(),
        json!({"type": "string", "enum": ["resolve", "resolve_next_release", "mute", "reopen"], "description": "resolve_next_release: events of the release seen last do not bring it back, a later release does."}),
    );
    properties.insert(
        "mute_minutes".into(),
        json!({"type": "integer", "description": "With mute: for this long."}),
    );
    properties.insert(
        "mute_events".into(),
        json!({"type": "integer", "description": "With mute: until this many more events came."}),
    );
    let mut tool = tool(
        "change_issue",
        "Resolve, mute or reopen an issue",
        "Changes an issue's state, e.g. resolve_next_release after a fix is deployed.",
        properties,
        &["id", "action"],
    );
    tool["annotations"] = json!({"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false});
    tool
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_relative_times() {
        assert_eq!(duration("15m"), Some(Duration::from_secs(900)));
        assert_eq!(duration("6h"), Some(Duration::from_secs(21_600)));
        assert_eq!(duration("2d"), Some(Duration::from_secs(172_800)));
        assert_eq!(duration("1w"), Some(Duration::from_secs(604_800)));
        for bad in ["", "h", "5y", "99999999999999999999h"] {
            assert_eq!(duration(bad), None, "{bad}");
        }
        assert!(range(&json!({"since": "soon"})).is_err());
        let from_to = range(&json!({"from": "2026-11-18T00:00:00Z", "to": "2026-11-19T00:00:00Z"})).unwrap();
        assert_eq!(from_to.len(), 2);
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode("order_id:42 \"x\"&y"), "order_id%3A42%20%22x%22%26y");
        assert_eq!(encode("ä"), "%C3%A4");
    }

    #[test]
    fn finds_projects_and_stages_by_name() {
        let projects = vec![
            Project {
                id: 1,
                name: "orders".into(),
                stages: vec![(10, "production".into()), (11, "qa".into())],
            },
            Project { id: 2, name: "billing".into(), stages: vec![(20, "production".into())] },
        ];
        let both = scope(&json!({"stage": "production"}), &projects).unwrap();
        assert_eq!(both, [("environment", "10,20".to_owned())]);
        let one = scope(&json!({"project": "Orders", "stage": "production"}), &projects).unwrap();
        assert_eq!(one, [("project", "1".to_owned()), ("environment", "10".to_owned())]);
        assert!(scope(&json!({"project": "shop"}), &projects).unwrap_err().contains("list_projects"));
        assert!(scope(&json!({"stage": "dev"}), &projects).is_err());

        let mut entry = json!({"entries": [{"project_id": 1, "environment_id": 11, "message": "m"}]});
        name_scopes(&mut entry, &projects);
        assert_eq!(entry, json!({"entries": [{"message": "m", "project": "orders", "stage": "qa"}]}));
    }

    #[test]
    fn leaves_out_what_an_agent_does_not_need() {
        let value = json!({"a": null, "b": [], "c": {}, "received": "x", "truncated": false, "d": [{"e": null, "f": 1}]});
        assert_eq!(compact(value), json!({"d": [{"f": 1}]}));
        let mut event = json!({
            "frames": [{"in_app": false, "context_line": "vendor", "function": "f"}, {"in_app": true, "context_line": "app"}],
            "breadcrumbs": (0..40).collect::<Vec<_>>(),
        });
        trim_event(&mut event);
        assert_eq!(event["frames"][0], json!({"in_app": false, "function": "f"}));
        assert_eq!(event["frames"][1]["context_line"], "app");
        assert_eq!(event["breadcrumbs"].as_array().unwrap().len(), 30);
        assert_eq!(event["breadcrumbs"][0], 10, "the last ones");
    }
}
