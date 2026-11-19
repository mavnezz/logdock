use std::path::PathBuf;
use std::sync::Arc;

use logdock_identity::application::IdentityUseCases;
use logdock_issues::application::IssueUseCases;
use logdock_logs::application::LogUseCases;
use logdock_monitors::application::MonitorUseCases;
use logdock_performance::application::PerformanceUseCases;
use logdock_projects::application::ProjectUseCases;
use logdock_retention::application::RetentionUseCases;

use crate::extract::Cidr;
use crate::ingest::{IngestConfig, IngestGuard};

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
    pub ingest: IngestConfig,
}

impl ApiConfig {
    /// The DSN an SDK is configured with for `key` of `project`.
    pub fn dsn(&self, key: &str, project: i64) -> String {
        let url = self.public_url.as_deref().unwrap_or("http://localhost:8080");
        let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
        format!("{scheme}://{key}@{rest}/{project}")
    }
}

/// The use cases of all contexts.
pub struct UseCases {
    pub identity: IdentityUseCases,
    pub issues: IssueUseCases,
    pub logs: LogUseCases,
    pub monitors: MonitorUseCases,
    pub performance: PerformanceUseCases,
    pub projects: ProjectUseCases,
    pub retention: RetentionUseCases,
}

/// The use cases the handlers call, and how the API is exposed. Cheap to
/// clone.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) identity: IdentityUseCases,
    pub(crate) issues: IssueUseCases,
    pub(crate) logs: LogUseCases,
    pub(crate) monitors: MonitorUseCases,
    pub(crate) performance: PerformanceUseCases,
    pub(crate) projects: ProjectUseCases,
    pub(crate) retention: RetentionUseCases,
    pub(crate) config: Arc<ApiConfig>,
    pub(crate) ingest: IngestConfig,
    pub(crate) ingest_guard: Arc<IngestGuard>,
}

impl ApiState {
    pub fn new(use_cases: UseCases, config: ApiConfig) -> Self {
        Self {
            identity: use_cases.identity,
            issues: use_cases.issues,
            logs: use_cases.logs,
            monitors: use_cases.monitors,
            performance: use_cases.performance,
            projects: use_cases.projects,
            retention: use_cases.retention,
            ingest: config.ingest,
            config: Arc::new(config),
            ingest_guard: Arc::new(IngestGuard::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_dsns_from_the_public_url() {
        let config = ApiConfig { public_url: Some("https://logs.example.com".into()), ..Default::default() };
        assert_eq!(config.dsn("abc", 3), "https://abc@logs.example.com/3");
        let sub = ApiConfig { public_url: Some("http://example.com/logdock".into()), ..Default::default() };
        assert_eq!(sub.dsn("abc", 3), "http://abc@example.com/logdock/3");
        assert_eq!(ApiConfig::default().dsn("k", 1), "http://k@localhost:8080/1");
    }
}
