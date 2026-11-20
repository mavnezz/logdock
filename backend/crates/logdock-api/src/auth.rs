//! Session cookies, CSRF protection, API tokens and the extractors that
//! authenticate a request.
//!
//! A handler is protected by taking [`Auth`] as an argument. A feature test
//! checks that every route not explicitly public answers 401 without a
//! session.
//!
//! An API token (`Authorization: Bearer ldt_…`) acts for its account, within
//! its scope. It reads; it changes something only through a handler that
//! takes [`TokenWrites`] instead of [`Auth`], and that handler's permission
//! must be in the token's scope. Tokens need no CSRF token: a browser never
//! sends them by itself.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use logdock_identity::application::{AuthContext, SignedIn};
use logdock_identity::domain::Permission;
use logdock_kernel::{Secret, constant_time_eq};

use crate::error::ApiError;
use crate::state::ApiState;

/// Header carrying the session's CSRF token on state-changing requests.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// With `Secure`, the `__Host-` prefix makes browsers refuse the cookie
/// unless it is host-only, `Secure` and for path `/`.
fn cookie_name(secure: bool) -> &'static str {
    if secure { "__Host-logdock_session" } else { "logdock_session" }
}

/// The cookie that carries a new session's token. It has no `Max-Age`, so it
/// ends with the browser session; the idle timeout and the lifetime are
/// enforced on the server.
pub(crate) fn session_cookie(signed_in: &SignedIn, secure: bool) -> Cookie<'static> {
    Cookie::build((cookie_name(secure), signed_in.token.expose().to_owned()))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Strict)
        .path("/")
        .build()
}

/// Tells the browser to drop the session cookie.
pub(crate) fn removal_cookie(secure: bool) -> Cookie<'static> {
    let mut cookie =
        Cookie::build((cookie_name(secure), "")).http_only(true).secure(secure).path("/").build();
    cookie.make_removal();
    cookie
}

/// Rejects requests a browser sent on behalf of another site. Needed even
/// before sign-in: otherwise a foreign page could sign the victim into an
/// account the attacker controls.
pub(crate) fn check_same_origin(headers: &HeaderMap) -> Result<(), ApiError> {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    if let Some(site) = header("sec-fetch-site")
        && !matches!(site, "same-origin" | "none")
    {
        return Err(ApiError::CsrfRejected);
    }
    if let Some(origin) = header(header::ORIGIN.as_str()) {
        let origin_host = origin.split_once("://").map(|(_, host)| host);
        let host = header("x-forwarded-host").or(header(header::HOST.as_str()));
        if origin_host.is_none() || origin_host != host {
            return Err(ApiError::CsrfRejected);
        }
    }
    Ok(())
}

/// Refuses a sign-in whose session cookie the browser would drop: a `Secure`
/// cookie set over plain HTTP, from anywhere but a loopback address. The
/// sign-in would seem to work and the next request would find no session.
pub(crate) fn check_cookie_kept(headers: &HeaderMap, secure: bool) -> Result<(), ApiError> {
    let origin = headers.get(header::ORIGIN).and_then(|value| value.to_str().ok());
    match origin.and_then(|origin| origin.strip_prefix("http://")) {
        Some(host) if secure && !is_loopback(host) => Err(ApiError::Rejected {
            status: StatusCode::CONFLICT,
            code: "insecure_connection",
            detail: Some(
                "Logdock sets secure cookies, which browsers keep over HTTPS only. Reach it over HTTPS, \
                 or set LOGDOCK_PUBLIC_URL to its http:// address."
                    .into(),
            ),
        }),
        _ => Ok(()),
    }
}

/// `localhost`, a name under it, or a loopback address, with or without a
/// port: browsers keep secure cookies from these over plain HTTP too.
fn is_loopback(authority: &str) -> bool {
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split_once(']').map_or(rest, |(host, _)| host),
        None => authority.split_once(':').map_or(authority, |(host, _)| host),
    };
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// The session token from the request's cookie, if any.
pub(crate) fn session_token(headers: &HeaderMap, secure: bool) -> Option<Secret> {
    CookieJar::from_headers(headers).get(cookie_name(secure)).map(|cookie| Secret::new(cookie.value()))
}

/// The API token in `Authorization: Bearer …`, if any.
fn bearer_token(headers: &HeaderMap) -> Option<Secret> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| Secret::new(token.trim()))
}

/// The caller of a request that carries an API token; `None` without one.
pub(crate) async fn token_caller(
    headers: &HeaderMap,
    state: &ApiState,
) -> Option<Result<AuthContext, ApiError>> {
    let token = bearer_token(headers)?;
    Some(state.identity.authenticate_token.execute(&token).await.map_err(ApiError::from))
}

async fn authenticate(parts: &Parts, state: &ApiState, token_writes: bool) -> Result<AuthContext, ApiError> {
    if let Some(caller) = token_caller(&parts.headers, state).await {
        let auth = caller?;
        if !parts.method.is_safe() && !token_writes {
            return Err(ApiError::Rejected {
                status: axum::http::StatusCode::FORBIDDEN,
                code: "token_read_only",
                detail: Some("API tokens read; only issues' state can be changed with one".into()),
            });
        }
        return Ok(auth);
    }
    let secure = state.config.secure_cookies;
    let token = session_token(&parts.headers, secure).ok_or(ApiError::Unauthenticated)?;
    let auth = state.identity.authenticate.execute(&token).await?;
    if !parts.method.is_safe() {
        check_same_origin(&parts.headers)?;
        let sent = parts.headers.get(CSRF_HEADER).map(|value| value.as_bytes()).unwrap_or_default();
        let expected = auth.session().map(|s| s.csrf_token.expose().as_bytes()).unwrap_or_default();
        if expected.is_empty() || !constant_time_eq(sent, expected) {
            return Err(ApiError::CsrfRejected);
        }
    }
    Ok(auth)
}

/// The signed-in caller. Rejects the request unless it carries a valid
/// session and, if it changes something, the session's CSRF token.
pub struct Auth(pub AuthContext);

impl Auth {
    pub fn require(&self, permission: Permission) -> Result<(), ApiError> {
        if self.0.can(permission) { Ok(()) } else { Err(ApiError::Forbidden) }
    }
}

impl FromRequestParts<ApiState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &ApiState) -> Result<Self, Self::Rejection> {
        authenticate(parts, state, false).await.map(Self)
    }
}

/// As [`Auth`], for the few changes an API token may make: the handler's
/// permission still decides, narrowed to the token's scope.
pub struct TokenWrites(pub AuthContext);

impl TokenWrites {
    pub fn require(&self, permission: Permission) -> Result<(), ApiError> {
        if self.0.can(permission) { Ok(()) } else { Err(ApiError::Forbidden) }
    }
}

impl FromRequestParts<ApiState> for TokenWrites {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &ApiState) -> Result<Self, Self::Rejection> {
        authenticate(parts, state, true).await.map(Self)
    }
}

/// Requests that sign someone in must come from our own pages, over a
/// connection the session cookie survives.
pub struct SameOrigin;

impl FromRequestParts<ApiState> for SameOrigin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &ApiState) -> Result<Self, Self::Rejection> {
        check_same_origin(&parts.headers)?;
        check_cookie_kept(&parts.headers, state.config.secure_cookies)?;
        Ok(Self)
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn refuses_cross_site_requests() {
        assert!(check_same_origin(&headers(&[])).is_ok());
        assert!(check_same_origin(&headers(&[("sec-fetch-site", "same-origin")])).is_ok());
        assert!(check_same_origin(&headers(&[("sec-fetch-site", "cross-site")])).is_err());
        let same = headers(&[("origin", "https://logs.example.com"), ("host", "logs.example.com")]);
        assert!(check_same_origin(&same).is_ok());
        let other = headers(&[("origin", "https://evil.example"), ("host", "logs.example.com")]);
        assert!(check_same_origin(&other).is_err());
        let forwarded = headers(&[
            ("origin", "https://logs.example.com"),
            ("host", "logdock:8080"),
            ("x-forwarded-host", "logs.example.com"),
        ]);
        assert!(check_same_origin(&forwarded).is_ok());
        assert!(check_same_origin(&headers(&[("origin", "null"), ("host", "x")])).is_err());
    }

    #[test]
    fn refuses_a_sign_in_whose_secure_cookie_the_browser_would_drop() {
        let from = |origin| headers(&[("origin", origin)]);
        let refused = check_cookie_kept(&from("http://192.168.1.20:8080"), true).unwrap_err();
        assert!(matches!(refused, ApiError::Rejected { code: "insecure_connection", .. }));
        assert!(check_cookie_kept(&from("http://logs.example.com"), true).is_err());
        assert!(check_cookie_kept(&from("http://192.168.1.20:8080"), false).is_ok());
        for kept in [
            "https://logs.example.com",
            "http://localhost:18480",
            "http://LOCALHOST",
            "http://logdock.localhost:8080",
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
        ] {
            assert!(check_cookie_kept(&from(kept), true).is_ok(), "{kept}");
        }
        assert!(check_cookie_kept(&headers(&[]), true).is_ok());
    }

    #[test]
    fn names_the_cookie_by_whether_it_is_secure() {
        let cookie = removal_cookie(true);
        assert_eq!(cookie.name(), "__Host-logdock_session");
        assert_eq!(removal_cookie(false).name(), "logdock_session");
        let jar = headers(&[("cookie", "logdock_session=abc")]);
        assert_eq!(session_token(&jar, false).map(|t| t.expose().to_owned()).as_deref(), Some("abc"));
        assert!(session_token(&jar, true).is_none());
    }
}
