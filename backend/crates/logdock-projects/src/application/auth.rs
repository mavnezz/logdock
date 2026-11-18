use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use logdock_kernel::{PortError, ProjectId, Scope, Timestamp};
use tokio::sync::RwLock;

use super::ProjectPorts;
use crate::domain::{EnvironmentName, KeyId, ProjectName, ProjectSettings, Rejection, Tally};

/// How long a checked key is believed without asking the database again.
/// Revoking a key, changing a project or a stage clears the cache, so this
/// only spares the database between changes.
const CACHE_FOR: Duration = Duration::from_secs(60);

/// How often a key's last use is written at most.
const TOUCH_EVERY: Duration = Duration::from_secs(60);

/// What a valid key lets in: where its data goes and the project's rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestScope {
    pub scope: Scope,
    pub key: KeyId,
    pub project_name: ProjectName,
    pub environment_name: EnvironmentName,
    pub settings: ProjectSettings,
}

#[derive(Debug, Clone)]
struct Cached {
    scope: IngestScope,
    checked_at: Timestamp,
    touched_at: Option<Timestamp>,
}

/// Keys checked recently, by their hash. Shared by the use cases that
/// change what a key stands for.
#[derive(Debug, Clone, Default)]
pub struct KeyCache(Arc<RwLock<HashMap<String, Cached>>>);

impl KeyCache {
    pub async fn clear(&self) {
        self.0.write().await.clear();
    }
}

/// Query: the scope an ingest key stands for, sent to `project`'s endpoint.
/// Refuses unknown and revoked keys and keys of another project.
#[derive(Clone)]
pub struct AuthenticateKey {
    ports: Arc<ProjectPorts>,
    cache: KeyCache,
}

impl AuthenticateKey {
    pub(crate) fn new(ports: Arc<ProjectPorts>, cache: KeyCache) -> Self {
        Self { ports, cache }
    }

    /// `Ok(None)` for a key that does not let in; `Err` only when the
    /// database fails.
    pub async fn execute(
        &self,
        project: Option<ProjectId>,
        key: &str,
    ) -> Result<Option<IngestScope>, PortError> {
        let p = &self.ports;
        if key.is_empty() || key.len() > 128 {
            return Ok(None);
        }
        let now = p.clock.now();
        let hash = p.tokens.hash(key);
        if let Some(cached) = self.cache.0.read().await.get(hash.as_str())
            && now.duration_since(cached.checked_at) < CACHE_FOR
        {
            let scope = cached.scope.clone();
            let needs_touch = cached.touched_at.is_none_or(|at| now.duration_since(at) >= TOUCH_EVERY);
            if needs_touch {
                self.touch(hash.as_str(), scope.key, now).await?;
            }
            return Ok(project.is_none_or(|p| p == scope.scope.project).then_some(scope));
        }

        let Some(stored) = p.keys.find_by_hash(&hash).await? else { return Ok(None) };
        if !stored.is_active() {
            return Ok(None);
        }
        let Some(found) = p.projects.find(stored.project).await? else { return Ok(None) };
        let Some(environment) = found.environment(stored.environment) else { return Ok(None) };
        let scope = IngestScope {
            scope: Scope { project: stored.project, environment: stored.environment },
            key: stored.id,
            project_name: found.name.clone(),
            environment_name: environment.name.clone(),
            settings: found.settings.clone(),
        };
        self.cache.0.write().await.insert(
            hash.as_str().to_owned(),
            Cached { scope: scope.clone(), checked_at: now, touched_at: None },
        );
        self.touch(hash.as_str(), stored.id, now).await?;
        Ok(project.is_none_or(|p| p == scope.scope.project).then_some(scope))
    }

    async fn touch(&self, hash: &str, key: KeyId, now: Timestamp) -> Result<(), PortError> {
        self.ports.keys.touch(key, now).await?;
        if let Some(cached) = self.cache.0.write().await.get_mut(hash) {
            cached.touched_at = Some(now);
        }
        Ok(())
    }
}

/// Command: adds what one ingest request brought to the project's counts.
#[derive(Clone)]
pub struct RecordIngest {
    ports: Arc<ProjectPorts>,
}

impl RecordIngest {
    pub(crate) fn new(ports: Arc<ProjectPorts>) -> Self {
        Self { ports }
    }

    pub async fn execute(&self, project: ProjectId, key: Option<KeyId>, tally: &Tally) {
        if tally.is_empty() {
            return;
        }
        if let Some((reason, _)) = tally.rejected.first() {
            tracing::debug!(project = project.get(), reason = reason.as_str(), detail = ?tally.detail, "ingest refused data");
        }
        self.ports.counter.record(project, key, tally, self.ports.clock.now()).await;
    }

    /// A refusal before any key was checked.
    pub async fn refused(&self, project: ProjectId, reason: Rejection, detail: &str) {
        self.execute(project, None, &Tally::rejected(reason, 1, detail)).await;
    }
}
