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

/// One segment: a kind's data of one day of receipt.
#[derive(Debug, Serialize, ToSchema)]
pub struct SegmentUsage {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub kind: String,
    /// Days it is kept: its stages' own retention, or the kind's.
    pub retention_days: u32,
    /// Whether its stages keep their data for days of their own.
    pub own_retention: bool,
    pub bytes: u64,
}

/// What one project's stage takes of one kind, over the days kept.
#[derive(Debug, Serialize, ToSchema)]
pub struct StageUsage {
    pub project_id: i64,
    pub environment_id: i64,
    pub kind: String,
    pub rows: u64,
    /// Its share of the segments, by the size of its rows; measured every
    /// hour.
    pub bytes: u64,
}

/// Where the data is heading at the rate of the last full days.
#[derive(Debug, Serialize, ToSchema)]
pub struct StorageForecast {
    /// The full days the rate is taken from, at most 7.
    pub days: u32,
    pub daily_bytes: u64,
    /// What is kept once each kind and stage keeps as many days as its
    /// retention says.
    pub settled_bytes: u64,
    /// Days until the storage limit, if it will be reached; from then the
    /// oldest days go before their time.
    pub limit_in_days: Option<u32>,
    /// Days until the disk is down to its reserve, if it will be.
    pub disk_in_days: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Storage {
    pub kinds: Vec<KindUsage>,
    /// Every segment, oldest day first.
    pub segments: Vec<SegmentUsage>,
    /// Per project and stage, the largest first.
    pub stages: Vec<StageUsage>,
    /// `null` before the first full day.
    pub forecast: Option<StorageForecast>,
    pub segment_bytes: u64,
    /// Above this, the oldest days are deleted; 0 for no limit.
    pub max_storage_bytes: u64,
    pub database_bytes: u64,
    pub free_bytes: Option<u64>,
    /// Below this much free space ingest is refused.
    pub min_free_bytes: u64,
    pub rollups_days: u32,
}

/// How much each kind of data takes, per day, project and stage, where it
/// is heading, and the limits.
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
        segments: status
            .segments
            .iter()
            .map(|s| SegmentUsage {
                day: s.day.to_string(),
                kind: s.kind.as_str().to_owned(),
                retention_days: status.policy.days_kept(s.kind, s.keep),
                own_retention: s.keep > 0,
                bytes: s.bytes,
            })
            .collect(),
        stages: status
            .scopes
            .iter()
            .map(|u| StageUsage {
                project_id: u.project.get(),
                environment_id: u.environment.get(),
                kind: u.kind.as_str().to_owned(),
                rows: u.rows,
                bytes: u.bytes,
            })
            .collect(),
        forecast: status.forecast.map(|f| StorageForecast {
            days: f.days,
            daily_bytes: f.daily_bytes,
            settled_bytes: f.settled_bytes,
            limit_in_days: f.limit_in_days,
            disk_in_days: f.disk_in_days,
        }),
        max_storage_bytes: status.policy.max_storage_bytes,
        database_bytes: status.database_bytes,
        free_bytes: status.free_bytes,
        min_free_bytes: status.policy.min_free_bytes,
        rollups_days: status.policy.rollups_days,
    }))
}
