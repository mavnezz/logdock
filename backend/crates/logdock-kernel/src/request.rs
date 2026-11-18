use std::net::IpAddr;

/// Where a request came from, for the audit log and throttling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestContext {
    /// The client's address, as the trusted reverse proxy reports it.
    pub client_ip: Option<IpAddr>,
    pub user_agent: Option<String>,
    pub request_id: Option<String>,
}
