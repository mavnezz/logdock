//! Liveness and readiness, for Docker's health check and monitoring. Open
//! to everyone; the answers hold no data.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new().routes(routes!(live)).routes(routes!(ready))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Health {
    /// `live`, `ready` or `not_ready`.
    pub status: &'static str,
    /// What keeps Logdock from taking data: `database_unavailable`,
    /// `low_disk`, `writer_stalled`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<&'static str>,
}

/// The process runs and answers.
#[utoipa::path(get, path = "/health/live", tag = "system", responses((status = OK, body = Health)))]
async fn live() -> Json<Health> {
    Json(Health { status: "live", problems: Vec::new() })
}

/// Logdock can take data: the database answers, there is free space, and no
/// writer has stalled.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "system",
    responses((status = OK, body = Health), (status = SERVICE_UNAVAILABLE, body = Health))
)]
async fn ready(State(state): State<ApiState>) -> impl IntoResponse {
    let readiness = state.retention.check_readiness.execute().await;
    if readiness.is_ready() {
        (StatusCode::OK, Json(Health { status: "ready", problems: Vec::new() }))
    } else {
        let problems = readiness.problems.iter().map(|p| p.code()).collect();
        (StatusCode::SERVICE_UNAVAILABLE, Json(Health { status: "not_ready", problems }))
    }
}
