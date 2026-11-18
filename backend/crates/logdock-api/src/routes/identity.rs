//! The first setup, sign-in and the session, and the caller's own account.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::CookieJar;
use logdock_identity::application::commands::{ChangePasswordInput, LoginInput, ProfileChanges, SetupInput};
use logdock_identity::application::{AccountSummary, AuthContext};
use logdock_identity::domain::Role;
use logdock_kernel::Secret;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{Auth, SameOrigin, removal_cookie, session_cookie};
use crate::dto::time;
use crate::error::{ApiError, Problem};
use crate::extract::{ApiJson, Client};
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new()
        .routes(routes!(setup_status, complete_setup))
        .routes(routes!(login))
        .routes(routes!(logout))
        .routes(routes!(session))
        .routes(routes!(update_profile))
        .routes(routes!(change_password))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AccountRole {
    Admin,
    Member,
}

impl From<Role> for AccountRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Admin => Self::Admin,
            Role::Member => Self::Member,
        }
    }
}

impl From<AccountRole> for Role {
    fn from(role: AccountRole) -> Self {
        match role {
            AccountRole::Admin => Self::Admin,
            AccountRole::Member => Self::Member,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Account {
    pub id: i64,
    /// Also the sign-in name.
    pub email: String,
    pub name: String,
    pub role: AccountRole,
    /// `en` or `de`.
    pub locale: String,
    pub disabled: bool,
    pub created_at: String,
}

impl From<&AccountSummary> for Account {
    fn from(account: &AccountSummary) -> Self {
        Self {
            id: account.id.get(),
            email: account.email.to_string(),
            name: account.name.clone(),
            role: account.role.into(),
            locale: account.locale.as_str().to_owned(),
            disabled: account.disabled,
            created_at: time(account.created_at),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Session {
    pub account: Account,
    /// Sent back in `X-CSRF-Token` with every request that changes something.
    pub csrf_token: String,
    /// When the session ends unless used before.
    pub expires_at: String,
}

impl From<&AuthContext> for Session {
    fn from(auth: &AuthContext) -> Self {
        Self {
            account: Account::from(&auth.account),
            csrf_token: auth.csrf_token.expose().to_owned(),
            expires_at: time(auth.expires_at),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatus {
    /// No account exists yet: the first administrator is to be created with
    /// the setup code from Logdock's log.
    pub needed: bool,
}

/// Whether the first setup is still to be done.
#[utoipa::path(get, path = "/setup", tag = "auth", responses((status = OK, body = SetupStatus)))]
async fn setup_status(State(state): State<ApiState>) -> Result<Json<SetupStatus>, ApiError> {
    Ok(Json(SetupStatus { needed: state.identity.setup_status.execute().await? }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    /// The code Logdock wrote to its log at the start.
    pub code: String,
    pub email: String,
    pub name: String,
    pub password: String,
    #[serde(default)]
    pub locale: Option<String>,
}

/// Creates the first administrator and signs them in.
#[utoipa::path(
    post,
    path = "/setup",
    tag = "auth",
    request_body = SetupRequest,
    responses(
        (status = OK, body = Session),
        (status = FORBIDDEN, body = Problem, description = "`invalid_setup_code`"),
        (status = CONFLICT, body = Problem, description = "`setup_completed`"),
        (status = UNPROCESSABLE_ENTITY, body = Problem, description = "`invalid_input`"),
        (status = TOO_MANY_REQUESTS, body = Problem, description = "`too_many_attempts`"),
    )
)]
async fn complete_setup(
    _: SameOrigin,
    State(state): State<ApiState>,
    Client(client): Client,
    jar: CookieJar,
    ApiJson(body): ApiJson<SetupRequest>,
) -> Result<(CookieJar, Json<Session>), ApiError> {
    let input = SetupInput {
        code: body.code,
        email: body.email,
        name: body.name,
        password: Secret::new(body.password),
        locale: body.locale,
    };
    let signed_in = state.identity.complete_setup.execute(input, &client).await?;
    let jar = jar.add(session_cookie(&signed_in, state.config.secure_cookies));
    Ok((jar, Json(Session::from(&signed_in.session))))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Signs in with e-mail address and password.
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = OK, body = Session),
        (status = UNAUTHORIZED, body = Problem, description = "`invalid_credentials`"),
        (status = TOO_MANY_REQUESTS, body = Problem, description = "`too_many_attempts`"),
    )
)]
async fn login(
    _: SameOrigin,
    State(state): State<ApiState>,
    Client(client): Client,
    jar: CookieJar,
    ApiJson(body): ApiJson<LoginRequest>,
) -> Result<(CookieJar, Json<Session>), ApiError> {
    let input = LoginInput { email: body.email, password: Secret::new(body.password) };
    let signed_in = state.identity.login.execute(input, &client).await?;
    let jar = jar.add(session_cookie(&signed_in, state.config.secure_cookies));
    Ok((jar, Json(Session::from(&signed_in.session))))
}

/// Ends the session.
#[utoipa::path(post, path = "/auth/logout", tag = "auth", security(("session" = [])), responses((status = NO_CONTENT)))]
async fn logout(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), ApiError> {
    state.identity.logout.execute(&auth.0, &client).await?;
    Ok((jar.add(removal_cookie(state.config.secure_cookies)), StatusCode::NO_CONTENT))
}

/// The signed-in account and the session's CSRF token.
#[utoipa::path(
    get,
    path = "/auth/session",
    tag = "auth",
    security(("session" = [])),
    responses((status = OK, body = Session), (status = UNAUTHORIZED, body = Problem))
)]
async fn session(auth: Auth) -> Json<Session> {
    Json(Session::from(&auth.0))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// `en` or `de`.
    #[serde(default)]
    pub locale: Option<String>,
}

/// Changes the caller's name or language.
#[utoipa::path(
    patch,
    path = "/me",
    tag = "auth",
    security(("session" = [])),
    request_body = ProfileRequest,
    responses((status = OK, body = Account), (status = UNPROCESSABLE_ENTITY, body = Problem))
)]
async fn update_profile(
    auth: Auth,
    State(state): State<ApiState>,
    ApiJson(body): ApiJson<ProfileRequest>,
) -> Result<Json<Account>, ApiError> {
    let changes = ProfileChanges { name: body.name, locale: body.locale };
    let account = state.identity.update_profile.execute(&auth.0, changes).await?;
    Ok(Json(Account::from(&account)))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PasswordRequest {
    pub current: String,
    pub new: String,
}

/// Changes the caller's password; their other sessions end.
#[utoipa::path(
    post,
    path = "/me/password",
    tag = "auth",
    security(("session" = [])),
    request_body = PasswordRequest,
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem, description = "`wrong_password`"),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn change_password(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    ApiJson(body): ApiJson<PasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let input = ChangePasswordInput { current: Secret::new(body.current), new: Secret::new(body.new) };
    state.identity.change_password.execute(&auth.0, input, &client).await?;
    Ok(StatusCode::NO_CONTENT)
}
