use std::sync::Arc;

use logdock_kernel::{AuditEntry, Outcome, RequestContext, Secret};

use crate::application::context::Context;
use crate::application::{AccountSummary, AuthContext, IdentityError};
use crate::domain::{AuditAction, DisplayName, Locale, NewPassword};

pub struct ChangePasswordInput {
    pub current: Secret,
    pub new: Secret,
}

/// Command: the caller changes their own password. Their other sessions
/// end; this one stays.
#[derive(Clone)]
pub struct ChangePassword {
    context: Arc<Context>,
}

impl ChangePassword {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        input: ChangePasswordInput,
        request: &RequestContext,
    ) -> Result<(), IdentityError> {
        let c = &self.context;
        let now = c.now();
        let keys = Context::throttle_keys(request, auth.account.email.as_str());
        c.check_throttle(&keys, now)?;
        let account =
            c.ports.accounts.find_by_id(auth.account.id).await?.ok_or(IdentityError::Unauthenticated)?;
        if !c.ports.hasher.verify(&input.current, Some(&account.password_hash)).await? {
            c.record_failure(&keys, now);
            let entry = AuditEntry::new(now, AuditAction::PasswordChange.as_str(), Outcome::Failure, request);
            c.audit(entry.by(&auth.actor()).reason("wrong_password")).await?;
            return Err(IdentityError::WrongPassword);
        }
        let password = NewPassword::parse(input.new)?;
        let hash = c.ports.hasher.hash(password.secret()).await?;
        c.ports.accounts.update_password(account.id, &hash, now).await?;
        c.ports.sessions.delete_for_account(account.id, auth.session().map(|session| session.id)).await?;
        c.record_success(&keys);
        let entry = AuditEntry::new(now, AuditAction::PasswordChange.as_str(), Outcome::Success, request);
        c.audit(entry.by(&auth.actor())).await
    }
}

#[derive(Debug, Default)]
pub struct ProfileChanges {
    pub name: Option<String>,
    pub locale: Option<String>,
}

/// Command: the caller changes their name or language.
#[derive(Clone)]
pub struct UpdateProfile {
    context: Arc<Context>,
}

impl UpdateProfile {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        changes: ProfileChanges,
    ) -> Result<AccountSummary, IdentityError> {
        let c = &self.context;
        let _changes = c.changes.lock().await;
        let mut account =
            c.ports.accounts.find_by_id(auth.account.id).await?.ok_or(IdentityError::Unauthenticated)?;
        if let Some(name) = changes.name {
            account.name = DisplayName::parse(&name)?;
        }
        if let Some(locale) = changes.locale {
            account.locale = Locale::parse(&locale)?;
        }
        account.updated_at = c.now();
        c.ports.accounts.update(&account).await?;
        Ok(AccountSummary::from(&account))
    }
}
