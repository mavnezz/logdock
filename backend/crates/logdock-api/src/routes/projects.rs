//! Projects, their stages, keys, settings and ingest counts. Everyone
//! signed in reads them; administrators change them.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use logdock_identity::domain::Permission;
use logdock_kernel::redaction::RedactionRules;
use logdock_kernel::{EnvironmentId, ProjectId};
use logdock_projects::application::ProjectChanges;
use logdock_projects::domain::{self as projects, IngestKey, KeyId, NotifyAddress, ProjectSettings};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::Auth;
use crate::dto::{optional_time, time};
use crate::error::{ApiError, Problem};
use crate::extract::{ApiJson, ApiPath as Path, ApiQuery, Client};
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new()
        .routes(routes!(list_projects, create_project))
        .routes(routes!(get_project, update_project, delete_project))
        .routes(routes!(add_environment))
        .routes(routes!(remove_environment))
        .routes(routes!(list_keys, create_key))
        .routes(routes!(revoke_key))
        .routes(routes!(ingest_counts))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Environment {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Redaction {
    /// More key fragments whose values are never stored, beyond the built-in
    /// ones (password, token, secret, …).
    #[serde(default)]
    pub extra_keys: Vec<String>,
    /// When set, only these attribute keys are kept; `user.*` keeps a prefix.
    #[serde(default)]
    pub allowed_keys: Option<Vec<String>>,
    #[serde(default)]
    pub scrub_emails: bool,
    #[serde(default)]
    pub scrub_ip_addresses: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Settings {
    /// Who gets e-mail about the project's errors, also people without an
    /// account.
    pub notify: Vec<String>,
    pub notify_new_issues: bool,
    pub notify_regressions: bool,
    pub notify_monitors: bool,
    pub redaction: Redaction,
}

impl From<&ProjectSettings> for Settings {
    fn from(settings: &ProjectSettings) -> Self {
        Self {
            notify: settings.notify.iter().map(|a| a.as_str().to_owned()).collect(),
            notify_new_issues: settings.notify_new_issues,
            notify_regressions: settings.notify_regressions,
            notify_monitors: settings.notify_monitors,
            redaction: Redaction {
                extra_keys: settings.redaction.extra_keys.clone(),
                allowed_keys: settings.redaction.allowed_keys.clone(),
                scrub_emails: settings.redaction.scrub_emails,
                scrub_ip_addresses: settings.redaction.scrub_ip_addresses,
            },
        }
    }
}

impl Settings {
    fn into_domain(self) -> Result<ProjectSettings, ApiError> {
        let notify = self
            .notify
            .iter()
            .filter(|a| !a.trim().is_empty())
            .map(|a| NotifyAddress::parse(a))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ApiError::InvalidInput(e.to_string()))?;
        let trimmed = |keys: Vec<String>| -> Vec<String> {
            keys.into_iter().map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()).collect()
        };
        Ok(ProjectSettings {
            notify,
            notify_new_issues: self.notify_new_issues,
            notify_regressions: self.notify_regressions,
            notify_monitors: self.notify_monitors,
            redaction: RedactionRules {
                extra_keys: trimmed(self.redaction.extra_keys),
                allowed_keys: self.redaction.allowed_keys.map(trimmed),
                scrub_emails: self.redaction.scrub_emails,
                scrub_ip_addresses: self.redaction.scrub_ip_addresses,
            },
        })
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub environments: Vec<Environment>,
    pub settings: Settings,
    pub created_at: String,
}

impl From<&projects::Project> for Project {
    fn from(project: &projects::Project) -> Self {
        Self {
            id: project.id.get(),
            name: project.name.to_string(),
            environments: project
                .environments
                .iter()
                .map(|e| Environment {
                    id: e.id.get(),
                    name: e.name.to_string(),
                    created_at: time(e.created_at),
                })
                .collect(),
            settings: Settings::from(&project.settings),
            created_at: time(project.created_at),
        }
    }
}

/// All projects with their stages.
#[utoipa::path(
    get,
    path = "/projects",
    tag = "projects",
    security(("session" = [])),
    responses((status = OK, body = Vec<Project>))
)]
async fn list_projects(auth: Auth, State(state): State<ApiState>) -> Result<Json<Vec<Project>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let projects = state.projects.list.execute().await?;
    Ok(Json(projects.iter().map(Project::from).collect()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectRequest {
    pub name: String,
    /// Its first stages, e.g. `["production", "staging"]`.
    #[serde(default)]
    pub environments: Vec<String>,
}

/// Creates a project with its first stages.
#[utoipa::path(
    post,
    path = "/projects",
    tag = "projects",
    security(("session" = [])),
    request_body = CreateProjectRequest,
    responses(
        (status = CREATED, body = Project),
        (status = CONFLICT, body = Problem, description = "`name_taken`"),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn create_project(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    ApiJson(body): ApiJson<CreateProjectRequest>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    auth.require(Permission::ManageProjects)?;
    let project =
        state.projects.create.execute(&auth.0.actor(), &body.name, &body.environments, &client).await?;
    Ok((StatusCode::CREATED, Json(Project::from(&project))))
}

/// One project.
#[utoipa::path(
    get,
    path = "/projects/{id}",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = OK, body = Project), (status = NOT_FOUND, body = Problem))
)]
async fn get_project(
    auth: Auth,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Project>, ApiError> {
    auth.require(Permission::ReadData)?;
    let project = state.projects.get.execute(ProjectId::new(id)).await?;
    Ok(Json(Project::from(&project)))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProjectRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub settings: Option<Settings>,
}

/// Renames a project or changes its settings.
#[utoipa::path(
    patch,
    path = "/projects/{id}",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path)),
    request_body = UpdateProjectRequest,
    responses(
        (status = OK, body = Project),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem, description = "`name_taken`"),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn update_project(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<UpdateProjectRequest>,
) -> Result<Json<Project>, ApiError> {
    auth.require(Permission::ManageProjects)?;
    let changes =
        ProjectChanges { name: body.name, settings: body.settings.map(Settings::into_domain).transpose()? };
    let project =
        state.projects.update.execute(&auth.0.actor(), ProjectId::new(id), changes, &client).await?;
    Ok(Json(Project::from(&project)))
}

/// Deletes a project with its stages, keys and issues.
#[utoipa::path(
    delete,
    path = "/projects/{id}",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = NO_CONTENT), (status = NOT_FOUND, body = Problem))
)]
async fn delete_project(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::ManageProjects)?;
    state.projects.delete.execute(&auth.0.actor(), ProjectId::new(id), &client).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddEnvironmentRequest {
    pub name: String,
}

/// Adds a stage to a project.
#[utoipa::path(
    post,
    path = "/projects/{id}/environments",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path)),
    request_body = AddEnvironmentRequest,
    responses(
        (status = CREATED, body = Environment),
        (status = CONFLICT, body = Problem, description = "`environment_taken`"),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn add_environment(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<AddEnvironmentRequest>,
) -> Result<(StatusCode, Json<Environment>), ApiError> {
    auth.require(Permission::ManageProjects)?;
    let env = state
        .projects
        .add_environment
        .execute(&auth.0.actor(), ProjectId::new(id), &body.name, &client)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(Environment { id: env.id.get(), name: env.name.to_string(), created_at: time(env.created_at) }),
    ))
}

/// Removes a stage that has no active keys.
#[utoipa::path(
    delete,
    path = "/projects/{id}/environments/{environment}",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path), ("environment" = i64, Path)),
    responses(
        (status = NO_CONTENT),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem, description = "`environment_in_use`"),
    )
)]
async fn remove_environment(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path((id, environment)): Path<(i64, i64)>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::ManageProjects)?;
    let (project, environment) = (ProjectId::new(id), EnvironmentId::new(environment));
    state.projects.remove_environment.execute(&auth.0.actor(), project, environment, &client).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Key {
    pub id: i64,
    pub environment_id: i64,
    pub label: Option<String>,
    /// The key's first characters.
    pub prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

impl From<&IngestKey> for Key {
    fn from(key: &IngestKey) -> Self {
        Self {
            id: key.id.get(),
            environment_id: key.environment.get(),
            label: key.label.as_ref().map(|l| l.as_str().to_owned()),
            prefix: key.prefix.clone(),
            created_at: time(key.created_at),
            last_used_at: optional_time(key.last_used_at),
            revoked_at: optional_time(key.revoked_at),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedKey {
    pub key: Key,
    /// The key itself, shown this once.
    pub secret: String,
    /// The DSN for the Sentry SDK: `SENTRY_LARAVEL_DSN`.
    pub dsn: String,
}

/// A project's keys, newest first, for administrators.
#[utoipa::path(
    get,
    path = "/projects/{id}/keys",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = OK, body = Vec<Key>), (status = NOT_FOUND, body = Problem))
)]
async fn list_keys(
    auth: Auth,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<Key>>, ApiError> {
    auth.require(Permission::ManageProjects)?;
    let keys = state.projects.list_keys.execute(ProjectId::new(id)).await?;
    Ok(Json(keys.iter().map(Key::from).collect()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateKeyRequest {
    pub environment_id: i64,
    #[serde(default)]
    pub label: Option<String>,
}

/// A new key for one stage; its secret and DSN are shown this once.
#[utoipa::path(
    post,
    path = "/projects/{id}/keys",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path)),
    request_body = CreateKeyRequest,
    responses((status = CREATED, body = CreatedKey), (status = NOT_FOUND, body = Problem))
)]
async fn create_key(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<CreateKeyRequest>,
) -> Result<(StatusCode, Json<CreatedKey>), ApiError> {
    auth.require(Permission::ManageProjects)?;
    let project = ProjectId::new(id);
    let label = body.label.unwrap_or_default();
    let created = state
        .projects
        .create_key
        .execute(&auth.0.actor(), project, EnvironmentId::new(body.environment_id), &label, &client)
        .await?;
    let dsn = state.config.dsn(created.secret.expose(), id);
    let body = CreatedKey { key: Key::from(&created.key), secret: created.secret.expose().to_owned(), dsn };
    Ok((StatusCode::CREATED, Json(body)))
}

/// A key stops working at once.
#[utoipa::path(
    delete,
    path = "/projects/{id}/keys/{key}",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path), ("key" = i64, Path)),
    responses((status = NO_CONTENT), (status = NOT_FOUND, body = Problem))
)]
async fn revoke_key(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path((id, key)): Path<(i64, i64)>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::ManageProjects)?;
    state.projects.revoke_key.execute(&auth.0.actor(), ProjectId::new(id), KeyId::new(key), &client).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LastRejection {
    pub reason: String,
    pub detail: String,
    pub at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IngestCount {
    /// `YYYY-MM-DD` in UTC.
    pub day: String,
    /// The key that was used; `null` for requests no key let in.
    pub key_id: Option<i64>,
    pub accepted: u64,
    /// Items of kinds Logdock does not keep.
    pub skipped: u64,
    /// Refused items by reason: `auth`, `environment`, `size`, `rate`,
    /// `invalid`, `disk`, `overloaded`.
    pub rejected: BTreeMap<String, u64>,
    pub last_rejection: Option<LastRejection>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CountsQuery {
    /// The last this many days including today (default 7, at most 90).
    pub days: Option<u32>,
}

/// What ingest accepted and refused for the project, per day and key.
#[utoipa::path(
    get,
    path = "/projects/{id}/ingest",
    tag = "projects",
    security(("session" = [])),
    params(("id" = i64, Path), CountsQuery),
    responses((status = OK, body = Vec<IngestCount>), (status = NOT_FOUND, body = Problem))
)]
async fn ingest_counts(
    auth: Auth,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    ApiQuery(query): ApiQuery<CountsQuery>,
) -> Result<Json<Vec<IngestCount>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let counts = state.projects.ingest_counts.execute(ProjectId::new(id), query.days.unwrap_or(7)).await?;
    Ok(Json(
        counts
            .into_iter()
            .map(|c| IngestCount {
                day: c.day.to_string(),
                key_id: c.key.map(KeyId::get),
                accepted: c.accepted,
                skipped: c.skipped,
                rejected: c.rejected.into_iter().map(|(r, n)| (r.as_str().to_owned(), n)).collect(),
                last_rejection: c.last_rejection.map(|(r, detail, at)| LastRejection {
                    reason: r.as_str().to_owned(),
                    detail,
                    at: time(at),
                }),
            })
            .collect(),
    ))
}
