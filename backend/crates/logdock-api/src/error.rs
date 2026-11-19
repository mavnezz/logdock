use std::error::Error;
use std::time::Duration;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

/// Error body as defined by RFC 9457 (`application/problem+json`).
#[derive(Debug, Serialize, ToSchema)]
pub struct Problem {
    /// Always `about:blank`; `code` identifies the problem.
    #[serde(rename = "type")]
    pub kind: String,
    /// Reason phrase of the status code.
    pub title: String,
    pub status: u16,
    /// Human-readable explanation, if there is more to say than the title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Stable, machine-readable identifier the frontend relies on, e.g. `not_found`.
    pub code: String,
}

/// Every error a handler can return. Internal details are logged, never sent
/// to the client.
#[derive(Debug)]
pub enum ApiError {
    NotFound,
    MethodNotAllowed,
    /// The request is malformed in a way the other variants do not cover.
    BadRequest(String),
    /// The request body could not be read as the expected JSON.
    InvalidBody {
        status: StatusCode,
        detail: String,
    },
    /// The input was understood but breaks a rule; `detail` says which.
    InvalidInput(String),
    Unauthenticated,
    Forbidden,
    /// Missing or wrong CSRF token, or a cross-site request.
    CsrfRejected,
    /// Too many requests or attempts; try again after the given time.
    TooManyRequests {
        code: &'static str,
        retry_after: Duration,
    },
    /// Logdock cannot take this now (a full disk, a writer that stalled).
    Unavailable {
        code: &'static str,
        retry_after: Option<Duration>,
    },
    /// A rule of a context refused the request; `code` names which.
    Rejected {
        status: StatusCode,
        code: &'static str,
        detail: Option<String>,
    },
    Internal(Box<dyn Error + Send + Sync>),
}

impl ApiError {
    pub fn internal(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self::Internal(error.into())
    }

    pub fn conflict(code: &'static str) -> Self {
        Self::Rejected { status: StatusCode::CONFLICT, code, detail: None }
    }

    fn parts(&self) -> (StatusCode, &'static str, Option<String>) {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", None),
            Self::MethodNotAllowed => (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", None),
            Self::BadRequest(detail) => (StatusCode::BAD_REQUEST, "bad_request", Some(detail.clone())),
            Self::InvalidBody { status, detail } => (*status, "invalid_body", Some(detail.clone())),
            Self::InvalidInput(detail) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_input", Some(detail.clone()))
            }
            Self::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated", None),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", None),
            Self::CsrfRejected => (StatusCode::FORBIDDEN, "csrf_rejected", None),
            Self::TooManyRequests { code, .. } => (StatusCode::TOO_MANY_REQUESTS, code, None),
            Self::Unavailable { code, .. } => (StatusCode::SERVICE_UNAVAILABLE, code, None),
            Self::Rejected { status, code, detail } => (*status, code, detail.clone()),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal", None),
        }
    }
}

impl From<logdock_kernel::PortError> for ApiError {
    fn from(error: logdock_kernel::PortError) -> Self {
        Self::internal(error)
    }
}

impl From<logdock_identity::application::IdentityError> for ApiError {
    fn from(error: logdock_identity::application::IdentityError) -> Self {
        use logdock_identity::application::IdentityError as E;
        let rejected = |status, code| Self::Rejected { status, code, detail: None };
        match error {
            E::InvalidAccount(e) => Self::InvalidInput(e.to_string()),
            E::PasswordRejected(e) => Self::InvalidInput(e.to_string()),
            E::InvalidCredentials => rejected(StatusCode::UNAUTHORIZED, "invalid_credentials"),
            E::WrongPassword => rejected(StatusCode::FORBIDDEN, "wrong_password"),
            E::TooManyAttempts { retry_after } => {
                Self::TooManyRequests { code: "too_many_attempts", retry_after }
            }
            E::SetupCompleted => rejected(StatusCode::CONFLICT, "setup_completed"),
            E::InvalidSetupCode => rejected(StatusCode::FORBIDDEN, "invalid_setup_code"),
            E::Unauthenticated => Self::Unauthenticated,
            E::Forbidden => Self::Forbidden,
            E::AccountNotFound => Self::NotFound,
            E::EmailTaken => Self::conflict("email_taken"),
            E::LastAdmin => Self::conflict("last_admin"),
            E::OwnAccount => Self::conflict("own_account"),
            E::Port(e) => Self::internal(e),
        }
    }
}

impl From<logdock_projects::application::ProjectError> for ApiError {
    fn from(error: logdock_projects::application::ProjectError) -> Self {
        use logdock_projects::application::ProjectError as E;
        match error {
            E::Invalid(e) => Self::InvalidInput(e.to_string()),
            E::NameTaken => Self::conflict("name_taken"),
            E::NotFound | E::EnvironmentNotFound | E::KeyNotFound => Self::NotFound,
            E::EnvironmentTaken => Self::conflict("environment_taken"),
            E::EnvironmentInUse => Self::conflict("environment_in_use"),
            E::MailNotConfigured => Self::conflict("mail_not_configured"),
            E::NoRecipients => Self::conflict("no_recipients"),
            E::MailFailed(detail) => {
                Self::Rejected { status: StatusCode::BAD_GATEWAY, code: "mail_failed", detail: Some(detail) }
            }
            E::Port(e) => Self::internal(e),
        }
    }
}

impl From<logdock_performance::application::PerformanceError> for ApiError {
    fn from(error: logdock_performance::application::PerformanceError) -> Self {
        use logdock_performance::application::PerformanceError as E;
        match error {
            E::Unreadable => Self::BadRequest("the transaction has no start or end".into()),
            E::InvalidQuery(detail) => Self::BadRequest(detail.into()),
            E::Port(e) => Self::internal(e),
        }
    }
}

impl From<logdock_monitors::application::MonitorError> for ApiError {
    fn from(error: logdock_monitors::application::MonitorError) -> Self {
        use logdock_monitors::application::MonitorError as E;
        match error {
            E::Unreadable => Self::BadRequest("the check-in has no monitor slug or status".into()),
            E::NotFound => Self::NotFound,
            E::Port(e) => Self::internal(e),
        }
    }
}

impl From<logdock_issues::application::IssueError> for ApiError {
    fn from(error: logdock_issues::application::IssueError) -> Self {
        use logdock_issues::application::IssueError as E;
        match error {
            E::NoEventId => Self::BadRequest("the event has no ID".into()),
            E::NotFound => Self::NotFound,
            E::Port(e) => Self::internal(e),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::Internal(error) = &self {
            let mut chain = error.to_string();
            let mut source = error.source();
            while let Some(cause) = source {
                chain.push_str(": ");
                chain.push_str(&cause.to_string());
                source = cause.source();
            }
            tracing::error!(error = %chain, "request failed");
        }
        let (status, code, detail) = self.parts();
        let body = Problem {
            kind: "about:blank".to_owned(),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            detail,
            code: code.to_owned(),
        };
        let mut response = (status, Json(body)).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/problem+json"));
        let retry_after = match &self {
            Self::TooManyRequests { retry_after, .. } => Some(*retry_after),
            Self::Unavailable { retry_after, .. } => *retry_after,
            _ => None,
        };
        if let Some(after) = retry_after {
            let seconds = after.as_secs().max(1);
            headers.insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;

    use super::*;

    async fn answer(error: ApiError) -> (StatusCode, Option<String>, serde_json::Value) {
        let response = error.into_response();
        let status = response.status();
        let retry = response.headers().get(header::RETRY_AFTER).map(|v| v.to_str().unwrap().to_owned());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, retry, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn answers_with_problem_details() {
        let (status, _, body) = answer(ApiError::NotFound).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "not_found");
        assert_eq!(body["type"], "about:blank");
        assert_eq!(body["title"], "Not Found");

        let (status, retry, body) = answer(ApiError::TooManyRequests {
            code: "rate_limited",
            retry_after: Duration::from_millis(10),
        })
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(retry.as_deref(), Some("1"));
        assert_eq!(body["code"], "rate_limited");

        let (status, _, body) = answer(ApiError::InvalidInput("too long".into())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["detail"], "too long");
    }

    #[tokio::test]
    async fn never_tells_internal_details() {
        let error = ApiError::from(logdock_kernel::PortError::new("log storage", "secret path /data/x"));
        let (status, _, body) = answer(error).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "internal");
        assert!(!body.to_string().contains("secret"));
        let (status, retry, _) =
            answer(ApiError::Unavailable { code: "low_disk", retry_after: Some(Duration::from_secs(30)) })
                .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(retry.as_deref(), Some("30"));
        let (status, _, body) = answer(ApiError::conflict("name_taken")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "name_taken");
    }
}
