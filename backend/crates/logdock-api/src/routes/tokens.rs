//! The caller's API tokens: made and revoked in the interface, used by
//! programs (scripts, Claude through MCP) in `Authorization: Bearer …`.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use logdock_identity::domain::{ApiToken, ApiTokenId, TokenScope};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::Auth;
use crate::dto::{optional_time, time};
use crate::error::{ApiError, Problem};
use crate::extract::{ApiJson, ApiPath as Path, Client};
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new().routes(routes!(list_tokens, create_token)).routes(routes!(revoke_token))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Logs, issues, traces, monitoring and projects.
    Read,
    /// Also resolve, mute and reopen issues.
    Triage,
}

impl From<TokenScope> for Scope {
    fn from(scope: TokenScope) -> Self {
        match scope {
            TokenScope::Read => Self::Read,
            TokenScope::Triage => Self::Triage,
        }
    }
}

impl From<Scope> for TokenScope {
    fn from(scope: Scope) -> Self {
        match scope {
            Scope::Read => Self::Read,
            Scope::Triage => Self::Triage,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Token {
    pub id: i64,
    pub name: String,
    pub scope: Scope,
    /// The token's first characters.
    pub prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

impl From<&ApiToken> for Token {
    fn from(token: &ApiToken) -> Self {
        Self {
            id: token.id.get(),
            name: token.name.as_str().to_owned(),
            scope: token.scope.into(),
            prefix: token.prefix.clone(),
            created_at: time(token.created_at),
            last_used_at: optional_time(token.last_used_at),
            revoked_at: optional_time(token.revoked_at),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedToken {
    pub token: Token,
    /// The token itself, shown this once.
    pub secret: String,
    /// Logdock's MCP endpoint, for `claude mcp add`.
    pub mcp_url: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTokenRequest {
    /// What it is for, such as `claude on my laptop`.
    pub name: String,
    pub scope: Scope,
}

/// The caller's API tokens, newest first.
#[utoipa::path(
    get,
    path = "/me/tokens",
    tag = "auth",
    security(("session" = [])),
    responses((status = OK, body = Vec<Token>))
)]
async fn list_tokens(auth: Auth, State(state): State<ApiState>) -> Result<Json<Vec<Token>>, ApiError> {
    let tokens = state.identity.list_tokens.execute(&auth.0).await?;
    Ok(Json(tokens.iter().map(Token::from).collect()))
}

/// A new API token of the caller's account; shown this once. Made in a
/// session only, never with another token.
#[utoipa::path(
    post,
    path = "/me/tokens",
    tag = "auth",
    security(("session" = [])),
    request_body = CreateTokenRequest,
    responses(
        (status = CREATED, body = CreatedToken),
        (status = UNPROCESSABLE_ENTITY, body = Problem, description = "`invalid_input`"),
    )
)]
async fn create_token(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    ApiJson(body): ApiJson<CreateTokenRequest>,
) -> Result<(StatusCode, Json<CreatedToken>), ApiError> {
    let created =
        state.identity.create_token.execute(&auth.0, &body.name, body.scope.into(), &client).await?;
    let body = CreatedToken {
        token: Token::from(&created.token),
        secret: created.secret.expose().to_owned(),
        mcp_url: state.config.url("/mcp"),
    };
    Ok((StatusCode::CREATED, Json(body)))
}

/// One of the caller's tokens stops working at once.
#[utoipa::path(
    delete,
    path = "/me/tokens/{id}",
    tag = "auth",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = NO_CONTENT), (status = NOT_FOUND, body = Problem))
)]
async fn revoke_token(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.identity.revoke_token.execute(&auth.0, ApiTokenId::new(id), &client).await?;
    Ok(StatusCode::NO_CONTENT)
}
