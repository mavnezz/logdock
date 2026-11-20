//! The ingest endpoints (docs/architecture.md, 3): Sentry's envelope
//! endpoint for the SDKs, and NDJSON for forwarders. Neither is part of the
//! interface's API, and neither uses sessions: a key lets data in.

mod findings;
mod ndjson;
mod sentry;
mod value;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use logdock_issues::application::{IssueError, NotifyPolicy};
use logdock_kernel::{PortError, ProjectId};
use logdock_logs::application::IngestError;
use logdock_monitors::application::MonitorError;
use logdock_performance::application::PerformanceError;
use logdock_performance::domain::{Transaction, findings as performance_findings};
use logdock_projects::application::{IngestScope, Stage, StageFor};
use logdock_projects::domain::{Rejection, Tally};
use logdock_sentry::{Envelope, LogItems};
use serde_json::json;
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;

use crate::extract::ApiPath as Path;
use crate::state::ApiState;

/// How much ingest takes (docs/architecture.md, 3.5).
#[derive(Debug, Clone, Copy)]
pub struct IngestConfig {
    /// A request body as sent, compressed or not.
    pub max_body: usize,
    /// A request body after decompression.
    pub max_decompressed: usize,
    /// Items per second and key, and how many at once.
    pub rate: f64,
    pub burst: f64,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self { max_body: 1024 * 1024, max_decompressed: 20 * 1024 * 1024, rate: 1000.0, burst: 5000.0 }
    }
}

/// What ingest keeps between requests: a token bucket per key, and the
/// readiness of the disk, asked at most every few seconds.
#[derive(Debug, Default)]
pub(crate) struct IngestGuard {
    buckets: Mutex<HashMap<i64, (f64, Instant)>>,
    disk: Mutex<Option<(bool, Instant)>>,
    findings: findings::Reported,
}

const DISK_CHECK_EVERY: Duration = Duration::from_secs(5);

impl IngestGuard {
    /// Takes `items` from the key's bucket; `Err` with when to come back.
    fn take(&self, key: i64, items: f64, config: &IngestConfig) -> Result<(), Duration> {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let (tokens, at) = buckets.entry(key).or_insert((config.burst, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * config.rate).min(config.burst);
        *at = now;
        if *tokens >= items || *tokens >= config.burst {
            *tokens -= items;
            Ok(())
        } else {
            let missing = items.min(config.burst) - *tokens;
            Err(Duration::from_secs_f64((missing / config.rate.max(1.0)).max(1.0)))
        }
    }

    async fn disk_full(&self, state: &ApiState) -> bool {
        let cached = *self.disk.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((full, at)) = cached
            && at.elapsed() < DISK_CHECK_EVERY
        {
            return full;
        }
        let full = state.retention.check_readiness.execute().await.disk_full();
        *self.disk.lock().unwrap_or_else(|e| e.into_inner()) = Some((full, Instant::now()));
        full
    }
}

pub(crate) fn router(state: &ApiState) -> Router<ApiState> {
    let config = state.ingest;
    Router::new()
        .route("/api/{project}/envelope/", post(envelope))
        .route("/api/{project}/envelope", post(envelope))
        .route("/api/v1/ingest/logs", post(ndjson))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestDecompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(config.max_body))
}

/// A refusal as SDKs understand it: the status, and for 429 and 503 when
/// to come back, in `Retry-After` and Sentry's `X-Sentry-Rate-Limits`.
fn refuse(status: StatusCode, code: &str, retry_after: Option<Duration>) -> Response {
    let mut response = (status, Json(json!({ "code": code }))).into_response();
    if let Some(after) = retry_after {
        let seconds = after.as_secs().max(1);
        let headers = response.headers_mut();
        headers.insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        if let Ok(limits) = HeaderValue::from_str(&format!("{seconds}::key")) {
            headers.insert("x-sentry-rate-limits", limits);
        }
    }
    response
}

/// Reads the body up to the decompressed limit; `None` when it is larger.
async fn body(body: Body, max: usize) -> Option<Bytes> {
    axum::body::to_bytes(body, max).await.ok()
}

fn too_large() -> Response {
    refuse(StatusCode::PAYLOAD_TOO_LARGE, "too_large", None)
}

/// Why [`admit`] refused, and how to answer.
struct Refusal {
    reason: Rejection,
    status: StatusCode,
    code: &'static str,
    retry_after: Duration,
}

impl Refusal {
    fn response(&self) -> Response {
        refuse(self.status, self.code, Some(self.retry_after))
    }
}

/// Checks the disk and the key's rate before anything is written.
async fn admit(state: &ApiState, scope: &IngestScope, items: usize) -> Result<(), Refusal> {
    if state.ingest_guard.disk_full(state).await {
        return Err(Refusal {
            reason: Rejection::Disk,
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "low_disk",
            retry_after: Duration::from_secs(60),
        });
    }
    if let Err(after) = state.ingest_guard.take(scope.key.get(), items as f64, &state.ingest) {
        return Err(Refusal {
            reason: Rejection::Rate,
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate_limited",
            retry_after: after,
        });
    }
    Ok(())
}

/// The stages one request's data names, each asked once.
struct Stages<'a> {
    state: &'a ApiState,
    scope: &'a IngestScope,
    known: HashMap<Option<String>, StageFor>,
}

impl<'a> Stages<'a> {
    fn new(state: &'a ApiState, scope: &'a IngestScope) -> Self {
        Self { state, scope, known: HashMap::new() }
    }

    /// Sorts entries by the stage their `environment` names, in the order
    /// the stages first appear; entries the key may not send are counted
    /// as refused.
    async fn sort<T>(
        &mut self,
        entries: Vec<(T, Option<String>)>,
        tally: &mut Tally,
    ) -> Result<Vec<(Stage, Vec<T>)>, PortError> {
        let mut sorted: Vec<(Stage, Vec<T>)> = Vec::new();
        for (entry, claimed) in entries {
            let claimed = claimed.map(|name| name.trim().to_ascii_lowercase());
            let answer = match self.known.get(&claimed) {
                Some(answer) => answer.clone(),
                None => {
                    let resolve = &self.state.projects.resolve_stage;
                    let answer = resolve.execute(self.scope, claimed.as_deref()).await?;
                    self.known.insert(claimed, answer.clone());
                    answer
                }
            };
            match answer {
                StageFor::Stage(stage) => match sorted.iter_mut().find(|(s, _)| s.scope == stage.scope) {
                    Some((_, list)) => list.push(entry),
                    None => sorted.push((stage, vec![entry])),
                },
                StageFor::Refused(why) => tally.reject(Rejection::Environment, 1, why),
            }
        }
        Ok(sorted)
    }
}

fn count<T>(sorted: &[(Stage, Vec<T>)]) -> usize {
    sorted.iter().map(|(_, list)| list.len()).sum()
}

fn unavailable() -> Response {
    refuse(StatusCode::SERVICE_UNAVAILABLE, "unavailable", Some(Duration::from_secs(30)))
}

/// Turns what a transaction shows of the project's performance rules into
/// issues. A failure here costs the issue, never the transaction.
async fn report_findings(state: &ApiState, scope: &IngestScope, stage: &Stage, transaction: &Transaction) {
    for finding in performance_findings(transaction, &scope.settings.performance) {
        let key = format!("{}\u{1f}{}", stage.scope.environment.get(), finding.fingerprint().join("\u{1f}"));
        if !state.ingest_guard.findings.take(key) {
            continue;
        }
        let payload = value::value(findings::event(transaction, &finding, stage.name.as_str()));
        let policy = notify_policy(scope, stage);
        let rules = &scope.settings.redaction;
        if let Err(error) = state.issues.ingest.execute(stage.scope, rules, &policy, payload, None).await {
            tracing::warn!(%error, kind = finding.kind.as_str(), "cannot keep a performance issue");
        }
    }
}

/// Who hears of the stage's issues, from the project's settings.
fn notify_policy(scope: &IngestScope, stage: &Stage) -> NotifyPolicy {
    let settings = &scope.settings;
    NotifyPolicy {
        recipients: settings.notify.iter().map(|a| a.as_str().to_owned()).collect(),
        new_issues: settings.notify_new_issues,
        regressions: settings.notify_regressions,
        project: scope.project_name.to_string(),
        environment: stage.name.to_string(),
    }
}

/// `POST /api/<project>/envelope/`: what Sentry's SDKs send.
async fn envelope(
    State(state): State<ApiState>,
    Path(project): Path<i64>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    request_body: Body,
) -> Response {
    let project = ProjectId::new(project);
    let record = &state.projects.record_ingest;
    let Some(bytes) = body(request_body, state.ingest.max_decompressed).await else {
        record.refused(project, Rejection::Size, "the envelope is too large").await;
        return too_large();
    };
    let envelope = match Envelope::parse(&bytes, logdock_sentry::Limits::default()) {
        Ok(envelope) => envelope,
        Err(error) => {
            record.refused(project, Rejection::Invalid, &error.to_string()).await;
            return refuse(StatusCode::BAD_REQUEST, "invalid_envelope", None);
        }
    };
    let key = headers
        .get("x-sentry-auth")
        .and_then(|value| value.to_str().ok())
        .and_then(logdock_sentry::key_from_auth_header)
        .or_else(|| query.as_deref().and_then(logdock_sentry::key_from_query))
        .or_else(|| envelope.dsn().and_then(logdock_sentry::key_from_dsn));
    let scope = match key {
        Some(key) => match state.projects.authenticate_key.execute(Some(project), &key).await {
            Ok(scope) => scope,
            Err(error) => {
                tracing::error!(%error, "cannot check an ingest key");
                return unavailable();
            }
        },
        None => None,
    };
    let Some(scope) = scope else {
        record.refused(project, Rejection::Auth, "no valid key for this project").await;
        return refuse(StatusCode::FORBIDDEN, "invalid_key", None);
    };

    let mut tally = Tally::default();
    let mut logs = Vec::new();
    let mut events = Vec::new();
    let mut transactions = Vec::new();
    let mut check_ins = Vec::new();
    for item in &envelope.items {
        match item.kind.as_str() {
            kind @ ("transaction" | "check_in") => match item.json() {
                Ok(payload @ serde_json::Value::Object(_)) => {
                    let environment = payload.get("environment").and_then(value::text);
                    let list = if kind == "transaction" { &mut transactions } else { &mut check_ins };
                    list.push((payload, environment));
                }
                Ok(_) => tally.reject(Rejection::Invalid, 1, format!("a {kind} is no JSON object")),
                Err(error) => tally.reject(Rejection::Invalid, 1, format!("a {kind} is no JSON: {error}")),
            },
            "event" => match item.json() {
                Ok(event @ serde_json::Value::Object(_)) => {
                    let environment = event.get("environment").and_then(value::text);
                    events.push((event, environment));
                }
                Ok(_) => tally.reject(Rejection::Invalid, 1, "an event is no JSON object"),
                Err(error) => tally.reject(Rejection::Invalid, 1, format!("an event is no JSON: {error}")),
            },
            "log" => match LogItems::parse(&item.payload) {
                Ok((items, broken)) => {
                    if broken > 0 {
                        tally.reject(Rejection::Invalid, broken as u64, "log entries that are no objects");
                    }
                    logs.extend(items.items.into_iter().map(sentry::entry));
                }
                Err(error) => tally.reject(Rejection::Invalid, 1, format!("a log item is no JSON: {error}")),
            },
            _ => tally.skipped += 1,
        }
    }
    let mut stages = Stages::new(&state, &scope);
    let sorted = async {
        Ok::<_, PortError>((
            stages.sort(logs, &mut tally).await?,
            stages.sort(events, &mut tally).await?,
            stages.sort(transactions, &mut tally).await?,
            stages.sort(check_ins, &mut tally).await?,
        ))
    };
    let (logs, events, transactions, check_ins) = match sorted.await {
        Ok(sorted) => sorted,
        Err(error) => {
            tracing::error!(%error, "cannot find the stage of ingested data");
            return unavailable();
        }
    };
    let items = (count(&logs) + count(&events) + count(&transactions) + count(&check_ins)).max(1);
    if let Err(refusal) = admit(&state, &scope, items).await {
        tally.reject(refusal.reason, items as u64, "refused before writing");
        record.execute(project, Some(scope.key), &tally).await;
        return refusal.response();
    }
    let rules = &scope.settings.redaction;
    for (stage, logs) in logs {
        let count = logs.len() as u64;
        match state.logs.ingest.execute(stage.scope, rules, logs, false).await {
            Ok(ingested) => tally.accepted += ingested.accepted,
            Err(IngestError::Overloaded) => {
                tally.reject(Rejection::Overloaded, count, "the writer's queue is full");
                record.execute(project, Some(scope.key), &tally).await;
                return refuse(StatusCode::TOO_MANY_REQUESTS, "overloaded", Some(Duration::from_secs(10)));
            }
            Err(IngestError::Port(error)) => {
                tracing::error!(%error, "cannot take log entries");
                return unavailable();
            }
        }
    }
    let id = envelope.event_id().map(str::to_owned);
    for (stage, events) in events {
        let policy = notify_policy(&scope, &stage);
        for event in events {
            let payload = value::value(event);
            match state.issues.ingest.execute(stage.scope, rules, &policy, payload, id.as_deref()).await {
                Ok(_) => tally.accepted += 1,
                Err(IssueError::Port(error)) => {
                    tracing::error!(%error, "cannot keep an error event");
                    record.execute(project, Some(scope.key), &tally).await;
                    return unavailable();
                }
                Err(error) => tally.reject(Rejection::Invalid, 1, error.to_string()),
            }
        }
    }
    for (stage, transactions) in transactions {
        for transaction in transactions {
            let payload = value::value(transaction);
            match state.performance.ingest.execute(stage.scope, rules, payload, envelope.sample_rate()).await
            {
                Ok(transaction) => {
                    tally.accepted += 1;
                    report_findings(&state, &scope, &stage, &transaction).await;
                }
                Err(PerformanceError::Port(error)) => {
                    tracing::error!(%error, "cannot keep a transaction");
                    record.execute(project, Some(scope.key), &tally).await;
                    return unavailable();
                }
                Err(error) => tally.reject(Rejection::Invalid, 1, error.to_string()),
            }
        }
    }
    for (stage, check_ins) in check_ins {
        for check_in in check_ins {
            match state.monitors.ingest.execute(stage.scope, &value::value(check_in)).await {
                Ok(_) => tally.accepted += 1,
                Err(MonitorError::Port(error)) => {
                    tracing::error!(%error, "cannot keep a check-in");
                    record.execute(project, Some(scope.key), &tally).await;
                    return unavailable();
                }
                Err(error) => tally.reject(Rejection::Invalid, 1, error.to_string()),
            }
        }
    }
    record.execute(project, Some(scope.key), &tally).await;
    let id = id.unwrap_or_default();
    (StatusCode::OK, Json(json!({ "id": id }))).into_response()
}

/// `POST /api/v1/ingest/logs`: one JSON object per line, from forwarders.
/// Answers after the commit, so a forwarder that gets 2xx may forget.
async fn ndjson(State(state): State<ApiState>, headers: HeaderMap, request_body: Body) -> Response {
    let key = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")))
        .map(str::trim);
    let scope = match key {
        Some(key) => match state.projects.authenticate_key.execute(None, key).await {
            Ok(scope) => scope,
            Err(error) => {
                tracing::error!(%error, "cannot check an ingest key");
                return unavailable();
            }
        },
        None => None,
    };
    let Some(scope) = scope else {
        return refuse(StatusCode::UNAUTHORIZED, "invalid_key", None);
    };
    let project = scope.project;
    let record = &state.projects.record_ingest;
    let Some(bytes) = body(request_body, state.ingest.max_decompressed).await else {
        let tally = Tally::rejected(Rejection::Size, 1, "the body is too large");
        record.execute(project, Some(scope.key), &tally).await;
        return too_large();
    };
    let mut tally = Tally::default();
    let mut entries = Vec::new();
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.iter().all(u8::is_ascii_whitespace)) {
        match ndjson::entry(line) {
            Some(entry) => entries.push(entry),
            None => tally.reject(Rejection::Invalid, 1, "a line is no JSON object with a message"),
        }
    }
    let invalid = tally.rejected.iter().map(|(_, n)| n).sum::<u64>();
    let entries = match Stages::new(&state, &scope).sort(entries, &mut tally).await {
        Ok(sorted) => sorted,
        Err(error) => {
            tracing::error!(%error, "cannot find the stage of ingested data");
            return unavailable();
        }
    };
    if entries.is_empty() {
        record.execute(project, Some(scope.key), &tally).await;
        let status = if invalid > 0 { StatusCode::BAD_REQUEST } else { StatusCode::OK };
        return (
            status,
            Json(json!({ "accepted": 0, "rejected": tally.rejected.iter().map(|(_, n)| n).sum::<u64>() })),
        )
            .into_response();
    }
    if let Err(refusal) = admit(&state, &scope, count(&entries)).await {
        tally.reject(refusal.reason, count(&entries) as u64, "refused before writing");
        record.execute(project, Some(scope.key), &tally).await;
        // Forwarders retry both 429 and 503.
        return refusal.response();
    }
    for (stage, entries) in entries {
        let count = entries.len() as u64;
        match state.logs.ingest.execute(stage.scope, &scope.settings.redaction, entries, true).await {
            Ok(ingested) => tally.accepted += ingested.accepted,
            Err(IngestError::Overloaded) => {
                tally.reject(Rejection::Overloaded, count, "the writer's queue is full");
                record.execute(project, Some(scope.key), &tally).await;
                return refuse(StatusCode::SERVICE_UNAVAILABLE, "overloaded", Some(Duration::from_secs(10)));
            }
            Err(IngestError::Port(error)) => {
                tracing::error!(%error, "cannot write log entries");
                return unavailable();
            }
        }
    }
    record.execute(project, Some(scope.key), &tally).await;
    let rejected: u64 = tally.rejected.iter().map(|(_, n)| n).sum();
    (StatusCode::OK, Json(json!({ "accepted": tally.accepted, "rejected": rejected }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bucket_refills_with_time_and_lets_a_burst_through() {
        let guard = IngestGuard::default();
        let config = IngestConfig { rate: 10.0, burst: 20.0, ..IngestConfig::default() };
        assert!(guard.take(1, 15.0, &config).is_ok());
        let after = guard.take(1, 15.0, &config).unwrap_err();
        assert!(after >= Duration::from_secs(1));
        assert!(guard.take(2, 15.0, &config).is_ok(), "each key has its own bucket");
        // One batch larger than the burst is let through when the bucket is
        // full, so no batch is refused forever.
        assert!(guard.take(3, 100.0, &config).is_ok());
    }

    #[test]
    fn refusals_say_when_to_come_back() {
        let response =
            refuse(StatusCode::TOO_MANY_REQUESTS, "rate_limited", Some(Duration::from_millis(1500)));
        assert_eq!(response.headers()[header::RETRY_AFTER], "1");
        assert_eq!(response.headers()["x-sentry-rate-limits"], "1::key");
        assert!(
            refuse(StatusCode::FORBIDDEN, "invalid_key", None).headers().get(header::RETRY_AFTER).is_none()
        );
    }
}
