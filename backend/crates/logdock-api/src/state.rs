use std::path::PathBuf;
use std::sync::Arc;

use logdock_retention::application::RetentionUseCases;

use crate::extract::Cidr;

/// How the API is exposed.
#[derive(Debug, Clone, Default)]
pub struct ApiConfig {
    /// Session cookies get `Secure` and the `__Host-` prefix. Off only where
    /// the interface is reached over plain HTTP.
    pub secure_cookies: bool,
    /// Proxies whose `X-Forwarded-For` is believed.
    pub trusted_proxies: Vec<Cidr>,
    /// The built web interface; `None` serves the API alone.
    pub web_dir: Option<PathBuf>,
    /// The address the interface is reached at, for the DSNs it shows.
    pub public_url: Option<String>,
}

/// The use cases of all contexts.
pub struct UseCases {
    pub retention: RetentionUseCases,
}

/// The use cases the handlers call, and how the API is exposed. Cheap to
/// clone.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) retention: RetentionUseCases,
    pub(crate) config: Arc<ApiConfig>,
}

impl ApiState {
    pub fn new(use_cases: UseCases, config: ApiConfig) -> Self {
        Self { retention: use_cases.retention, config: Arc::new(config) }
    }
}
