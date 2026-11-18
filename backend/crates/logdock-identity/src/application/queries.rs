use std::sync::Arc;

use logdock_kernel::{AuditEntry, Secret};

use crate::application::context::Context;
use crate::application::{AccountSummary, AuthContext, IdentityError};
use crate::domain::Permission;

/// Query: whether the first setup is still to be done.
#[derive(Clone)]
pub struct GetSetupStatus {
    context: Arc<Context>,
}

impl GetSetupStatus {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    /// `true` while there is no account and a setup code is set.
    pub async fn execute(&self) -> Result<bool, IdentityError> {
        let c = &self.context;
        Ok(c.config.setup_code.is_some() && c.ports.accounts.count().await? == 0)
    }
}

/// Query: the session a token belongs to, if it is valid. Keeps the session
/// alive (at most one write a minute).
#[derive(Clone)]
pub struct Authenticate {
    context: Arc<Context>,
}

impl Authenticate {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(&self, token: &Secret) -> Result<AuthContext, IdentityError> {
        let c = &self.context;
        let now = c.now();
        let hash = c.ports.tokens.hash(token);
        let Some(mut session) = c.ports.sessions.find_by_token(&hash).await? else {
            return Err(IdentityError::Unauthenticated);
        };
        if !session.is_valid(&c.config.sessions, now) {
            c.ports.sessions.delete(session.id).await?;
            return Err(IdentityError::Unauthenticated);
        }
        let account = c.ports.accounts.find_by_id(session.account_id).await?;
        let Some(account) = account.filter(|a| !a.disabled) else {
            c.ports.sessions.delete(session.id).await?;
            return Err(IdentityError::Unauthenticated);
        };
        if session.needs_touch(now) {
            c.ports.sessions.touch(session.id, now).await?;
            session.last_seen_at = now;
        }
        Ok(AuthContext {
            session_id: session.id,
            expires_at: session.expires_at(&c.config.sessions),
            csrf_token: session.csrf_token,
            account: AccountSummary::from(&account),
        })
    }
}

/// Query: all accounts, for administrators.
#[derive(Clone)]
pub struct ListAccounts {
    context: Arc<Context>,
}

impl ListAccounts {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(&self, auth: &AuthContext) -> Result<Vec<AccountSummary>, IdentityError> {
        Context::require(auth, Permission::ManageAccounts)?;
        Ok(self.context.ports.accounts.list().await?.iter().map(AccountSummary::from).collect())
    }
}

/// Query: the audit log, newest first, for administrators.
#[derive(Clone)]
pub struct ListAudit {
    context: Arc<Context>,
}

impl ListAudit {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        before_id: Option<i64>,
        limit: u32,
    ) -> Result<Vec<(i64, AuditEntry)>, IdentityError> {
        Context::require(auth, Permission::Administer)?;
        Ok(self.context.ports.audit_reader.list(before_id, limit.clamp(1, 500)).await?)
    }
}
