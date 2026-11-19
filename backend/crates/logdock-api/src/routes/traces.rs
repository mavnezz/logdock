//! A trace with everything that carries it: transactions and their spans,
//! log entries and error events.

use axum::Json;
use axum::extract::State;
use logdock_identity::domain::Permission;
use logdock_logs::domain::{LogQuery, SearchText};
use logdock_performance::application::ports::StoredTransaction;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::issues::EventListItem;
use super::logs::LogEntry;
use crate::auth::Auth;
use crate::dto::{json, time};
use crate::error::ApiError;
use crate::extract::ApiPath as Path;
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new().routes(routes!(get_trace))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceSpan {
    pub span_id: String,
    pub parent_span_id: Option<String>,
    /// `db.sql.query`, `http.client`, `cache.get`, `view.render`, …
    pub op: String,
    pub description: Option<String>,
    pub start: String,
    pub duration_us: u64,
    pub status: Option<String>,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceTransaction {
    pub project_id: i64,
    pub environment_id: i64,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    /// `request`, `job`, `command` or `other`.
    pub kind: String,
    /// As counted: `GET /orders/{order}`.
    pub name: String,
    pub op: String,
    pub status: Option<String>,
    pub http_status: Option<i64>,
    pub failed: bool,
    pub start: String,
    pub duration_us: u64,
    pub release: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
    pub container: Option<String>,
    /// Oldest first.
    pub spans: Vec<TraceSpan>,
}

impl From<&StoredTransaction> for TraceTransaction {
    fn from(stored: &StoredTransaction) -> Self {
        let t = &stored.transaction;
        Self {
            project_id: stored.scope.project.get(),
            environment_id: stored.scope.environment.get(),
            span_id: t.span_id.clone(),
            parent_span_id: t.parent_span_id.clone(),
            kind: t.kind.as_str().to_owned(),
            name: t.display_name(),
            op: t.op.clone(),
            status: t.status.clone(),
            http_status: t.http_status,
            failed: t.failed(),
            start: time(t.start),
            duration_us: t.duration_us,
            release: t.release.clone(),
            service: t.service.clone(),
            host: t.host.clone(),
            container: t.container.clone(),
            spans: t
                .spans
                .iter()
                .map(|s| TraceSpan {
                    span_id: s.span_id.clone(),
                    parent_span_id: s.parent_span_id.clone(),
                    op: s.op.clone(),
                    description: s.description.clone(),
                    start: time(s.start),
                    duration_us: s.duration_us,
                    status: s.status.clone(),
                    data: json(&s.data),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Trace {
    pub trace_id: String,
    /// The requests, jobs and commands of the trace, oldest first; a job
    /// continues the trace of the request that queued it.
    pub transactions: Vec<TraceTransaction>,
    /// The trace's log entries, oldest first (at most 500).
    pub logs: Vec<LogEntry>,
    pub events: Vec<EventListItem>,
}

/// Everything that carries one trace ID.
#[utoipa::path(
    get,
    path = "/traces/{trace_id}",
    tag = "traces",
    security(("session" = [])),
    params(("trace_id" = String, Path)),
    responses((status = OK, body = Trace))
)]
async fn get_trace(
    auth: Auth,
    State(state): State<ApiState>,
    Path(trace_id): Path<String>,
) -> Result<Json<Trace>, ApiError> {
    auth.require(Permission::ReadData)?;
    let query = LogQuery {
        trace_id: Some(trace_id.clone()),
        text: SearchText::default(),
        limit: 500,
        ..LogQuery::default()
    };
    let mut logs = state.logs.search.execute(query).await?.entries;
    logs.reverse();
    let events = state.issues.trace.execute(&trace_id).await?;
    let transactions = state.performance.trace.execute(&trace_id).await?;
    Ok(Json(Trace {
        trace_id,
        transactions: transactions.iter().map(TraceTransaction::from).collect(),
        logs: logs.iter().map(LogEntry::from).collect(),
        events: events.iter().map(EventListItem::from).collect(),
    }))
}
