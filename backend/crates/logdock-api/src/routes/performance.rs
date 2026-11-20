//! Requests, jobs, commands, queries, outgoing requests and cache: their
//! rollups and samples.

use axum::Json;
use axum::extract::State;
use logdock_identity::domain::Permission;
use logdock_kernel::{EnvironmentId, ProjectId, Timestamp};
use logdock_performance::application::Order;
use logdock_performance::application::ports::{RollupQuery, SampleQuery, TransactionSummary};
use logdock_performance::domain::{Kind, Rollup};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::Auth;
use crate::dto::{ids, now, query_time, time};
use crate::error::{ApiError, Problem};
use crate::extract::ApiQuery;
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new()
        .routes(routes!(overview))
        .routes(routes!(series))
        .routes(routes!(samples))
        .routes(routes!(queues))
}

/// What a range of one name adds up to.
#[derive(Debug, Serialize, ToSchema)]
pub struct Stats {
    pub count: u64,
    /// Failed requests, jobs, commands, queries or outgoing requests; for
    /// the cache, misses.
    pub flagged: u64,
    pub total_ms: f64,
    pub average_ms: f64,
    /// Estimated from a histogram of durations.
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

fn ms(us: u64) -> f64 {
    (us as f64 / 1000.0 * 100.0).round() / 100.0
}

impl From<&Rollup> for Stats {
    fn from(r: &Rollup) -> Self {
        Self {
            count: r.count,
            flagged: r.flagged,
            total_ms: ms(r.total_us),
            average_ms: ms(r.average_us()),
            p50_ms: ms(r.percentile_us(0.5)),
            p95_ms: ms(r.percentile_us(0.95)),
            p99_ms: ms(r.percentile_us(0.99)),
            max_ms: ms(r.max_us),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NamedStats {
    /// `GET /orders/{order}`, `App\Jobs\SendInvoice`, a normalized statement,
    /// `POST api.stripe.com`, `get user:<n>:cart`.
    pub name: String,
    #[serde(flatten)]
    pub stats: Stats,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RangeQuery {
    /// `request`, `job`, `command`, `other`, `query`, `outgoing`, `cache`,
    /// or of queues `queue_wait`, `queue_process`, `queue_publish`.
    pub kind: String,
    /// Project IDs, comma-separated.
    pub project: Option<String>,
    /// Stage IDs, comma-separated.
    pub environment: Option<String>,
    /// RFC 3339; the last 24 hours by default.
    pub from: Option<String>,
    pub to: Option<String>,
}

fn range(q: &RangeQuery) -> Result<RollupQuery, ApiError> {
    let kind =
        Kind::parse(&q.kind).ok_or_else(|| ApiError::BadRequest(format!("{:?} is no kind", q.kind)))?;
    let now = now();
    let to = query_time("to", q.to.as_deref())?.unwrap_or(now);
    let from = query_time("from", q.from.as_deref())?
        .unwrap_or_else(|| Timestamp::from_unix_seconds(to.unix_seconds() - 86_400));
    Ok(RollupQuery {
        projects: ids(&q.project)?.into_iter().map(ProjectId::new).collect(),
        environments: ids(&q.environment)?.into_iter().map(EnvironmentId::new).collect(),
        kind,
        from,
        to,
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct QueueStats {
    /// The queue: `default`, `emails`, …
    pub name: String,
    /// Jobs put into it by the requests, jobs and commands measured.
    pub published: u64,
    /// Jobs that ran from it; of them, those that failed and the retries.
    pub processed: u64,
    pub failed: u64,
    pub retries: u64,
    /// From being queued to running.
    pub wait: Option<Stats>,
    /// Running.
    pub processing: Option<Stats>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct QueuesQuery {
    /// Project IDs, comma-separated.
    pub project: Option<String>,
    /// Stage IDs, comma-separated.
    pub environment: Option<String>,
    /// RFC 3339; the last 24 hours by default.
    pub from: Option<String>,
    pub to: Option<String>,
}

/// The queues over a range: what was put in and ran, what failed, and how
/// long jobs waited and ran. As in sentry-laravel, the queue is the job's
/// `messaging.destination.name`; jobs run without tracing are not counted.
#[utoipa::path(
    get,
    path = "/queues",
    tag = "performance",
    security(("session" = [])),
    params(QueuesQuery),
    responses((status = OK, body = Vec<QueueStats>), (status = BAD_REQUEST, body = Problem))
)]
async fn queues(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(q): ApiQuery<QueuesQuery>,
) -> Result<Json<Vec<QueueStats>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let query = |kind: Kind| -> Result<RollupQuery, ApiError> {
        let mut query = range(&RangeQuery {
            kind: kind.as_str().to_owned(),
            project: q.project.clone(),
            environment: q.environment.clone(),
            from: q.from.clone(),
            to: q.to.clone(),
        })?;
        query.kind = kind;
        Ok(query)
    };
    let mut queues: Vec<QueueStats> = Vec::new();
    for kind in [Kind::QueueProcess, Kind::QueueWait, Kind::QueuePublish] {
        for row in state.performance.overview.execute(query(kind)?, Order::Count, 500).await? {
            let at = match queues.iter().position(|queue| queue.name == row.name) {
                Some(at) => at,
                None => {
                    queues.push(QueueStats {
                        name: row.name.clone(),
                        published: 0,
                        processed: 0,
                        failed: 0,
                        retries: 0,
                        wait: None,
                        processing: None,
                    });
                    queues.len() - 1
                }
            };
            let queue = &mut queues[at];
            match kind {
                Kind::QueueProcess => {
                    queue.processed = row.rollup.count;
                    queue.failed = row.rollup.flagged;
                    queue.processing = Some(Stats::from(&row.rollup));
                }
                Kind::QueueWait => {
                    queue.retries = row.rollup.flagged;
                    queue.wait = Some(Stats::from(&row.rollup));
                }
                _ => queue.published = row.rollup.count,
            }
        }
    }
    queues.sort_by(|a, b| b.processed.cmp(&a.processed).then_with(|| a.name.cmp(&b.name)));
    Ok(Json(queues))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OverviewQuery {
    /// `total_time` (the default), `count`, `slowest` or `failures`.
    pub sort: Option<String>,
    pub limit: Option<u32>,
}

/// The names of one kind over a range, where the time goes first.
#[utoipa::path(
    get,
    path = "/performance",
    tag = "performance",
    security(("session" = [])),
    params(RangeQuery, OverviewQuery),
    responses((status = OK, body = Vec<NamedStats>), (status = BAD_REQUEST, body = Problem))
)]
async fn overview(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(q): ApiQuery<RangeQuery>,
    ApiQuery(o): ApiQuery<OverviewQuery>,
) -> Result<Json<Vec<NamedStats>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let order = match o.sort.as_deref().unwrap_or("total_time") {
        "total_time" => Order::TotalTime,
        "count" => Order::Count,
        "slowest" => Order::Slowest,
        "failures" => Order::Failures,
        other => return Err(ApiError::BadRequest(format!("{other:?} is no sort order"))),
    };
    let rows = state.performance.overview.execute(range(&q)?, order, o.limit.unwrap_or(100) as usize).await?;
    Ok(Json(
        rows.iter().map(|r| NamedStats { name: r.name.clone(), stats: Stats::from(&r.rollup) }).collect(),
    ))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct NameQuery {
    /// As listed by the overview.
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SeriesBucket {
    pub start: String,
    #[serde(flatten)]
    pub stats: Stats,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SeriesPage {
    pub bucket_seconds: i64,
    /// Every bucket of the range, oldest first.
    pub buckets: Vec<SeriesBucket>,
    pub total: Stats,
}

/// One name over a range, in about 60 buckets.
#[utoipa::path(
    get,
    path = "/performance/series",
    tag = "performance",
    security(("session" = [])),
    params(RangeQuery, NameQuery),
    responses((status = OK, body = SeriesPage), (status = BAD_REQUEST, body = Problem))
)]
async fn series(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(q): ApiQuery<RangeQuery>,
    ApiQuery(n): ApiQuery<NameQuery>,
) -> Result<Json<SeriesPage>, ApiError> {
    auth.require(Permission::ReadData)?;
    let page = state.performance.series.execute(range(&q)?, Some(&n.name)).await?;
    Ok(Json(SeriesPage {
        bucket_seconds: page.bucket_seconds,
        buckets: page
            .buckets
            .iter()
            .map(|(start, r)| SeriesBucket {
                start: time(Timestamp::from_unix_seconds(*start)),
                stats: Stats::from(r),
            })
            .collect(),
        total: Stats::from(&page.total),
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SampleParams {
    /// As listed by the overview.
    pub name: String,
    /// `slowest` (the default) or `newest`.
    pub order: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Sample {
    pub trace_id: String,
    pub project_id: i64,
    pub environment_id: i64,
    pub name: String,
    pub start: String,
    pub duration_ms: f64,
    pub status: Option<String>,
    pub http_status: Option<i64>,
    pub failed: bool,
    pub release: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
}

impl From<&TransactionSummary> for Sample {
    fn from(s: &TransactionSummary) -> Self {
        Self {
            trace_id: s.trace_id.clone(),
            project_id: s.scope.project.get(),
            environment_id: s.scope.environment.get(),
            name: s.name.clone(),
            start: time(s.start),
            duration_ms: ms(s.duration_us),
            status: s.status.clone(),
            http_status: s.http_status,
            failed: s.failed,
            release: s.release.clone(),
            service: s.service.clone(),
            host: s.host.clone(),
        }
    }
}

/// Requests, jobs or commands of one name, each with its trace.
#[utoipa::path(
    get,
    path = "/performance/samples",
    tag = "performance",
    security(("session" = [])),
    params(RangeQuery, SampleParams),
    responses((status = OK, body = Vec<Sample>), (status = BAD_REQUEST, body = Problem))
)]
async fn samples(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(q): ApiQuery<RangeQuery>,
    ApiQuery(p): ApiQuery<SampleParams>,
) -> Result<Json<Vec<Sample>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let range = range(&q)?;
    let slowest = match p.order.as_deref().unwrap_or("slowest") {
        "slowest" => true,
        "newest" => false,
        other => return Err(ApiError::BadRequest(format!("{other:?} is no order"))),
    };
    let query = SampleQuery {
        projects: range.projects,
        environments: range.environments,
        kind: range.kind,
        name: p.name,
        from: range.from,
        to: range.to,
        slowest,
        limit: p.limit.unwrap_or(20),
    };
    let found = state.performance.samples.execute(query).await?;
    Ok(Json(found.iter().map(Sample::from).collect()))
}
