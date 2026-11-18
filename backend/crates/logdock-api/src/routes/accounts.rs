//! Accounts managed by administrators, and the audit log.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use logdock_identity::application::commands::{AccountChanges, NewAccountInput};
use logdock_identity::domain::AccountId;
use logdock_kernel::Secret;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::identity::{Account, AccountRole};
use crate::auth::Auth;
use crate::dto::time;
use crate::error::{ApiError, Problem};
use crate::extract::{ApiJson, ApiPath as Path, ApiQuery, Client};
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new()
        .routes(routes!(list_accounts, create_account))
        .routes(routes!(update_account, delete_account))
        .routes(routes!(reset_password))
        .routes(routes!(audit_log))
}

/// All accounts, for administrators.
#[utoipa::path(
    get,
    path = "/accounts",
    tag = "accounts",
    security(("session" = [])),
    responses((status = OK, body = Vec<Account>), (status = FORBIDDEN, body = Problem))
)]
async fn list_accounts(auth: Auth, State(state): State<ApiState>) -> Result<Json<Vec<Account>>, ApiError> {
    let accounts = state.identity.list_accounts.execute(&auth.0).await?;
    Ok(Json(accounts.iter().map(Account::from).collect()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAccountRequest {
    pub email: String,
    pub name: String,
    pub role: AccountRole,
    pub password: String,
    #[serde(default)]
    pub locale: Option<String>,
}

/// Creates an account.
#[utoipa::path(
    post,
    path = "/accounts",
    tag = "accounts",
    security(("session" = [])),
    request_body = CreateAccountRequest,
    responses(
        (status = CREATED, body = Account),
        (status = CONFLICT, body = Problem, description = "`email_taken`"),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn create_account(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    ApiJson(body): ApiJson<CreateAccountRequest>,
) -> Result<(StatusCode, Json<Account>), ApiError> {
    let input = NewAccountInput {
        email: body.email,
        name: body.name,
        role: body.role.into(),
        password: Secret::new(body.password),
        locale: body.locale,
    };
    let account = state.identity.create_account.execute(&auth.0, input, &client).await?;
    Ok((StatusCode::CREATED, Json(Account::from(&account))))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAccountRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role: Option<AccountRole>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

/// Changes another account's name, role, or whether it is disabled.
#[utoipa::path(
    patch,
    path = "/accounts/{id}",
    tag = "accounts",
    security(("session" = [])),
    params(("id" = i64, Path)),
    request_body = UpdateAccountRequest,
    responses(
        (status = OK, body = Account),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem, description = "`last_admin`, `own_account`"),
    )
)]
async fn update_account(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<UpdateAccountRequest>,
) -> Result<Json<Account>, ApiError> {
    let changes =
        AccountChanges { name: body.name, role: body.role.map(Into::into), disabled: body.disabled };
    let account =
        state.identity.update_account.execute(&auth.0, AccountId::new(id), changes, &client).await?;
    Ok(Json(Account::from(&account)))
}

/// Deletes another account.
#[utoipa::path(
    delete,
    path = "/accounts/{id}",
    tag = "accounts",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses(
        (status = NO_CONTENT),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem, description = "`last_admin`, `own_account`"),
    )
)]
async fn delete_account(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.identity.delete_account.execute(&auth.0, AccountId::new(id), &client).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResetPasswordRequest {
    pub password: String,
}

/// Sets another account's password; its sessions end.
#[utoipa::path(
    post,
    path = "/accounts/{id}/password",
    tag = "accounts",
    security(("session" = [])),
    params(("id" = i64, Path)),
    request_body = ResetPasswordRequest,
    responses((status = NO_CONTENT), (status = NOT_FOUND, body = Problem), (status = UNPROCESSABLE_ENTITY, body = Problem))
)]
async fn reset_password(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let password = Secret::new(body.password);
    state.identity.reset_password.execute(&auth.0, AccountId::new(id), password, &client).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditEntry {
    pub id: i64,
    pub at: String,
    pub action: String,
    /// `success`, `failure` or `denied`.
    pub outcome: String,
    pub actor: Option<String>,
    pub target: Option<String>,
    pub reason: Option<String>,
    pub client_ip: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditQuery {
    /// Entries older than this one.
    pub before: Option<i64>,
    /// At most this many (default 100, at most 500).
    pub limit: Option<u32>,
}

/// The audit log, newest first, for administrators.
#[utoipa::path(
    get,
    path = "/audit",
    tag = "accounts",
    security(("session" = [])),
    params(AuditQuery),
    responses((status = OK, body = Vec<AuditEntry>), (status = FORBIDDEN, body = Problem))
)]
async fn audit_log(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(query): ApiQuery<AuditQuery>,
) -> Result<Json<Vec<AuditEntry>>, ApiError> {
    let entries = state.identity.audit_log.execute(&auth.0, query.before, query.limit.unwrap_or(100)).await?;
    Ok(Json(
        entries
            .into_iter()
            .map(|(id, e)| AuditEntry {
                id,
                at: time(e.at),
                action: e.action,
                outcome: e.outcome.as_str().to_owned(),
                actor: e.actor_name,
                target: e.target,
                reason: e.reason,
                client_ip: e.client_ip.map(|ip| ip.to_string()),
            })
            .collect(),
    ))
}
