//! How a project or stage is doing over a range, on one page, as
//! Nightwatch's dashboard shows it: requests, jobs, errors, logs and
//! scheduled tasks.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use logdock_identity::domain::Permission;
use logdock_issues::application::ports::ActivityQuery;
use logdock_kernel::{EnvironmentId, ProjectId, Timestamp};
use logdock_logs::domain::HistogramQuery;
use logdock_monitors::application::ports::MonitorQuery;
use logdock_monitors::domain::Health;
use logdock_performance::application::Order;
use logdock_performance::application::ports::RollupQuery;
use logdock_performance::domain::Kind;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::issues::Issue;
use super::logs::HistogramBucket;
use super::monitors::Monitor;
use super::performance::{NamedStats, Stats};
use crate::auth::Auth;
use crate::dto::{ids, now, query_time, time};
use crate::error::{ApiError, Problem};
use crate::extract::ApiQuery;
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new().routes(routes!(at_a_glance))
}

/// Routes shown as the slowest and the most failing.
const TOP_ROUTES: usize = 5;
/// Issues shown with the most events.
const TOP_ISSUES: u32 = 5;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OverviewRange {
    /// Project IDs, comma-separated.
    pub project: Option<String>,
    /// Stage IDs, comma-separated.
    pub environment: Option<String>,
    /// RFC 3339; the last 24 hours by default.
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityBucket {
    pub start: String,
    pub count: u64,
    /// Failed requests or jobs.
    pub failed: u64,
    pub p95_ms: f64,
}

/// Requests or jobs over the range.
#[derive(Debug, Serialize, ToSchema)]
pub struct Activity {
    pub bucket_seconds: i64,
    pub buckets: Vec<ActivityBucket>,
    /// `flagged` counts the failed ones.
    pub total: Stats,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBucket {
    pub start: String,
    pub count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActiveIssueDto {
    #[serde(flatten)]
    pub issue: Issue,
    /// Events in the range.
    pub events: u64,
    /// Users affected, over all its events.
    pub users: u64,
}

/// Error events over the range, and their issues.
#[derive(Debug, Serialize, ToSchema)]
pub struct Errors {
    /// Whole hours: events are counted per hour.
    pub bucket_seconds: i64,
    pub buckets: Vec<ErrorBucket>,
    pub events: u64,
    /// First seen in the range.
    pub new_issues: u64,
    /// Came back in the range after they were resolved.
    pub regressed: u64,
    /// The issues with the most events in the range; muted ones left out.
    pub top: Vec<ActiveIssueDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogLevels {
    pub trace: u64,
    pub debug: u64,
    pub info: u64,
    pub warn: u64,
    pub error: u64,
    pub fatal: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogActivity {
    pub bucket_seconds: i64,
    pub buckets: Vec<HistogramBucket>,
    pub total: LogLevels,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MonitorHealth {
    pub total: u32,
    pub ok: u32,
    /// Failing or missed, as they are now.
    pub failing: Vec<Monitor>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Overview {
    pub from: String,
    pub to: String,
    pub requests: Activity,
    pub jobs: Activity,
    /// The routes with the slowest p95, and those with the most failed
    /// requests.
    pub slowest_routes: Vec<NamedStats>,
    pub failing_routes: Vec<NamedStats>,
    pub errors: Errors,
    pub logs: LogActivity,
    pub monitors: MonitorHealth,
}

/// Requests, jobs, errors, logs and scheduled tasks of a range at a glance.
#[utoipa::path(
    get,
    path = "/overview",
    tag = "overview",
    security(("session" = [])),
    params(OverviewRange),
    responses((status = OK, body = Overview), (status = BAD_REQUEST, body = Problem))
)]
async fn at_a_glance(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(q): ApiQuery<OverviewRange>,
) -> Result<Json<Overview>, ApiError> {
    auth.require(Permission::ReadData)?;
    let projects: Vec<ProjectId> = ids(&q.project)?.into_iter().map(ProjectId::new).collect();
    let environments: Vec<EnvironmentId> = ids(&q.environment)?.into_iter().map(EnvironmentId::new).collect();
    let to = query_time("to", q.to.as_deref())?.unwrap_or_else(now);
    let from = query_time("from", q.from.as_deref())?
        .unwrap_or_else(|| to.saturating_sub(Duration::from_secs(86_400)));
    if to <= from {
        return Err(ApiError::BadRequest("the range ends before it starts".into()));
    }
    let rollups = |kind: Kind| RollupQuery {
        projects: projects.clone(),
        environments: environments.clone(),
        kind,
        from,
        to,
    };
    let activity = async |kind: Kind| -> Result<Activity, ApiError> {
        let page = state.performance.series.execute(rollups(kind), None).await?;
        Ok(Activity {
            bucket_seconds: page.bucket_seconds,
            buckets: page
                .buckets
                .iter()
                .map(|(start, r)| ActivityBucket {
                    start: time(Timestamp::from_unix_seconds(*start)),
                    count: r.count,
                    failed: r.flagged,
                    p95_ms: Stats::from(r).p95_ms,
                })
                .collect(),
            total: Stats::from(&page.total),
        })
    };
    let requests = activity(Kind::Request).await?;
    let jobs = activity(Kind::Job).await?;
    let named = async |order: Order| -> Result<Vec<NamedStats>, ApiError> {
        let rows = state.performance.overview.execute(rollups(Kind::Request), order, TOP_ROUTES).await?;
        Ok(rows
            .iter()
            .filter(|r| order != Order::Failures || r.rollup.flagged > 0)
            .map(|r| NamedStats { name: r.name.clone(), stats: Stats::from(&r.rollup) })
            .collect())
    };
    let slowest_routes = named(Order::Slowest).await?;
    let failing_routes = named(Order::Failures).await?;

    let found = state
        .issues
        .activity
        .execute(ActivityQuery {
            projects: projects.clone(),
            environments: environments.clone(),
            from,
            to,
            bucket_seconds: requests.bucket_seconds,
            top: TOP_ISSUES,
        })
        .await?;
    let errors = Errors {
        bucket_seconds: found.bucket_seconds,
        buckets: found
            .buckets
            .iter()
            .map(|(start, count)| ErrorBucket {
                start: time(Timestamp::from_unix_seconds(*start)),
                count: *count,
            })
            .collect(),
        events: found.events,
        new_issues: found.new_issues,
        regressed: found.regressed,
        top: found
            .top
            .iter()
            .map(|a| ActiveIssueDto { issue: Issue::from(&a.issue), events: a.events, users: a.users })
            .collect(),
    };

    let bucket_seconds = HistogramQuery::bucket_for(from, to);
    let histogram = state
        .logs
        .histogram
        .execute(HistogramQuery {
            projects: projects.clone(),
            environments: environments.clone(),
            services: Vec::new(),
            levels: Vec::new(),
            from,
            to,
            bucket_seconds,
        })
        .await?;
    let mut sums = [0u64; 6];
    for bucket in &histogram {
        for (sum, count) in sums.iter_mut().zip(bucket.counts) {
            *sum += count;
        }
    }
    let logs = LogActivity {
        bucket_seconds,
        buckets: histogram.iter().map(HistogramBucket::from).collect(),
        total: LogLevels {
            trace: sums[0],
            debug: sums[1],
            info: sums[2],
            warn: sums[3],
            error: sums[4],
            fatal: sums[5],
        },
    };

    let all = state.monitors.list.execute(MonitorQuery { projects, environments }).await?;
    let monitors = MonitorHealth {
        total: all.len() as u32,
        ok: all.iter().filter(|m| m.monitor.health == Health::Ok).count() as u32,
        failing: all.iter().filter(|m| m.monitor.health.bad()).map(Monitor::from).collect(),
    };

    Ok(Json(Overview {
        from: time(from),
        to: time(to),
        requests,
        jobs,
        slowest_routes,
        failing_routes,
        errors,
        logs,
        monitors,
    }))
}
