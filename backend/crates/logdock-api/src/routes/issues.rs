//! Issues and their events; a trace with everything that carries it.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use logdock_identity::domain::Permission;
use logdock_issues::application::ports::{EventSummary, IssueQuery, IssueSort, IssueSummary};
use logdock_issues::application::{IssueDetail, StatusChange};
use logdock_issues::domain::{self as issues, ErrorEvent, Frame, IssueId, IssueStatus};
use logdock_kernel::{EnvironmentId, ProjectId, Timestamp};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{Auth, TokenWrites};
use crate::dto::{ids, json, optional_time, time};
use crate::error::{ApiError, Problem};
use crate::extract::{ApiJson, ApiPath as Path, ApiQuery, Client};
use crate::state::ApiState;

pub(crate) fn routes() -> OpenApiRouter<ApiState> {
    OpenApiRouter::new()
        .routes(routes!(list_issues))
        .routes(routes!(get_issue, change_issue, delete_issue))
        .routes(routes!(issue_events))
        .routes(routes!(issue_tags))
        .routes(routes!(get_event))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub environment_id: i64,
    pub title: String,
    pub culprit: Option<String>,
    /// `debug`, `info`, `warning`, `error` or `fatal`: the worst seen.
    pub level: String,
    /// `unresolved`, `resolved` or `muted`.
    pub status: String,
    pub resolved_at: Option<String>,
    /// Resolved in the release after this one.
    pub resolved_in_next_release: Option<String>,
    pub muted_until: Option<String>,
    pub muted_remaining: Option<u32>,
    pub first_seen: String,
    pub last_seen: String,
    pub count: u64,
    pub first_release: Option<String>,
    pub last_release: Option<String>,
    pub regressed_at: Option<String>,
    pub last_event_id: Option<String>,
}

impl From<&issues::Issue> for Issue {
    fn from(issue: &issues::Issue) -> Self {
        let (resolved_at, next_release, muted_until, muted_remaining) = match &issue.status {
            IssueStatus::Unresolved => (None, None, None, None),
            IssueStatus::Resolved { at, in_next_release } => {
                (Some(time(*at)), in_next_release.clone(), None, None)
            }
            IssueStatus::Muted { until, remaining } => (None, None, optional_time(*until), *remaining),
        };
        Self {
            id: issue.id.get(),
            project_id: issue.scope.project.get(),
            environment_id: issue.scope.environment.get(),
            title: issue.title.clone(),
            culprit: issue.culprit.clone(),
            level: issue.level.as_str().to_owned(),
            status: issue.status.name().to_owned(),
            resolved_at,
            resolved_in_next_release: next_release,
            muted_until,
            muted_remaining,
            first_seen: time(issue.first_seen),
            last_seen: time(issue.last_seen),
            count: issue.count,
            first_release: issue.first_release.clone(),
            last_release: issue.last_release.clone(),
            regressed_at: optional_time(issue.regressed_at),
            last_event_id: issue.last_event_id.clone(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssueListItem {
    #[serde(flatten)]
    pub issue: Issue,
    /// Events in the last 24 hours.
    pub last_day: u64,
}

impl From<&IssueSummary> for IssueListItem {
    fn from(summary: &IssueSummary) -> Self {
        Self { issue: Issue::from(&summary.issue), last_day: summary.last_day }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct IssueFilters {
    /// Project IDs, comma-separated.
    pub project: Option<String>,
    /// Stage IDs, comma-separated.
    pub environment: Option<String>,
    /// `unresolved` (the default), `resolved`, `muted` or `all`.
    pub status: Option<String>,
    /// Words in the title or culprit.
    pub q: Option<String>,
    /// `last_seen` (the default), `first_seen` or `count`.
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Issues, newest first unless sorted otherwise.
#[utoipa::path(
    get,
    path = "/issues",
    tag = "issues",
    security(("session" = [])),
    params(IssueFilters),
    responses((status = OK, body = Vec<IssueListItem>), (status = BAD_REQUEST, body = Problem))
)]
async fn list_issues(
    auth: Auth,
    State(state): State<ApiState>,
    ApiQuery(filters): ApiQuery<IssueFilters>,
) -> Result<Json<Vec<IssueListItem>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let status = match filters.status.as_deref().unwrap_or("unresolved") {
        "all" => None,
        status @ ("unresolved" | "resolved" | "muted") => Some(status.to_owned()),
        other => return Err(ApiError::BadRequest(format!("{other:?} is no status"))),
    };
    let sort = match filters.sort.as_deref().unwrap_or("last_seen") {
        "last_seen" => IssueSort::LastSeen,
        "first_seen" => IssueSort::FirstSeen,
        "count" => IssueSort::Count,
        other => return Err(ApiError::BadRequest(format!("{other:?} is no sort order"))),
    };
    let query = IssueQuery {
        projects: ids(&filters.project)?.into_iter().map(ProjectId::new).collect(),
        environments: ids(&filters.environment)?.into_iter().map(EnvironmentId::new).collect(),
        status,
        text: filters.q,
        sort,
        limit: filters.limit.unwrap_or(50),
        offset: filters.offset.unwrap_or(0),
    };
    let issues = state.issues.list.execute(query).await?;
    Ok(Json(issues.iter().map(IssueListItem::from).collect()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StackFrame {
    pub filename: Option<String>,
    pub abs_path: Option<String>,
    pub function: Option<String>,
    pub module: Option<String>,
    pub lineno: Option<i64>,
    pub colno: Option<i64>,
    pub in_app: bool,
    pub context_line: Option<String>,
    pub pre_context: Vec<String>,
    pub post_context: Vec<String>,
}

impl From<&Frame> for StackFrame {
    fn from(f: &Frame) -> Self {
        Self {
            filename: f.filename.clone(),
            abs_path: f.abs_path.clone(),
            function: f.function.clone(),
            module: f.module.clone(),
            lineno: f.lineno,
            colno: f.colno,
            in_app: f.in_app,
            context_line: f.context_line.clone(),
            pre_context: f.pre_context.clone(),
            post_context: f.post_context.clone(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExceptionInfo {
    /// The class.
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
    pub module: Option<String>,
    pub handled: Option<bool>,
    /// Oldest call first; the last frame is where it was thrown.
    pub frames: Vec<StackFrame>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Breadcrumb {
    pub time: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub category: Option<String>,
    pub level: Option<String>,
    pub message: Option<String>,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RequestInfo {
    pub method: Option<String>,
    pub url: Option<String>,
    pub query: Option<String>,
    /// All headers; secret ones (Authorization, Cookie, CSRF tokens) and
    /// those the redaction rules match read `[redacted]`.
    pub headers: Vec<(String, String)>,
    /// The body (form fields, JSON), redacted; a large one is left out.
    #[schema(value_type = Option<serde_json::Value>)]
    pub data: Option<serde_json::Value>,
    /// The cookies' names; their values are never kept.
    pub cookies: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfo {
    pub id: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Event {
    pub event_id: String,
    pub time: Option<String>,
    pub level: String,
    pub platform: Option<String>,
    pub release: Option<String>,
    pub environment: Option<String>,
    /// The container.
    pub server_name: Option<String>,
    /// The route or job.
    pub transaction: Option<String>,
    pub logger: Option<String>,
    pub message: Option<String>,
    /// Oldest cause first; the last one was thrown.
    pub exceptions: Vec<ExceptionInfo>,
    /// The stack of an event without an exception.
    pub frames: Vec<StackFrame>,
    pub tags: Vec<(String, String)>,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub request: Option<RequestInfo>,
    pub user: Option<UserInfo>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub sdk: Option<String>,
    /// The payload's contexts (runtime, os, trace, …), redacted.
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub contexts: serde_json::Value,
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub extra: serde_json::Value,
}

impl From<&ErrorEvent> for Event {
    fn from(e: &ErrorEvent) -> Self {
        let object = |key: &str| {
            e.payload.get(key).map(json).filter(|v| v.is_object()).unwrap_or_else(|| serde_json::json!({}))
        };
        Self {
            event_id: e.event_id.clone(),
            time: optional_time(e.time),
            level: e.level.as_str().to_owned(),
            platform: e.platform.clone(),
            release: e.release.clone(),
            environment: e.environment.clone(),
            server_name: e.server_name.clone(),
            transaction: e.transaction.clone(),
            logger: e.logger.clone(),
            message: e.message.clone(),
            exceptions: e
                .exceptions
                .iter()
                .map(|x| ExceptionInfo {
                    kind: x.kind.clone(),
                    value: x.value.clone(),
                    module: x.module.clone(),
                    handled: x.handled,
                    frames: x.frames.iter().map(StackFrame::from).collect(),
                })
                .collect(),
            frames: e.frames.iter().map(StackFrame::from).collect(),
            tags: e.tags.clone(),
            breadcrumbs: e
                .breadcrumbs
                .iter()
                .map(|b| Breadcrumb {
                    time: optional_time(b.time),
                    kind: b.kind.clone(),
                    category: b.category.clone(),
                    level: b.level.clone(),
                    message: b.message.clone(),
                    data: json(&b.data),
                })
                .collect(),
            request: e.request.as_ref().map(|r| RequestInfo {
                method: r.method.clone(),
                url: r.url.clone(),
                query: r.query.clone(),
                headers: r.headers.clone(),
                data: r.data.as_ref().map(json),
                cookies: r.cookies.clone(),
            }),
            user: e.user.as_ref().map(|u| UserInfo {
                id: u.id.clone(),
                email: u.email.clone(),
                username: u.username.clone(),
                ip_address: u.ip_address.clone(),
            }),
            trace_id: e.trace_id.clone(),
            span_id: e.span_id.clone(),
            sdk: e.sdk.clone(),
            contexts: object("contexts"),
            extra: object("extra"),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HourCount {
    /// The hour's start, RFC 3339.
    pub hour: String,
    pub count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReleaseCount {
    pub release: String,
    pub count: u64,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssueDetailDto {
    pub issue: Issue,
    /// Events per hour of the last 14 days, oldest first.
    pub hours: Vec<HourCount>,
    pub releases: Vec<ReleaseCount>,
    /// The latest event, kept with the issue.
    pub latest: Option<Event>,
}

impl From<&IssueDetail> for IssueDetailDto {
    fn from(d: &IssueDetail) -> Self {
        Self {
            issue: Issue::from(&d.issue),
            hours: d
                .hours
                .iter()
                .map(|(hour, count)| HourCount {
                    hour: time(Timestamp::from_unix_seconds(*hour)),
                    count: *count,
                })
                .collect(),
            releases: d
                .releases
                .iter()
                .map(|r| ReleaseCount {
                    release: r.release.clone(),
                    count: r.count,
                    first_seen: time(r.first_seen),
                    last_seen: time(r.last_seen),
                })
                .collect(),
            latest: d.latest.as_ref().map(Event::from),
        }
    }
}

/// One issue with its counts and latest event.
#[utoipa::path(
    get,
    path = "/issues/{id}",
    tag = "issues",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = OK, body = IssueDetailDto), (status = NOT_FOUND, body = Problem))
)]
async fn get_issue(
    auth: Auth,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<IssueDetailDto>, ApiError> {
    auth.require(Permission::ReadData)?;
    let detail = state.issues.get.execute(IssueId::new(id)).await?;
    Ok(Json(IssueDetailDto::from(&detail)))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueAction {
    Resolve,
    /// Events of the latest release seen do not bring it back.
    ResolveNextRelease,
    Mute,
    Reopen,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeIssueRequest {
    pub action: IssueAction,
    /// With `mute`: for this many minutes.
    #[serde(default)]
    pub mute_minutes: Option<u64>,
    /// With `mute`: until this many more events came; the last of them
    /// brings it back.
    #[serde(default)]
    pub mute_events: Option<u32>,
}

/// Resolves, mutes or reopens an issue.
#[utoipa::path(
    patch,
    path = "/issues/{id}",
    tag = "issues",
    security(("session" = [])),
    security(("token" = [])),
    params(("id" = i64, Path)),
    request_body = ChangeIssueRequest,
    responses((status = OK, body = Issue), (status = NOT_FOUND, body = Problem))
)]
async fn change_issue(
    auth: TokenWrites,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<ChangeIssueRequest>,
) -> Result<Json<Issue>, ApiError> {
    auth.require(Permission::TriageIssues)?;
    let change = match body.action {
        IssueAction::Resolve => StatusChange::Resolve,
        IssueAction::ResolveNextRelease => StatusChange::ResolveInNextRelease,
        IssueAction::Mute => StatusChange::Mute {
            for_duration: body.mute_minutes.map(|m| Duration::from_secs(m.saturating_mul(60))),
            events: body.mute_events,
        },
        IssueAction::Reopen => StatusChange::Reopen,
    };
    let issue = state.issues.change.execute(&auth.0.actor(), IssueId::new(id), change, &client).await?;
    Ok(Json(Issue::from(&issue)))
}

/// Deletes an issue; its events go with their segments.
#[utoipa::path(
    delete,
    path = "/issues/{id}",
    tag = "issues",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = NO_CONTENT), (status = NOT_FOUND, body = Problem))
)]
async fn delete_issue(
    auth: Auth,
    State(state): State<ApiState>,
    Client(client): Client,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::ManageProjects)?;
    state.issues.delete.execute(&auth.0.actor(), IssueId::new(id), &client).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventListItem {
    pub event_id: String,
    pub issue_id: i64,
    pub project_id: i64,
    pub environment_id: i64,
    pub time: String,
    pub level: String,
    pub release: Option<String>,
    pub message: String,
    pub transaction: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
    pub trace_id: Option<String>,
}

impl From<&EventSummary> for EventListItem {
    fn from(e: &EventSummary) -> Self {
        Self {
            event_id: e.event_id.clone(),
            issue_id: e.issue.get(),
            project_id: e.scope.project.get(),
            environment_id: e.scope.environment.get(),
            time: time(e.time),
            level: e.level.as_str().to_owned(),
            release: e.release.clone(),
            message: e.message.clone(),
            transaction: e.transaction.clone(),
            service: e.service.clone(),
            host: e.host.clone(),
            trace_id: e.trace_id.clone(),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventPage {
    /// Events older than this time, RFC 3339.
    pub before: Option<String>,
    pub limit: Option<u32>,
}

/// An issue's events, newest first.
#[utoipa::path(
    get,
    path = "/issues/{id}/events",
    tag = "issues",
    security(("session" = [])),
    params(("id" = i64, Path), EventPage),
    responses((status = OK, body = Vec<EventListItem>))
)]
async fn issue_events(
    auth: Auth,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    ApiQuery(page): ApiQuery<EventPage>,
) -> Result<Json<Vec<EventListItem>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let before = match page.before.as_deref().filter(|b| !b.is_empty()) {
        Some(text) => Some(
            Timestamp::parse_rfc3339(text).ok_or_else(|| ApiError::BadRequest("before is no time".into()))?,
        ),
        None => None,
    };
    let events = state.issues.events.execute(IssueId::new(id), before, page.limit.unwrap_or(20)).await?;
    Ok(Json(events.iter().map(EventListItem::from).collect()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TagValueDto {
    pub value: String,
    pub count: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TagSummaryDto {
    /// `release`, `transaction`, `service` or `host`.
    pub key: String,
    /// Events that had the tag.
    pub total: u32,
    /// The ten most frequent values.
    pub values: Vec<TagValueDto>,
}

/// Where an issue's recent events (the last 1,000) came from: release,
/// route or job, service and host, each value with how often.
#[utoipa::path(
    get,
    path = "/issues/{id}/tags",
    tag = "issues",
    security(("session" = [])),
    params(("id" = i64, Path)),
    responses((status = OK, body = Vec<TagSummaryDto>))
)]
async fn issue_tags(
    auth: Auth,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<TagSummaryDto>>, ApiError> {
    auth.require(Permission::ReadData)?;
    let tags = state.issues.tags.execute(IssueId::new(id)).await?;
    Ok(Json(
        tags.into_iter()
            .map(|tag| TagSummaryDto {
                key: tag.key.to_owned(),
                total: tag.total,
                values: tag
                    .values
                    .into_iter()
                    .map(|v| TagValueDto { value: v.value, count: v.count })
                    .collect(),
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StoredEventDto {
    pub issue_id: i64,
    pub project_id: i64,
    pub environment_id: i64,
    pub received: String,
    pub event: Event,
}

/// One event by the SDK's event ID, as Sentry's SDKs and Bugsink name it.
#[utoipa::path(
    get,
    path = "/events/{event_id}",
    tag = "issues",
    security(("session" = [])),
    params(("event_id" = String, Path)),
    responses((status = OK, body = StoredEventDto), (status = NOT_FOUND, body = Problem))
)]
async fn get_event(
    auth: Auth,
    State(state): State<ApiState>,
    Path(event_id): Path<String>,
) -> Result<Json<StoredEventDto>, ApiError> {
    auth.require(Permission::ReadData)?;
    let stored = state.issues.event.execute(&event_id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(StoredEventDto {
        issue_id: stored.issue.get(),
        project_id: stored.scope.project.get(),
        environment_id: stored.scope.environment.get(),
        received: time(stored.received),
        event: Event::from(&stored.event),
    }))
}
