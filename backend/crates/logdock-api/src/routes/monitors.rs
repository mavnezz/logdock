//! Scheduled tasks by their check-ins.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use logdock_identity::domain::Permission;
use logdock_kernel::{EnvironmentId, ProjectId};
use logdock_monitors::application::MonitorSummary;
use logdock_monitors::application::ports::MonitorQuery;
use logdock_monitors::domain::{CheckIn, MonitorId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::Auth;
use crate::dto::{ids, optional_time, time};
use crate::error::{ApiError, Problem};
use crate::extract::{ApiPath as Path, ApiQuery, Client};
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new().routes(routes!(list_monitors)).routes(routes!(get_monitor, delete_monitor))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Monitor {
    pub id: i64,
    pub project_id: i64,
    pub environment_id: i64,
    /// As the SDK names it: `scheduled_nightly-report`.
    pub slug: String,
    /// The slug without sentry-laravel's prefix.
    pub name: String,
    /// A crontab (`0 3 * * *`) or `every 10 minute`; `null` without one.
    pub schedule: Option<String>,
    pub timezone: Option<String>,
    pub margin_minutes: Option<u64>,
    pub max_runtime_minutes: Option<u64>,
    /// `new`, `ok`, `failing` or `missed`.
    pub health: String,
    pub last_check_in: Option<String>,
    /// `in_progress`, `ok`, `error`, `missed` or `timeout`.
    pub last_status: Option<String>,
    /// When the next run should start.
    pub next_run: Option<String>,
    pub created_at: String,
}

impl From<&MonitorSummary> for Monitor {
    fn from(s: &MonitorSummary) -> Self {
        let m = &s.monitor;
        Self {
            id: m.id.get(),
            project_id: m.scope.project.get(),
            environment_id: m.scope.environment.get(),
            slug: m.slug.clone(),
            name: m.name().to_owned(),
            schedule: m.config.as_ref().map(|c| c.schedule.text()),
            timezone: m.config.as_ref().map(|c| c.timezone.clone()),
            margin_minutes: m.config.as_ref().map(|c| c.margin.as_secs() / 60),
            max_runtime_minutes: m.config.as_ref().map(|c| c.max_runtime.as_secs() / 60),
            health: m.health.as_str().to_owned(),
            last_check_in: optional_time(m.last_check_in),
            last_status: m.last_status.map(|s| s.as_str().to_owned()),
            next_run: optional_time(s.next_run),
            created_at: time(m.created_at),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CheckInDto {
    pub check_in_id: String,
    /// `in_progress`, `ok`, `error`, `missed` (expected, never started) or
    /// `timeout` (did not finish in time).
    pub status: String,
    /// For a missed run, when it should have started.
    pub started: String,
    pub finished: Option<String>,
    pub duration_ms: Option<f64>,
    pub release: Option<String>,
    pub trace_id: Option<String>,
}

impl From<&CheckIn> for CheckInDto {
    fn from(c: &CheckIn) -> Self {
        Self {
            check_in_id: c.check_in_id.clone(),
            status: c.status.as_str().to_owned(),
            started: time(c.started),
            finished: optional_time(c.finished),
            duration_ms: c.duration.map(|d| (d.as_secs_f64() * 100_000.0).round() / 100.0),
            release: c.release.clone(),
            trace_id: c.trace_id.clone(),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MonitorFilters {
    /// Project IDs, comma-separated.
    pub project: Option<String>,
    /// Stage IDs, comma-separated.
    pub environment: Option<String>,
}

/// The scheduled tasks, those in trouble first.
#[utoipa::path(
    get,
    path = "/monitors",
    tag = "monitors",
    security(("session" = [])),
    params(MonitorFilters),
    responses((status = OK, body = Vec<Monitor>), (status = BAD_REQUEST, body = Problem))
)]
async fn list_monitors(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(filters): ApiQuery<MonitorFilters>,
) -> Result<Json<Vec<Monitor>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let query = MonitorQuery {
        projects: ids(&filters.project)?.into_iter().map(ProjectId::new).collect(),
        environments: ids(&filters.environment)?.into_iter().map(EnvironmentId::new).collect(),
    };
    let monitors = state.monitors.list.execute(query).await?;
    Ok(Json(monitors.iter().map(Monitor::from).collect()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MonitorDetail {
    pub monitor: Monitor,
    /// The latest 100, newest first.
    pub check_ins: Vec<CheckInDto>,
}

/// One scheduled task with its latest runs.
#[utoipa::path(
    get,
    path = "/monitors/{id}",
    tag = "monitors",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = OK, body = MonitorDetail), (status = NOT_FOUND, body = Problem))
)]
async fn get_monitor(
    auth: Auth,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<MonitorDetail>, ApiError> {
    auth.require(Permission::ReadData)?;
    let detail = state.monitors.get.execute(MonitorId::new(id)).await?;
    Ok(Json(MonitorDetail {
        monitor: Monitor::from(&detail.summary),
        check_ins: detail.check_ins.iter().map(CheckInDto::from).collect(),
    }))
}

/// Forgets a scheduled task, for one that was removed from the schedule;
/// its next check-in brings it back.
#[utoipa::path(
    delete,
    path = "/monitors/{id}",
    tag = "monitors",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = NO_CONTENT), (status = NOT_FOUND, body = Problem))
)]
async fn delete_monitor(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::ManageProjects)?;
    state.monitors.delete.execute(&auth.0.actor(), MonitorId::new(id), &client).await?;
    Ok(StatusCode::NO_CONTENT)
}
