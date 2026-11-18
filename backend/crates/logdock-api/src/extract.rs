//! Extractors shared by the handlers.

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Request};
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use logdock_kernel::RequestContext;
use serde::de::DeserializeOwned;

use crate::error::ApiError;
use crate::state::ApiState;

/// `axum::Json`, but failures are answered as problem details.
pub struct ApiJson<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match <axum::Json<T> as FromRequest<S>>::from_request(request, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(rejection) => {
                Err(ApiError::InvalidBody { status: rejection.status(), detail: rejection.body_text() })
            }
        }
    }
}

/// `axum::extract::Query`, but failures are answered as problem details.
pub struct ApiQuery<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(Self(value)),
            Err(rejection) => Err(ApiError::BadRequest(rejection.body_text())),
        }
    }
}

/// `axum::extract::Path`, but failures (an id that is no number) are
/// answered as problem details. Routes import it as `Path`.
pub struct ApiPath<T>(pub T);

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for ApiPath<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Self(value)),
            Err(rejection) => Err(ApiError::BadRequest(rejection.body_text())),
        }
    }
}

/// A network, such as `10.0.0.0/8`, whose proxies are believed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    address: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.address, normalize(ip)) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX.checked_shl(32 - u32::from(self.prefix)).unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX.checked_shl(128 - u32::from(self.prefix)).unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// IPv4 addresses mapped into IPv6 (`::ffff:10.0.0.1`) as IPv4.
fn normalize(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

impl FromStr for Cidr {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = address.trim().parse().map_err(|_| format!("{text:?} is no address"))?;
        let address = normalize(address);
        let max = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => {
                prefix.trim().parse::<u8>().map_err(|_| format!("{text:?} has no valid prefix"))?
            }
            None => max,
        };
        if prefix > max {
            return Err(format!("{text:?} has a prefix longer than {max}"));
        }
        Ok(Self { address, prefix })
    }
}

/// The client's address: the peer, or, when the peer is a trusted proxy,
/// the last address in `X-Forwarded-For` that is not itself one.
pub(crate) fn client_ip(peer: Option<IpAddr>, forwarded: Option<&str>, trusted: &[Cidr]) -> Option<IpAddr> {
    let peer = peer.map(normalize);
    let is_trusted = |ip: IpAddr| trusted.iter().any(|net| net.contains(ip));
    let peer_ip = peer?;
    if !is_trusted(peer_ip) {
        return Some(peer_ip);
    }
    let hops =
        forwarded.into_iter().flat_map(|value| value.split(',')).filter_map(|hop| hop.trim().parse().ok());
    let hops: Vec<IpAddr> = hops.map(normalize).collect();
    hops.iter().rev().copied().find(|ip| !is_trusted(*ip)).or_else(|| hops.first().copied()).or(Some(peer_ip))
}

/// Where the request came from, for audit and throttling.
pub struct Client(pub RequestContext);

const MAX_USER_AGENT_CHARS: usize = 256;

impl FromRequestParts<ApiState> for Client {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &ApiState) -> Result<Self, Self::Rejection> {
        let header = |name| parts.headers.get(name).and_then(|value| value.to_str().ok());
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|ConnectInfo(addr)| addr.ip());
        let client_ip = client_ip(peer, header("x-forwarded-for"), &state.config.trusted_proxies);
        Ok(Client(RequestContext {
            client_ip,
            user_agent: header(USER_AGENT.as_str()).map(|ua| ua.chars().take(MAX_USER_AGENT_CHARS).collect()),
            request_id: header("x-request-id").map(str::to_owned),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn reads_networks() {
        let net: Cidr = "10.0.0.0/8".parse().unwrap();
        assert!(net.contains(ip("10.1.2.3")));
        assert!(net.contains(ip("::ffff:10.1.2.3")));
        assert!(!net.contains(ip("11.0.0.1")));
        assert!(!net.contains(ip("fd00::1")));
        let single: Cidr = "192.168.1.5".parse().unwrap();
        assert!(single.contains(ip("192.168.1.5")));
        assert!(!single.contains(ip("192.168.1.6")));
        let v6: Cidr = "fd00::/8".parse().unwrap();
        assert!(v6.contains(ip("fd12::1")));
        let all: Cidr = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains(ip("8.8.8.8")));
        for bad in ["10.0.0.0/33", "nonsense", "10.0.0.0/x"] {
            assert!(bad.parse::<Cidr>().is_err(), "{bad}");
        }
    }

    #[test]
    fn believes_forwarded_addresses_from_trusted_proxies_only() {
        let trusted: Vec<Cidr> = vec!["172.16.0.0/12".parse().unwrap()];
        let proxy = Some(ip("172.18.0.2"));
        assert_eq!(client_ip(proxy, Some("203.0.113.9"), &trusted), Some(ip("203.0.113.9")));
        // A client cannot put itself in front of the real address.
        assert_eq!(
            client_ip(proxy, Some("1.1.1.1, 203.0.113.9, 172.18.0.3"), &trusted),
            Some(ip("203.0.113.9"))
        );
        assert_eq!(client_ip(proxy, None, &trusted), proxy);
        let outside = Some(ip("198.51.100.7"));
        assert_eq!(client_ip(outside, Some("203.0.113.9"), &trusted), outside);
        assert_eq!(client_ip(None, Some("203.0.113.9"), &trusted), None);
        assert_eq!(client_ip(proxy, Some("172.18.0.9"), &trusted), Some(ip("172.18.0.9")));
    }
}
