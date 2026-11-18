use std::sync::Arc;

use logdock_kernel::{AuditEntry, Outcome, RequestContext, Secret};

use crate::application::context::Context;
use crate::application::{AccountSummary, AuthContext, IdentityError};
use crate::domain::{
    AccountId, AuditAction, DisplayName, Email, Locale, NewAccount, NewPassword, Permission, Role,
};

pub struct NewAccountInput {
    pub email: String,
    pub name: String,
    pub role: Role,
    pub password: Secret,
    pub locale: Option<String>,
}

/// Command: an administrator creates an account.
#[derive(Clone)]
pub struct CreateAccount {
    context: Arc<Context>,
}

impl CreateAccount {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        input: NewAccountInput,
        request: &RequestContext,
    ) -> Result<AccountSummary, IdentityError> {
        Context::require(auth, Permission::ManageAccounts)?;
        let c = &self.context;
        let now = c.now();
        let email = Email::parse(&input.email)?;
        let name = DisplayName::parse(&input.name)?;
        let locale = input.locale.as_deref().map(Locale::parse).transpose()?.unwrap_or_default();
        let password = NewPassword::parse(input.password)?;
        let password_hash = c.ports.hasher.hash(password.secret()).await?;
        let _changes = c.changes.lock().await;
        let new = NewAccount {
            email: email.clone(),
            name,
            role: input.role,
            locale,
            password_hash,
            created_at: now,
        };
        let Some(account) = c.ports.accounts.create(new).await? else {
            return Err(IdentityError::EmailTaken);
        };
        let entry = AuditEntry::new(now, AuditAction::AccountCreate.as_str(), Outcome::Success, request);
        c.audit(entry.by(&auth.actor()).target(email.as_str())).await?;
        Ok(AccountSummary::from(&account))
    }
}

#[derive(Debug, Default)]
pub struct AccountChanges {
    pub name: Option<String>,
    pub role: Option<Role>,
    pub disabled: Option<bool>,
}

/// Command: an administrator changes another account's name, role or
/// whether it is disabled. A disabled account's sessions end.
#[derive(Clone)]
pub struct UpdateAccount {
    context: Arc<Context>,
}

impl UpdateAccount {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        id: AccountId,
        changes: AccountChanges,
        request: &RequestContext,
    ) -> Result<AccountSummary, IdentityError> {
        Context::require(auth, Permission::ManageAccounts)?;
        if id == auth.account.id {
            return Err(IdentityError::OwnAccount);
        }
        let c = &self.context;
        let _changes = c.changes.lock().await;
        let before = c.ports.accounts.find_by_id(id).await?.ok_or(IdentityError::AccountNotFound)?;
        let mut account = before.clone();
        if let Some(name) = changes.name {
            account.name = DisplayName::parse(&name)?;
        }
        if let Some(role) = changes.role {
            account.role = role;
        }
        if let Some(disabled) = changes.disabled {
            account.disabled = disabled;
        }
        let stops_being_admin = account.role != Role::Admin || account.disabled;
        if stops_being_admin && c.is_last_admin(&before).await? {
            return Err(IdentityError::LastAdmin);
        }
        account.updated_at = c.now();
        c.ports.accounts.update(&account).await?;
        if account.disabled && !before.disabled {
            c.ports.sessions.delete_for_account(id, None).await?;
        }
        let entry = AuditEntry::new(
            account.updated_at,
            AuditAction::AccountUpdate.as_str(),
            Outcome::Success,
            request,
        );
        c.audit(entry.by(&auth.actor()).target(account.email.as_str())).await?;
        Ok(AccountSummary::from(&account))
    }
}

/// Command: an administrator sets another account's password; its
/// sessions end.
#[derive(Clone)]
pub struct ResetPassword {
    context: Arc<Context>,
}

impl ResetPassword {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        id: AccountId,
        password: Secret,
        request: &RequestContext,
    ) -> Result<(), IdentityError> {
        Context::require(auth, Permission::ManageAccounts)?;
        if id == auth.account.id {
            return Err(IdentityError::OwnAccount);
        }
        let c = &self.context;
        let account = c.ports.accounts.find_by_id(id).await?.ok_or(IdentityError::AccountNotFound)?;
        let password = NewPassword::parse(password)?;
        let hash = c.ports.hasher.hash(password.secret()).await?;
        let now = c.now();
        c.ports.accounts.update_password(id, &hash, now).await?;
        c.ports.sessions.delete_for_account(id, None).await?;
        let entry =
            AuditEntry::new(now, AuditAction::AccountPasswordReset.as_str(), Outcome::Success, request);
        c.audit(entry.by(&auth.actor()).target(account.email.as_str())).await
    }
}

/// Command: an administrator deletes another account.
#[derive(Clone)]
pub struct DeleteAccount {
    context: Arc<Context>,
}

impl DeleteAccount {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        id: AccountId,
        request: &RequestContext,
    ) -> Result<(), IdentityError> {
        Context::require(auth, Permission::ManageAccounts)?;
        if id == auth.account.id {
            return Err(IdentityError::OwnAccount);
        }
        let c = &self.context;
        let _changes = c.changes.lock().await;
        let account = c.ports.accounts.find_by_id(id).await?.ok_or(IdentityError::AccountNotFound)?;
        if c.is_last_admin(&account).await? {
            return Err(IdentityError::LastAdmin);
        }
        c.ports.accounts.delete(id).await?;
        let entry = AuditEntry::new(c.now(), AuditAction::AccountDelete.as_str(), Outcome::Success, request);
        c.audit(entry.by(&auth.actor()).target(account.email.as_str())).await
    }
}
