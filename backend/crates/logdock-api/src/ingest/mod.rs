//! The ingest endpoints (docs/architecture.md, 3): Sentry's envelope
//! endpoint for the SDKs, and NDJSON for forwarders. Neither is part of the
//! interface's API, and neither uses sessions: a key lets data in.

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
use logdock_kernel::ProjectId;
use logdock_logs::application::IngestError;
use logdock_logs::domain::NewLogEntry;
use logdock_projects::application::IngestScope;
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

/// Entries whose claimed stage is another than the key's are refused.
fn own_stage(
    scope: &IngestScope,
    entries: Vec<(NewLogEntry, Option<String>)>,
    tally: &mut Tally,
) -> Vec<NewLogEntry> {
    let mut kept = Vec::with_capacity(entries.len());
    for (entry, environment) in entries {
        match environment {
            Some(claimed) if !claimed.eq_ignore_ascii_case(scope.environment_name.as_str()) => {
                tally.reject(
                    Rejection::Environment,
                    1,
                    format!(
                        "data for stage {claimed:?} sent with a key of {:?}",
                        scope.environment_name.as_str()
                    ),
                );
            }
            _ => kept.push(entry),
        }
    }
    kept
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
                return refuse(StatusCode::SERVICE_UNAVAILABLE, "unavailable", Some(Duration::from_secs(30)));
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
    for item in &envelope.items {
        match item.kind.as_str() {
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
    let logs = own_stage(&scope, logs, &mut tally);
    if let Err(refusal) = admit(&state, &scope, logs.len().max(1)).await {
        tally.reject(refusal.reason, (logs.len() as u64).max(1), "refused before writing");
        record.execute(project, Some(scope.key), &tally).await;
        return refusal.response();
    }
    if !logs.is_empty() {
        let count = logs.len() as u64;
        match state.logs.ingest.execute(scope.scope, &scope.settings.redaction, logs, false).await {
            Ok(ingested) => tally.accepted += ingested.accepted,
            Err(IngestError::Overloaded) => {
                tally.reject(Rejection::Overloaded, count, "the writer's queue is full");
                record.execute(project, Some(scope.key), &tally).await;
                return refuse(StatusCode::TOO_MANY_REQUESTS, "overloaded", Some(Duration::from_secs(10)));
            }
            Err(IngestError::Port(error)) => {
                tracing::error!(%error, "cannot take log entries");
                return refuse(StatusCode::SERVICE_UNAVAILABLE, "unavailable", Some(Duration::from_secs(30)));
            }
        }
    }
    record.execute(project, Some(scope.key), &tally).await;
    let id = envelope.event_id().unwrap_or_default().to_owned();
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
                return refuse(StatusCode::SERVICE_UNAVAILABLE, "unavailable", Some(Duration::from_secs(30)));
            }
        },
        None => None,
    };
    let Some(scope) = scope else {
        return refuse(StatusCode::UNAUTHORIZED, "invalid_key", None);
    };
    let project = scope.scope.project;
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
    let entries = own_stage(&scope, entries, &mut tally);
    if entries.is_empty() {
        record.execute(project, Some(scope.key), &tally).await;
        let status = if invalid > 0 { StatusCode::BAD_REQUEST } else { StatusCode::OK };
        return (
            status,
            Json(json!({ "accepted": 0, "rejected": tally.rejected.iter().map(|(_, n)| n).sum::<u64>() })),
        )
            .into_response();
    }
    if let Err(refusal) = admit(&state, &scope, entries.len()).await {
        tally.reject(refusal.reason, entries.len() as u64, "refused before writing");
        record.execute(project, Some(scope.key), &tally).await;
        // Forwarders retry both 429 and 503.
        return refusal.response();
    }
    let count = entries.len() as u64;
    match state.logs.ingest.execute(scope.scope, &scope.settings.redaction, entries, true).await {
        Ok(ingested) => tally.accepted += ingested.accepted,
        Err(IngestError::Overloaded) => {
            tally.reject(Rejection::Overloaded, count, "the writer's queue is full");
            record.execute(project, Some(scope.key), &tally).await;
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "overloaded", Some(Duration::from_secs(10)));
        }
        Err(IngestError::Port(error)) => {
            tracing::error!(%error, "cannot write log entries");
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "unavailable", Some(Duration::from_secs(30)));
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
