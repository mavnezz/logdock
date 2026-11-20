//! HTTP API of Logdock.
//!
//! Every route of the web interface's API is registered through
//! [`utoipa_axum`], so the OpenAPI document ([`openapi`]) always matches the
//! router ([`router`]). The frontend's TypeScript types are generated from
//! that document. Everything that is not the API is the web interface's
//! static files.

mod auth;
mod dto;
mod error;
mod extract;
mod ingest;
mod mcp;
mod routes;
mod state;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tower::ServiceExt;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;

pub use auth::CSRF_HEADER;
pub use error::{ApiError, Problem};
pub use extract::Cidr;
pub use ingest::IngestConfig;
pub use state::{ApiConfig, ApiState, UseCases};

/// Prefix of the current API version.
pub const API_V1: &str = "/api/v1";

#[derive(OpenApi)]
#[openapi(
    info(title = "Logdock API", description = "API of Logdock's web interface."),
    tags(
        (name = "system", description = "Health, version and storage"),
        (name = "auth", description = "The first setup, sign-in, the session and the caller's own account"),
        (name = "accounts", description = "Accounts and the audit log, for administrators"),
        (name = "projects", description = "Projects, their stages, ingest keys, settings and ingest counts"),
        (name = "logs", description = "Log entries: search, one entry, counts over time, sources"),
        (name = "issues", description = "Error events grouped into issues and their states"),
        (name = "traces", description = "Everything that carries one trace ID"),
        (name = "performance", description = "Requests, jobs, commands, queries, outgoing requests and cache over time"),
        (name = "monitors", description = "Scheduled tasks by their check-ins"),
        (name = "releases", description = "What each release brought in errors and how its requests did"),
    ),
    modifiers(&SessionSecurity)
)]
struct ApiDoc;

/// Documents the session cookie and API tokens. Operations that need a
/// session say `security(("session" = []))`; those that change something
/// also need the session's CSRF token in `X-CSRF-Token`. An API token works
/// for every operation that reads, and for those that say
/// `security(("token" = []))`.
struct SessionSecurity;

impl Modify for SessionSecurity {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                "__Host-logdock_session",
                "Set by sign-in or setup. Named `logdock_session` where secure cookies are off.",
            ))),
        );
        components.add_security_scheme(
            "token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "An API token (`ldt_…`) from the profile page. It acts for its account and reads \
                         everything a session does; it changes nothing but, with the `triage` scope, issues' \
                         state. No CSRF token is needed.",
                    ))
                    .build(),
            ),
        );
    }
}

fn api() -> OpenApiRouter<ApiState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi()).merge(routes::health::routes()).nest(API_V1, routes::v1())
}

/// The OpenAPI document describing [`router`].
pub fn openapi() -> utoipa::openapi::OpenApi {
    api().split_for_parts().1
}

/// What the browser may load and run: our own files only.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
     script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

/// The complete HTTP application, ready to be served.
pub fn router(state: ApiState) -> Router {
    let (router, openapi) = api().split_for_parts();
    let openapi_json = openapi.to_json().expect("the OpenAPI document serializes");
    // The MCP server's tools read through the API itself.
    let for_mcp = router.clone().with_state(state.clone());

    router
        .route(
            &format!("{API_V1}/openapi.json"),
            get(move || async move { ([(header::CONTENT_TYPE, "application/json")], openapi_json) }),
        )
        .merge(ingest::router(&state))
        .merge(routes::logs::stream_router())
        .merge(mcp::router(for_mcp))
        .fallback(web)
        .with_state(state)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        // Layers wrap everything added before them: the request ID is set
        // first, then the request is traced, and the ID is copied to the
        // response last.
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(TraceLayer::new_for_http().make_span_with(|request: &Request| {
            let request_id =
                request.headers().get("x-request-id").and_then(|value| value.to_str().ok()).unwrap_or("");
            // The path only: query strings may carry search terms.
            tracing::info_span!("request", method = %request.method(), path = %request.uri().path(), request_id)
        }))
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

/// Everything outside the API: the web interface's files, and its
/// `index.html` for the paths its router knows. Unknown API paths stay 404.
async fn web(State(state): State<ApiState>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    if path == "/api" || path.starts_with("/api/") || path.starts_with("/health/") {
        return ApiError::NotFound.into_response();
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return ApiError::MethodNotAllowed.into_response();
    }
    let Some(dir) = state.config.web_dir.clone() else {
        return ApiError::NotFound.into_response();
    };
    let service = ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")));
    let mut response = match service.oneshot(request).await {
        Ok(response) => response.map(axum::body::Body::new),
        Err(never) => match never {},
    };
    // Built assets carry a hash of their content in their name.
    let cache = if path.starts_with("/assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

#[cfg(test)]
mod tests;
