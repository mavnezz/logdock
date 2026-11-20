//! Log entries: the search, one entry, the counts over time, the sources,
//! and the live view as Server-Sent Events.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use logdock_identity::domain::Permission;
use logdock_kernel::{EnvironmentId, ProjectId, Timestamp};
use logdock_logs::domain::{self as logs, Cursor, HistogramQuery, Level, LogId, LogQuery, SearchText};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::Auth;
use crate::dto::{json, time};
use crate::error::{ApiError, Problem};
use crate::extract::{ApiPath as Path, ApiQuery};
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new()
        .routes(routes!(search))
        .routes(routes!(histogram))
        .routes(routes!(sources))
        .routes(routes!(entry))
}

/// The live view, outside the OpenAPI document: an event stream.
pub(crate) fn stream_router() -> Router<ApiState> {
    Router::new().route(&format!("{}/logs/stream", crate::API_V1), get(stream))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogEntry {
    pub id: String,
    /// When it was written, RFC 3339 with microseconds.
    pub time: String,
    pub received: String,
    pub project_id: i64,
    pub environment_id: i64,
    /// `trace`, `debug`, `info`, `warn`, `error` or `fatal`.
    pub level: String,
    pub message: String,
    pub template: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
    pub container: Option<String>,
    pub release: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub user_id: Option<String>,
    /// The attributes, flattened (`user.id`), in the order they were sent.
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub attributes: serde_json::Value,
    /// How many values redaction replaced.
    pub redactions: u32,
    /// Whether limits cut something short.
    pub truncated: bool,
}

impl From<&logs::LogEntry> for LogEntry {
    fn from(entry: &logs::LogEntry) -> Self {
        Self {
            id: entry.id.map(|id| id.to_string()).unwrap_or_default(),
            time: time(entry.time),
            received: time(entry.received),
            project_id: entry.scope.project.get(),
            environment_id: entry.scope.environment.get(),
            level: entry.level.as_str().to_owned(),
            message: entry.message.clone(),
            template: entry.template.clone(),
            service: entry.service.clone(),
            host: entry.host.clone(),
            container: entry.container.clone(),
            release: entry.release.clone(),
            trace_id: entry.trace_id.clone(),
            span_id: entry.span_id.clone(),
            user_id: entry.user_id.clone(),
            attributes: serde_json::Value::Object(
                entry.attributes.iter().map(|(k, v)| (k.clone(), json(v))).collect(),
            ),
            redactions: entry.redactions,
            truncated: entry.truncated,
        }
    }
}

/// The filters every log route takes. Lists are comma-separated.
#[derive(Debug, Deserialize, IntoParams, Default)]
#[into_params(parameter_in = Query)]
pub struct Filters {
    /// Project IDs.
    pub project: Option<String>,
    /// Stage IDs.
    pub environment: Option<String>,
    /// Services, e.g. `web,horizon`.
    pub service: Option<String>,
    pub host: Option<String>,
    /// Levels, e.g. `warn,error`.
    pub level: Option<String>,
    /// From this time on, RFC 3339.
    pub from: Option<String>,
    /// Before this time, RFC 3339.
    pub to: Option<String>,
    /// Words, `"phrases"`, `prefix*`, `key:value`; also `level:`,
    /// `service:`, `host:` and `trace:`.
    pub q: Option<String>,
    pub trace: Option<String>,
}

fn list(text: &Option<String>) -> Vec<String> {
    text.iter()
        .flat_map(|t| t.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn ids(text: &Option<String>) -> Result<Vec<i64>, ApiError> {
    list(text)
        .iter()
        .map(|id| id.parse().map_err(|_| ApiError::BadRequest(format!("{id:?} is no ID"))))
        .collect()
}

fn levels(text: &Option<String>) -> Result<Vec<Level>, ApiError> {
    list(text)
        .iter()
        .map(|l| Level::parse(l).ok_or_else(|| ApiError::BadRequest(format!("{l:?} is no level"))))
        .collect()
}

fn instant(text: &Option<String>) -> Result<Option<Timestamp>, ApiError> {
    text.as_deref()
        .filter(|t| !t.is_empty())
        .map(|t| {
            Timestamp::parse_rfc3339(t)
                .ok_or_else(|| ApiError::BadRequest(format!("{t:?} is no RFC 3339 time")))
        })
        .transpose()
}

impl Filters {
    fn query(&self) -> Result<LogQuery, ApiError> {
        Ok(LogQuery {
            projects: ids(&self.project)?.into_iter().map(ProjectId::new).collect(),
            environments: ids(&self.environment)?.into_iter().map(EnvironmentId::new).collect(),
            services: list(&self.service),
            hosts: list(&self.host),
            levels: levels(&self.level)?,
            from: instant(&self.from)?,
            to: instant(&self.to)?,
            text: SearchText::parse(self.q.as_deref().unwrap_or("")),
            trace_id: self.trace.clone().filter(|t| !t.is_empty()),
            before: None,
            limit: 100,
        })
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct Page {
    /// Where the previous page ended (its `next`).
    pub before: Option<String>,
    /// Entries per page (default 100, at most 500).
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogPage {
    pub entries: Vec<LogEntry>,
    /// Pass as `before` for the next page; `null` on the last one.
    pub next: Option<String>,
}

/// A page of entries, newest first.
#[utoipa::path(
    get,
    path = "/logs",
    tag = "logs",
    security(("session" = [])),
    params(Filters, Page),
    responses((status = OK, body = LogPage), (status = BAD_REQUEST, body = Problem))
)]
async fn search(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(filters): ApiQuery<Filters>,
    ApiQuery(page): ApiQuery<Page>,
) -> Result<Json<LogPage>, ApiError> {
    auth.require(Permission::ReadData)?;
    let mut query = filters.query()?;
    query.limit = page.limit.unwrap_or(100);
    query.before = match page.before.as_deref().filter(|b| !b.is_empty()) {
        Some(cursor) => {
            Some(Cursor::parse(cursor).ok_or_else(|| ApiError::BadRequest("before is no cursor".into()))?)
        }
        None => None,
    };
    let page = state.logs.search.execute(query).await?;
    Ok(Json(LogPage {
        entries: page.entries.iter().map(LogEntry::from).collect(),
        next: page.next.map(|cursor| cursor.to_string()),
    }))
}

/// One entry.
#[utoipa::path(
    get,
    path = "/logs/{id}",
    tag = "logs",
    security(("session" = [])),
    params(("id" = String, Path)),
    responses((status = OK, body = LogEntry), (status = NOT_FOUND, body = Problem))
)]
async fn entry(
    auth: Auth,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<LogEntry>, ApiError> {
    auth.require(Permission::ReadData)?;
    let id = LogId::parse(&id).ok_or(ApiError::NotFound)?;
    let entry = state.logs.get.execute(id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(LogEntry::from(&entry)))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HistogramBucket {
    /// Start of the bucket, RFC 3339.
    pub start: String,
    pub trace: u64,
    pub debug: u64,
    pub info: u64,
    pub warn: u64,
    pub error: u64,
    pub fatal: u64,
}

impl From<&logs::Bucket> for HistogramBucket {
    fn from(b: &logs::Bucket) -> Self {
        Self {
            start: time(Timestamp::from_unix_seconds(b.start)),
            trace: b.counts[0],
            debug: b.counts[1],
            info: b.counts[2],
            warn: b.counts[3],
            error: b.counts[4],
            fatal: b.counts[5],
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Histogram {
    pub bucket_seconds: i64,
    pub buckets: Vec<HistogramBucket>,
}

/// Entries per level over time. The search text is not applied: the counts
/// are kept per minute, project, stage, service and level.
#[utoipa::path(
    get,
    path = "/logs/histogram",
    tag = "logs",
    security(("session" = [])),
    params(Filters),
    responses((status = OK, body = Histogram), (status = BAD_REQUEST, body = Problem))
)]
async fn histogram(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(filters): ApiQuery<Filters>,
) -> Result<Json<Histogram>, ApiError> {
    auth.require(Permission::ReadData)?;
    let query = filters.query()?;
    let to = query.to.unwrap_or_else(|| Timestamp::from_unix_seconds(now_seconds() + 60));
    let from = query.from.unwrap_or_else(|| to.saturating_sub(Duration::from_secs(86_400)));
    let histogram = HistogramQuery {
        projects: query.projects,
        environments: query.environments,
        services: query.services,
        levels: query.levels,
        from,
        to,
        bucket_seconds: HistogramQuery::bucket_for(from, to),
    };
    let bucket_seconds = histogram.bucket_seconds;
    let buckets = state.logs.histogram.execute(histogram).await?;
    Ok(Json(Histogram { bucket_seconds, buckets: buckets.iter().map(HistogramBucket::from).collect() }))
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogSource {
    pub project_id: i64,
    pub environment_id: i64,
    pub service: Option<String>,
    pub host: Option<String>,
    pub last_seen: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SourcesQuery {
    /// Project IDs, comma-separated; all when left out.
    pub project: Option<String>,
}

/// The services and hosts entries came from, for the filters.
#[utoipa::path(
    get,
    path = "/logs/sources",
    tag = "logs",
    security(("session" = [])),
    params(SourcesQuery),
    responses((status = OK, body = Vec<LogSource>))
)]
async fn sources(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(query): ApiQuery<SourcesQuery>,
) -> Result<Json<Vec<LogSource>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let projects: Vec<ProjectId> = ids(&query.project)?.into_iter().map(ProjectId::new).collect();
    let sources = state.logs.sources.execute(&projects).await?;
    Ok(Json(
        sources
            .into_iter()
            .map(|s| LogSource {
                project_id: s.project.get(),
                environment_id: s.environment.get(),
                service: s.service,
                host: s.host,
                last_seen: time(s.last_seen),
            })
            .collect(),
    ))
}

/// `GET /api/v1/logs/stream`: new entries matching the filters as they are
/// written, as `event: entry` with a [`LogEntry`]; `event: lagged` when
/// more came than the stream could pass on.
async fn stream(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(filters): ApiQuery<Filters>,
) -> Result<Response, ApiError> {
    auth.require(Permission::ReadData)?;
    let query = filters.query()?;
    let receiver = state.logs.watch.execute();
    let events = BroadcastStream::new(receiver).filter_map(move |item| match item {
        Ok(entry) if query.matches(&entry) => {
            Some(Event::default().event("entry").json_data(LogEntry::from(entry.as_ref())).ok())
        }
        Ok(_) => None,
        Err(BroadcastStreamRecvError::Lagged(missed)) => {
            Some(Some(Event::default().event("lagged").data(missed.to_string())))
        }
    });
    let events = events.filter_map(|event| event.map(Ok::<_, Infallible>));
    let mut response =
        Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response();
    // Nginx would otherwise hold the events back.
    response.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(response)
}
