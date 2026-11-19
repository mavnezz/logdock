//! What Logdock is and how much it keeps.

use axum::Json;
use axum::extract::State;
use logdock_identity::domain::Permission;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::Auth;
use crate::error::ApiError;
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new().routes(routes!(info)).routes(routes!(storage))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SystemInfo {
    pub version: String,
    /// The address SDKs and forwarders send to.
    pub public_url: Option<String>,
}

/// The version that runs, and where it is reached.
#[utoipa::path(get, path = "/system/info", tag = "system", security(("session" = [])), responses((status = OK, body = SystemInfo)))]
async fn info(auth: Auth, State(state): State<ApiState>) -> Result<Json<SystemInfo>, ApiError> {
    auth.require(Permission::ReadData)?;
    Ok(Json(SystemInfo {
        version: logdock_kernel::VERSION.to_owned(),
        public_url: state.config.public_url.clone(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KindUsage {
    /// `logs`, `events` or `traces`.
    pub kind: String,
    pub bytes: u64,
    pub segments: u32,
    /// The oldest day kept, `YYYY-MM-DD`.
    pub oldest: Option<String>,
    pub retention_days: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Storage {
    pub kinds: Vec<KindUsage>,
    pub segment_bytes: u64,
    /// Above this, the oldest days are deleted; 0 for no limit.
    pub max_storage_bytes: u64,
    pub database_bytes: u64,
    pub free_bytes: Option<u64>,
    /// Below this much free space ingest is refused.
    pub min_free_bytes: u64,
    pub rollups_days: u32,
}

/// How much each kind of data takes, and the limits.
#[utoipa::path(get, path = "/system/storage", tag = "system", security(("session" = [])), responses((status = OK, body = Storage)))]
async fn storage(auth: Auth, State(state): State<ApiState>) -> Result<Json<Storage>, ApiError> {
    auth.require(Permission::ReadData)?;
    let status = state.retention.status.execute().await?;
    Ok(Json(Storage {
        segment_bytes: status.segment_bytes(),
        kinds: status
            .kinds
            .iter()
            .map(|k| KindUsage {
                kind: k.kind.as_str().to_owned(),
                bytes: k.bytes,
                segments: k.segments,
                oldest: k.oldest.map(|d| d.to_string()),
                retention_days: status.policy.days(k.kind),
            })
            .collect(),
        max_storage_bytes: status.policy.max_storage_bytes,
        database_bytes: status.database_bytes,
        free_bytes: status.free_bytes,
        min_free_bytes: status.policy.min_free_bytes,
        rollups_days: status.policy.rollups_days,
    }))
}
