use std::sync::Arc;

use logdock_kernel::{AuditEntry, Outcome, RequestContext, Secret};

use crate::application::context::Context;
use crate::application::{AuthContext, IdentityError, SignedIn};
use crate::domain::{AuditAction, Email};

pub struct LoginInput {
    pub email: String,
    pub password: Secret,
}

/// Command: checks e-mail address and password and starts a session.
#[derive(Clone)]
pub struct Login {
    context: Arc<Context>,
}

impl Login {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        input: LoginInput,
        request: &RequestContext,
    ) -> Result<SignedIn, IdentityError> {
        let c = &self.context;
        let now = c.now();
        let typed = input.email.trim().to_lowercase();
        let keys = Context::throttle_keys(request, &typed);
        c.check_throttle(&keys, now)?;
        let audit = |outcome| AuditEntry::new(now, AuditAction::Login.as_str(), outcome, request);

        let account = match Email::parse(&typed) {
            Ok(email) => c.ports.accounts.find_by_email(&email).await?,
            Err(_) => None,
        };
        // Runs for unknown addresses too, so the time taken does not tell
        // which accounts exist.
        let verified =
            c.ports.hasher.verify(&input.password, account.as_ref().map(|a| &a.password_hash)).await?;
        match account {
            Some(account) if verified && !account.disabled => {
                c.record_success(&keys);
                c.audit(audit(Outcome::Success).by(&account.actor())).await?;
                c.start_session(&account, request).await
            }
            Some(account) => {
                c.record_failure(&keys, now);
                let reason = if verified { "disabled" } else { "wrong_password" };
                c.audit(audit(Outcome::Failure).by(&account.actor()).reason(reason)).await?;
                Err(IdentityError::InvalidCredentials)
            }
            None => {
                c.record_failure(&keys, now);
                c.audit(audit(Outcome::Failure).actor(&typed).reason("unknown_account")).await?;
                Err(IdentityError::InvalidCredentials)
            }
        }
    }
}

/// Command: ends the caller's session.
#[derive(Clone)]
pub struct Logout {
    context: Arc<Context>,
}

impl Logout {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(&self, auth: &AuthContext, request: &RequestContext) -> Result<(), IdentityError> {
        let c = &self.context;
        c.ports.sessions.delete(auth.session_id).await?;
        let entry = AuditEntry::new(c.now(), AuditAction::Logout.as_str(), Outcome::Success, request)
            .by(&auth.actor());
        c.audit(entry).await
    }
}

/// Command: deletes sessions that have run out; run periodically.
#[derive(Clone)]
pub struct PurgeExpiredSessions {
    context: Arc<Context>,
}

impl PurgeExpiredSessions {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(&self) -> Result<u64, IdentityError> {
        let c = &self.context;
        let now = c.now();
        let policy = c.config.sessions;
        Ok(c.ports
            .sessions
            .delete_expired(now.saturating_sub(policy.idle), now.saturating_sub(policy.lifetime))
            .await?)
    }
}
