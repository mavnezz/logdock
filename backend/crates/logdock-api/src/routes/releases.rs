//! Releases: what each brought in errors and how its requests did, so one
//! can be compared with the one before.

use axum::Json;
use axum::extract::State;
use logdock_identity::domain::Permission;
use logdock_kernel::{EnvironmentId, ProjectId, Timestamp};
use logdock_performance::application::Order;
use logdock_performance::application::ports::RollupQuery;
use logdock_performance::domain::Kind;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::performance::Stats;
use crate::auth::Auth;
use crate::dto::{ids, now, time};
use crate::error::ApiError;
use crate::extract::ApiQuery;
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new().routes(routes!(releases))
}

/// How far back requests are looked at: as long as the rollups are kept.
const REQUESTS_DAYS: i64 = 90;

#[derive(Debug, Serialize, ToSchema)]
pub struct Release {
    /// As the SDK sends it (`SENTRY_RELEASE`).
    pub release: String,
    /// When its first and last error or request was seen.
    pub first_seen: String,
    pub last_seen: String,
    /// Error events in it, the issues they belong to, and the issues that
    /// first appeared in it.
    pub events: u64,
    pub issues: u64,
    pub new_issues: u64,
    /// Its requests (measured with tracing): count, failures, durations.
    pub requests: Option<Stats>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReleasesQuery {
    /// Project IDs, comma-separated.
    pub project: Option<String>,
    /// Stage IDs, comma-separated.
    pub environment: Option<String>,
}

/// Adds a release seen from `first` to `last`, or widens its times.
fn note(out: &mut Vec<(Release, Timestamp)>, name: &str, first: Timestamp, last: Timestamp) {
    match out.iter_mut().find(|(r, _)| r.release == name) {
        Some((release, start)) => {
            if first < *start {
                *start = first;
                release.first_seen = time(first);
            }
            if time(last) > release.last_seen {
                release.last_seen = time(last);
            }
        }
        None => out.push((
            Release {
                release: name.to_owned(),
                first_seen: time(first),
                last_seen: time(last),
                events: 0,
                issues: 0,
                new_issues: 0,
                requests: None,
            },
            first,
        )),
    }
}

/// Releases, newest first: errors, new issues and requests of each.
#[utoipa::path(
    get,
    path = "/releases",
    tag = "releases",
    security(("session" = [])),
    params(ReleasesQuery),
    responses((status = OK, body = Vec<Release>))
)]
async fn releases(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(q): ApiQuery<ReleasesQuery>,
) -> Result<Json<Vec<Release>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let projects: Vec<ProjectId> = ids(&q.project)?.into_iter().map(ProjectId::new).collect();
    let environments: Vec<EnvironmentId> = ids(&q.environment)?.into_iter().map(EnvironmentId::new).collect();
    let errors = state.issues.releases.execute(&projects, &environments).await?;
    let to = now();
    let query = RollupQuery {
        projects: projects.clone(),
        environments: environments.clone(),
        kind: Kind::Release,
        from: Timestamp::from_unix_seconds(to.unix_seconds() - REQUESTS_DAYS * 86_400),
        to,
    };
    let requests = state.performance.overview.execute(query.clone(), Order::Count, 1000).await?;
    let extents = state.performance.extents.execute(query).await?;

    let mut out: Vec<(Release, Timestamp)> = Vec::new();
    for e in &errors {
        note(&mut out, &e.release, e.first_seen, e.last_seen);
    }
    for (name, first, last) in &extents {
        note(&mut out, name, *first, *last);
    }
    for (release, _) in &mut out {
        if let Some(e) = errors.iter().find(|e| e.release == release.release) {
            (release.events, release.issues, release.new_issues) = (e.events, e.issues, e.new_issues);
        }
        release.requests =
            requests.iter().find(|r| r.name == release.release).map(|r| Stats::from(&r.rollup));
    }
    // Newest first; within a minute (what requests are counted by), the one
    // seen last, then the higher name.
    out.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| b.0.last_seen.cmp(&a.0.last_seen))
            .then_with(|| b.0.release.cmp(&a.0.release))
    });
    out.truncate(100);
    Ok(Json(out.into_iter().map(|(release, _)| release).collect()))
}
