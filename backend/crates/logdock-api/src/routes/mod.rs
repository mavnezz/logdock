mod accounts;
pub(crate) mod health;
mod identity;
mod projects;

use utoipa_axum::router::OpenApiRouter;

use crate::state::ApiState;

/// All routes of API version 1, relative to [`crate::API_V1`].
pub(crate) fn v1() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new().merge(identity::routes()).merge(accounts::routes()).merge(projects::routes())
}
