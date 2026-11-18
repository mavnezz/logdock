//! In-memory implementations of every port, for tests of this crate and its
//! adapters' callers.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::{AuditEntry, AuditLog, Clock, PortError, Secret, Timestamp};

use crate::application::ports::{AccountRepository, AuditReader, PasswordHasher, SessionRepository, Tokens};
use crate::application::{IdentityConfig, IdentityPorts, IdentityUseCases};
use crate::domain::{
    Account, AccountId, Email, NewAccount, NewSession, PasswordHash, Session, SessionId, SetupCode, TokenHash,
};

pub const SETUP_CODE: &str = "TEST-CODE";

#[derive(Default)]
pub struct MemoryAccounts {
    accounts: Mutex<Vec<Account>>,
    next: AtomicI64,
}

impl MemoryAccounts {
    /// The stored accounts' e-mail addresses, in the order created.
    pub fn emails(&self) -> Vec<String> {
        self.accounts.lock().unwrap().iter().map(|a| a.email.to_string()).collect()
    }
}

#[async_trait]
impl AccountRepository for MemoryAccounts {
    async fn count(&self) -> Result<u64, PortError> {
        Ok(self.accounts.lock().unwrap().len() as u64)
    }

    async fn find_by_id(&self, id: AccountId) -> Result<Option<Account>, PortError> {
        Ok(self.accounts.lock().unwrap().iter().find(|a| a.id == id).cloned())
    }

    async fn find_by_email(&self, email: &Email) -> Result<Option<Account>, PortError> {
        Ok(self.accounts.lock().unwrap().iter().find(|a| &a.email == email).cloned())
    }

    async fn list(&self) -> Result<Vec<Account>, PortError> {
        let mut accounts = self.accounts.lock().unwrap().clone();
        accounts.sort_by(|a, b| a.email.as_str().cmp(b.email.as_str()));
        Ok(accounts)
    }

    async fn create(&self, new: NewAccount) -> Result<Option<Account>, PortError> {
        let mut accounts = self.accounts.lock().unwrap();
        if accounts.iter().any(|a| a.email == new.email) {
            return Ok(None);
        }
        let account = Account {
            id: AccountId::new(self.next.fetch_add(1, Ordering::SeqCst) + 1),
            email: new.email,
            name: new.name,
            role: new.role,
            locale: new.locale,
            password_hash: new.password_hash,
            disabled: false,
            created_at: new.created_at,
            updated_at: new.created_at,
        };
        accounts.push(account.clone());
        Ok(Some(account))
    }

    async fn create_first(&self, account: NewAccount) -> Result<Option<Account>, PortError> {
        if !self.accounts.lock().unwrap().is_empty() {
            return Ok(None);
        }
        self.create(account).await
    }

    async fn update(&self, account: &Account) -> Result<(), PortError> {
        let mut accounts = self.accounts.lock().unwrap();
        if let Some(stored) = accounts.iter_mut().find(|a| a.id == account.id) {
            let hash = stored.password_hash.clone();
            *stored = Account { password_hash: hash, ..account.clone() };
        }
        Ok(())
    }

    async fn update_password(
        &self,
        id: AccountId,
        hash: &PasswordHash,
        at: Timestamp,
    ) -> Result<(), PortError> {
        let mut accounts = self.accounts.lock().unwrap();
        if let Some(stored) = accounts.iter_mut().find(|a| a.id == id) {
            stored.password_hash = hash.clone();
            stored.updated_at = at;
        }
        Ok(())
    }

    async fn delete(&self, id: AccountId) -> Result<(), PortError> {
        self.accounts.lock().unwrap().retain(|a| a.id != id);
        Ok(())
    }
}

#[derive(Default)]
pub struct MemorySessions {
    pub sessions: Mutex<Vec<Session>>,
    next: AtomicI64,
}

#[async_trait]
impl SessionRepository for MemorySessions {
    async fn create(&self, new: NewSession) -> Result<Session, PortError> {
        let session = Session {
            id: SessionId::new(self.next.fetch_add(1, Ordering::SeqCst) + 1),
            account_id: new.account_id,
            token_hash: new.token_hash,
            csrf_token: new.csrf_token,
            created_at: new.created_at,
            last_seen_at: new.created_at,
            client_ip: new.client_ip,
            user_agent: new.user_agent,
        };
        self.sessions.lock().unwrap().push(session.clone());
        Ok(session)
    }

    async fn find_by_token(&self, hash: &TokenHash) -> Result<Option<Session>, PortError> {
        Ok(self.sessions.lock().unwrap().iter().find(|s| &s.token_hash == hash).cloned())
    }

    async fn touch(&self, id: SessionId, at: Timestamp) -> Result<(), PortError> {
        if let Some(session) = self.sessions.lock().unwrap().iter_mut().find(|s| s.id == id) {
            session.last_seen_at = at;
        }
        Ok(())
    }

    async fn delete(&self, id: SessionId) -> Result<(), PortError> {
        self.sessions.lock().unwrap().retain(|s| s.id != id);
        Ok(())
    }

    async fn delete_for_account(
        &self,
        account: AccountId,
        except: Option<SessionId>,
    ) -> Result<(), PortError> {
        self.sessions.lock().unwrap().retain(|s| s.account_id != account || Some(s.id) == except);
        Ok(())
    }

    async fn delete_expired(
        &self,
        idle_before: Timestamp,
        created_before: Timestamp,
    ) -> Result<u64, PortError> {
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|s| s.last_seen_at >= idle_before && s.created_at >= created_before);
        Ok((before - sessions.len()) as u64)
    }
}

/// "Hashes" by prefixing, so tests stay fast and readable.
pub struct PlainHasher;

#[async_trait]
impl PasswordHasher for PlainHasher {
    async fn hash(&self, password: &Secret) -> Result<PasswordHash, PortError> {
        Ok(PasswordHash::new(format!("plain:{}", password.expose())))
    }

    async fn verify(&self, password: &Secret, hash: Option<&PasswordHash>) -> Result<bool, PortError> {
        Ok(hash.is_some_and(|h| h.as_str() == format!("plain:{}", password.expose())))
    }
}

/// Counts tokens up: `token-1`, `csrf-2`, …
#[derive(Default)]
pub struct CountingTokens(AtomicI64);

impl Tokens for CountingTokens {
    fn session_token(&self) -> Secret {
        Secret::new(format!("token-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1))
    }

    fn csrf_token(&self) -> Secret {
        Secret::new(format!("csrf-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1))
    }

    fn hash(&self, token: &Secret) -> TokenHash {
        TokenHash::new(format!("hash:{}", token.expose()))
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

#[async_trait]
impl AuditReader for MemoryAudit {
    async fn list(&self, before_id: Option<i64>, limit: u32) -> Result<Vec<(i64, AuditEntry)>, PortError> {
        let entries = self.0.lock().unwrap();
        Ok(entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i as i64 + 1, e.clone()))
            .rev()
            .filter(|(id, _)| before_id.is_none_or(|before| *id < before))
            .take(limit as usize)
            .collect())
    }
}

/// A clock that stands still until moved.
pub struct FixedClock(pub Mutex<Timestamp>);

impl FixedClock {
    pub fn at(seconds: i64) -> Self {
        Self(Mutex::new(Timestamp::from_unix_seconds(seconds)))
    }

    pub fn advance(&self, duration: std::time::Duration) {
        let mut now = self.0.lock().unwrap();
        *now = now.saturating_add(duration);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

/// The context with every port in memory, and the parts tests look into.
pub struct TestIdentity {
    pub use_cases: IdentityUseCases,
    pub accounts: Arc<MemoryAccounts>,
    pub sessions: Arc<MemorySessions>,
    pub audit: Arc<MemoryAudit>,
    pub clock: Arc<FixedClock>,
}

impl TestIdentity {
    pub fn new() -> Self {
        let accounts = Arc::new(MemoryAccounts::default());
        let sessions = Arc::new(MemorySessions::default());
        let audit = Arc::new(MemoryAudit::default());
        let clock = Arc::new(FixedClock::at(1_800_000_000));
        let ports = IdentityPorts {
            accounts: accounts.clone(),
            sessions: sessions.clone(),
            hasher: Arc::new(PlainHasher),
            tokens: Arc::new(CountingTokens::default()),
            audit: audit.clone(),
            audit_reader: audit.clone(),
            clock: clock.clone(),
        };
        let config = IdentityConfig {
            setup_code: Some(SetupCode::new(Secret::new(SETUP_CODE))),
            ..Default::default()
        };
        Self { use_cases: IdentityUseCases::new(ports, config), accounts, sessions, audit, clock }
    }
}

impl Default for TestIdentity {
    fn default() -> Self {
        Self::new()
    }
}
