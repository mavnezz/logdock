use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use logdock_kernel::{PortError, ProjectId, Scope, Timestamp};
use tokio::sync::RwLock;

use super::ProjectPorts;
use crate::domain::{
    Environment, EnvironmentName, KeyId, Project, ProjectName, ProjectSettings, Rejection, Tally,
};

/// How long a checked key is believed without asking the database again.
/// Revoking a key, changing a project or a stage clears the cache, so this
/// only spares the database between changes.
const CACHE_FOR: Duration = Duration::from_secs(60);

/// How often a key's last use is written at most.
const TOUCH_EVERY: Duration = Duration::from_secs(60);

/// The stage data that names none goes to under a key of the whole
/// project: what Sentry's SDKs send when no environment is set.
pub const DEFAULT_STAGE: &str = "production";

/// The most stages a project gets from its data; beyond, data that names
/// another stage is refused, so a client cannot make stages without end.
pub const MAX_STAGES: usize = 50;

/// One stage of a project, where its data goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    pub scope: Scope,
    pub name: EnvironmentName,
}

impl Stage {
    fn of(project: &Project, environment: &Environment) -> Self {
        Self {
            scope: Scope { project: project.id, environment: environment.id },
            name: environment.name.clone(),
        }
    }
}

/// What a valid key lets in: its project, its stage and the project's rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestScope {
    pub project: ProjectId,
    pub key: KeyId,
    pub project_name: ProjectName,
    /// The key's one stage; `None` for a key of the whole project, whose
    /// data names its stage.
    pub stage: Option<Stage>,
    /// The project's stages when the key was checked.
    pub stages: Vec<Stage>,
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
            return Ok(project.is_none_or(|p| p == scope.project).then_some(scope));
        }

        let Some(stored) = p.keys.find_by_hash(&hash).await? else { return Ok(None) };
        if !stored.is_active() {
            return Ok(None);
        }
        let Some(found) = p.projects.find(stored.project).await? else { return Ok(None) };
        let stage = match stored.environment {
            Some(id) => match found.environment(id) {
                Some(environment) => Some(Stage::of(&found, environment)),
                None => return Ok(None),
            },
            None => None,
        };
        let scope = IngestScope {
            project: stored.project,
            key: stored.id,
            project_name: found.name.clone(),
            stage,
            stages: found.environments.iter().map(|e| Stage::of(&found, e)).collect(),
            settings: found.settings.clone(),
        };
        self.cache.0.write().await.insert(
            hash.as_str().to_owned(),
            Cached { scope: scope.clone(), checked_at: now, touched_at: None },
        );
        self.touch(hash.as_str(), stored.id, now).await?;
        Ok(project.is_none_or(|p| p == scope.project).then_some(scope))
    }

    async fn touch(&self, hash: &str, key: KeyId, now: Timestamp) -> Result<(), PortError> {
        self.ports.keys.touch(key, now).await?;
        if let Some(cached) = self.cache.0.write().await.get_mut(hash) {
            cached.touched_at = Some(now);
        }
        Ok(())
    }
}

/// Where data goes, or why it is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageFor {
    Stage(Stage),
    Refused(String),
}

/// Command: the stage of data that names `claimed` (its `environment`), sent
/// with a key. A key of one stage takes that stage's data and data that
/// names none. A key of the whole project takes the stage its data names and
/// adds it to the project the first time; data that names none goes to
/// [`DEFAULT_STAGE`].
#[derive(Clone)]
pub struct ResolveStage {
    ports: Arc<ProjectPorts>,
    cache: KeyCache,
}

impl ResolveStage {
    pub(crate) fn new(ports: Arc<ProjectPorts>, cache: KeyCache) -> Self {
        Self { ports, cache }
    }

    pub async fn execute(&self, scope: &IngestScope, claimed: Option<&str>) -> Result<StageFor, PortError> {
        if let Some(own) = &scope.stage {
            return Ok(match claimed {
                Some(claimed) if !claimed.trim().eq_ignore_ascii_case(own.name.as_str()) => {
                    StageFor::Refused(format!(
                        "data for stage {claimed:?} sent with a key of {:?}",
                        own.name.as_str()
                    ))
                }
                _ => StageFor::Stage(own.clone()),
            });
        }
        let claimed = claimed.unwrap_or(DEFAULT_STAGE);
        let Ok(name) = EnvironmentName::parse(claimed) else {
            return Ok(StageFor::Refused(format!(
                "{claimed:?} is no stage name: up to {} lowercase letters, digits, '.', '_' and '-'",
                EnvironmentName::MAX_LEN
            )));
        };
        if let Some(known) = scope.stages.iter().find(|stage| stage.name == name) {
            return Ok(StageFor::Stage(known.clone()));
        }

        // Named for the first time, or since the key was checked.
        let p = &self.ports;
        let Some(project) = p.projects.find(scope.project).await? else {
            return Ok(StageFor::Refused("the project was deleted".into()));
        };
        if let Some(environment) = project.environment_named(&name) {
            return Ok(StageFor::Stage(Stage::of(&project, environment)));
        }
        if project.environments.len() >= MAX_STAGES {
            return Ok(StageFor::Refused(format!(
                "stage {:?} is new, and the project has {MAX_STAGES} stages already",
                name.as_str()
            )));
        }
        let added = match p.projects.add_environment(project.id, &name, p.clock.now()).await? {
            Some(environment) => environment,
            // Another request added it meanwhile.
            None => {
                match p.projects.find(project.id).await?.and_then(|p| p.environment_named(&name).cloned()) {
                    Some(environment) => environment,
                    None => return Ok(StageFor::Refused("the project was deleted".into())),
                }
            }
        };
        self.cache.clear().await;
        tracing::info!(project = %project.name, stage = %name, "a new stage appeared in the data");
        Ok(StageFor::Stage(Stage::of(&project, &added)))
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
