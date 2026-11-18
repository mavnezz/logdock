//! What this context needs from the outside world.

use async_trait::async_trait;
use logdock_kernel::{AuditEntry, PortError, Secret, Timestamp};

use crate::domain::{
    Account, AccountId, Email, NewAccount, NewSession, PasswordHash, Session, SessionId, TokenHash,
};

#[async_trait]
pub trait AccountRepository: Send + Sync {
    async fn count(&self) -> Result<u64, PortError>;

    async fn find_by_id(&self, id: AccountId) -> Result<Option<Account>, PortError>;

    async fn find_by_email(&self, email: &Email) -> Result<Option<Account>, PortError>;

    /// All accounts, ordered by e-mail address.
    async fn list(&self) -> Result<Vec<Account>, PortError>;

    /// `None` if the e-mail address is taken.
    async fn create(&self, account: NewAccount) -> Result<Option<Account>, PortError>;

    /// Stores `account` only if there is no account at all yet, atomically,
    /// so two concurrent setups cannot both succeed. `None` if one existed.
    async fn create_first(&self, account: NewAccount) -> Result<Option<Account>, PortError>;

    /// Stores name, role, language, `disabled` and `updated_at`.
    async fn update(&self, account: &Account) -> Result<(), PortError>;

    async fn update_password(
        &self,
        id: AccountId,
        hash: &PasswordHash,
        at: Timestamp,
    ) -> Result<(), PortError>;

    /// Deletes the account with its sessions.
    async fn delete(&self, id: AccountId) -> Result<(), PortError>;
}

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn create(&self, session: NewSession) -> Result<Session, PortError>;

    async fn find_by_token(&self, hash: &TokenHash) -> Result<Option<Session>, PortError>;

    async fn touch(&self, id: SessionId, at: Timestamp) -> Result<(), PortError>;

    async fn delete(&self, id: SessionId) -> Result<(), PortError>;

    /// Ends every session of the account, but `except`.
    async fn delete_for_account(
        &self,
        account: AccountId,
        except: Option<SessionId>,
    ) -> Result<(), PortError>;

    /// Deletes sessions last used before `idle_before` or created before
    /// `created_before`; returns how many.
    async fn delete_expired(
        &self,
        idle_before: Timestamp,
        created_before: Timestamp,
    ) -> Result<u64, PortError>;
}

/// Argon2id, which takes a while: adapters run it off the async threads.
#[async_trait]
pub trait PasswordHasher: Send + Sync {
    async fn hash(&self, password: &Secret) -> Result<PasswordHash, PortError>;

    /// With `None`, verifies against a dummy hash, so an unknown account
    /// takes as long as a wrong password.
    async fn verify(&self, password: &Secret, hash: Option<&PasswordHash>) -> Result<bool, PortError>;
}

/// Random tokens and their hashes.
pub trait Tokens: Send + Sync {
    /// A new session token, as it goes into the cookie.
    fn session_token(&self) -> Secret;

    /// A new CSRF token.
    fn csrf_token(&self) -> Secret;

    fn hash(&self, token: &Secret) -> TokenHash;
}

/// Reads the audit log, newest first.
#[async_trait]
pub trait AuditReader: Send + Sync {
    async fn list(&self, before_id: Option<i64>, limit: u32) -> Result<Vec<(i64, AuditEntry)>, PortError>;
}
