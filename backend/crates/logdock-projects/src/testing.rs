//! In-memory implementations of every port, for tests.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::{
    AuditEntry, AuditLog, Clock, Day, EnvironmentId, Mail, Mailer, PortError, ProjectId, Secret, Timestamp,
};

use crate::application::ports::{IngestCounter, KeyRepository, KeyTokens, ProjectRepository};
use crate::application::{ProjectPorts, ProjectUseCases};
use crate::domain::{
    Environment, EnvironmentName, IngestCount, IngestKey, KeyHash, KeyId, KeyLabel, Project, ProjectName,
    ProjectSettings, Tally,
};

#[derive(Default)]
pub struct MemoryProjects {
    projects: Mutex<Vec<Project>>,
    next: AtomicI64,
}

impl MemoryProjects {
    fn id(&self) -> i64 {
        self.next.fetch_add(1, Ordering::SeqCst) + 1
    }
}

#[async_trait]
impl ProjectRepository for MemoryProjects {
    async fn list(&self) -> Result<Vec<Project>, PortError> {
        let mut projects = self.projects.lock().unwrap().clone();
        projects.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
        Ok(projects)
    }

    async fn find(&self, id: ProjectId) -> Result<Option<Project>, PortError> {
        Ok(self.projects.lock().unwrap().iter().find(|p| p.id == id).cloned())
    }

    async fn create(
        &self,
        name: &ProjectName,
        environments: &[EnvironmentName],
        at: Timestamp,
    ) -> Result<Option<Project>, PortError> {
        if self.projects.lock().unwrap().iter().any(|p| &p.name == name) {
            return Ok(None);
        }
        let project = Project {
            id: ProjectId::new(self.id()),
            name: name.clone(),
            environments: environments
                .iter()
                .map(|name| Environment {
                    id: EnvironmentId::new(self.id()),
                    name: name.clone(),
                    created_at: at,
                })
                .collect(),
            settings: ProjectSettings::default(),
            created_at: at,
        };
        self.projects.lock().unwrap().push(project.clone());
        Ok(Some(project))
    }

    async fn rename(&self, id: ProjectId, name: &ProjectName) -> Result<bool, PortError> {
        let mut projects = self.projects.lock().unwrap();
        if projects.iter().any(|p| &p.name == name && p.id != id) {
            return Ok(false);
        }
        if let Some(project) = projects.iter_mut().find(|p| p.id == id) {
            project.name = name.clone();
        }
        Ok(true)
    }

    async fn update_settings(&self, id: ProjectId, settings: &ProjectSettings) -> Result<(), PortError> {
        if let Some(project) = self.projects.lock().unwrap().iter_mut().find(|p| p.id == id) {
            project.settings = settings.clone();
        }
        Ok(())
    }

    async fn delete(&self, id: ProjectId) -> Result<(), PortError> {
        self.projects.lock().unwrap().retain(|p| p.id != id);
        Ok(())
    }

    async fn add_environment(
        &self,
        id: ProjectId,
        name: &EnvironmentName,
        at: Timestamp,
    ) -> Result<Option<Environment>, PortError> {
        let env_id = self.id();
        let mut projects = self.projects.lock().unwrap();
        let Some(project) = projects.iter_mut().find(|p| p.id == id) else { return Ok(None) };
        if project.environment_named(name).is_some() {
            return Ok(None);
        }
        let environment = Environment { id: EnvironmentId::new(env_id), name: name.clone(), created_at: at };
        project.environments.push(environment.clone());
        Ok(Some(environment))
    }

    async fn remove_environment(&self, id: ProjectId, environment: EnvironmentId) -> Result<(), PortError> {
        if let Some(project) = self.projects.lock().unwrap().iter_mut().find(|p| p.id == id) {
            project.environments.retain(|e| e.id != environment);
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryKeys {
    keys: Mutex<Vec<(IngestKey, KeyHash)>>,
    next: AtomicI64,
    pub touches: AtomicI64,
}

#[async_trait]
impl KeyRepository for MemoryKeys {
    async fn list(&self, project: ProjectId) -> Result<Vec<IngestKey>, PortError> {
        let keys = self.keys.lock().unwrap();
        Ok(keys.iter().rev().filter(|(k, _)| k.project == project).map(|(k, _)| k.clone()).collect())
    }

    async fn create(
        &self,
        project: ProjectId,
        environment: Option<EnvironmentId>,
        label: Option<&KeyLabel>,
        prefix: &str,
        hash: &KeyHash,
        at: Timestamp,
    ) -> Result<IngestKey, PortError> {
        let key = IngestKey {
            id: KeyId::new(self.next.fetch_add(1, Ordering::SeqCst) + 1),
            project,
            environment,
            label: label.cloned(),
            prefix: prefix.to_owned(),
            created_at: at,
            last_used_at: None,
            revoked_at: None,
        };
        self.keys.lock().unwrap().push((key.clone(), hash.clone()));
        Ok(key)
    }

    async fn find_by_hash(&self, hash: &KeyHash) -> Result<Option<IngestKey>, PortError> {
        Ok(self.keys.lock().unwrap().iter().find(|(_, h)| h == hash).map(|(k, _)| k.clone()))
    }

    async fn revoke(&self, id: KeyId, at: Timestamp) -> Result<(), PortError> {
        if let Some((key, _)) = self.keys.lock().unwrap().iter_mut().find(|(k, _)| k.id == id) {
            key.revoked_at = Some(at);
        }
        Ok(())
    }

    async fn touch(&self, id: KeyId, at: Timestamp) -> Result<(), PortError> {
        self.touches.fetch_add(1, Ordering::SeqCst);
        if let Some((key, _)) = self.keys.lock().unwrap().iter_mut().find(|(k, _)| k.id == id) {
            key.last_used_at = Some(at);
        }
        Ok(())
    }
}

/// Keys `key0001…`, hashed by prefixing.
#[derive(Default)]
pub struct CountingKeys(AtomicI64);

impl KeyTokens for CountingKeys {
    fn new_key(&self) -> Secret {
        Secret::new(format!("key{:029}", self.0.fetch_add(1, Ordering::SeqCst) + 1))
    }

    fn hash(&self, key: &str) -> KeyHash {
        KeyHash::new(format!("hash:{key}"))
    }
}

#[derive(Default)]
pub struct MemoryCounter(pub Mutex<Vec<(ProjectId, Option<KeyId>, Tally)>>);

#[async_trait]
impl IngestCounter for MemoryCounter {
    async fn record(&self, project: ProjectId, key: Option<KeyId>, tally: &Tally, _at: Timestamp) {
        self.0.lock().unwrap().push((project, key, tally.clone()));
    }

    async fn counts(&self, project: ProjectId, since: Day) -> Result<Vec<IngestCount>, PortError> {
        let recorded = self.0.lock().unwrap();
        let accepted = recorded.iter().filter(|(p, _, _)| *p == project).map(|(_, _, t)| t.accepted).sum();
        Ok(vec![IngestCount {
            day: since,
            key: None,
            accepted,
            skipped: 0,
            rejected: Vec::new(),
            last_rejection: None,
        }])
    }
}

#[derive(Default)]
pub struct MemoryAudit(pub Mutex<Vec<AuditEntry>>);

#[async_trait]
impl AuditLog for MemoryAudit {
    async fn record(&self, entry: AuditEntry) -> Result<(), PortError> {
        self.0.lock().unwrap().push(entry);
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryMailer(pub Mutex<Vec<Mail>>);

#[async_trait]
impl Mailer for MemoryMailer {
    fn configured(&self) -> bool {
        true
    }

    async fn send(&self, mail: Mail) -> Result<(), PortError> {
        self.0.lock().unwrap().push(mail);
        Ok(())
    }
}

pub struct FixedClock(pub Mutex<Timestamp>);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

impl FixedClock {
    pub fn advance(&self, duration: std::time::Duration) {
        let mut now = self.0.lock().unwrap();
        *now = now.saturating_add(duration);
    }
}

/// The context with every port in memory.
pub struct TestProjects {
    pub use_cases: ProjectUseCases,
    pub keys: Arc<MemoryKeys>,
    pub counter: Arc<MemoryCounter>,
    pub audit: Arc<MemoryAudit>,
    pub mailer: Arc<MemoryMailer>,
    pub clock: Arc<FixedClock>,
}

impl TestProjects {
    pub fn new() -> Self {
        let keys = Arc::new(MemoryKeys::default());
        let counter = Arc::new(MemoryCounter::default());
        let audit = Arc::new(MemoryAudit::default());
        let mailer = Arc::new(MemoryMailer::default());
        let clock = Arc::new(FixedClock(Mutex::new(Timestamp::from_unix_seconds(1_800_000_000))));
        let ports = ProjectPorts {
            projects: Arc::new(MemoryProjects::default()),
            keys: keys.clone(),
            tokens: Arc::new(CountingKeys::default()),
            counter: counter.clone(),
            audit: audit.clone(),
            mailer: mailer.clone(),
            clock: clock.clone(),
        };
        Self { use_cases: ProjectUseCases::new(ports), keys, counter, audit, mailer, clock }
    }
}

impl Default for TestProjects {
    fn default() -> Self {
        Self::new()
    }
}
