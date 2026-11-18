use std::sync::Arc;

use logdock_kernel::{AuditEntry, Outcome, RequestContext, Secret};

use crate::application::context::Context;
use crate::application::{IdentityError, SignedIn};
use crate::domain::{AuditAction, DisplayName, Email, Locale, NewAccount, NewPassword, Role};

pub struct SetupInput {
    pub code: String,
    pub email: String,
    pub name: String,
    pub password: Secret,
    pub locale: Option<String>,
}

/// Command: creates the first administrator with the setup code from the
/// log, and signs them in. Works once: as soon as any account exists, the
/// setup is complete.
#[derive(Clone)]
pub struct CompleteSetup {
    context: Arc<Context>,
}

impl CompleteSetup {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        input: SetupInput,
        request: &RequestContext,
    ) -> Result<SignedIn, IdentityError> {
        let c = &self.context;
        let now = c.now();
        let keys = Context::throttle_keys(request, "setup");
        c.check_throttle(&keys, now)?;
        if c.ports.accounts.count().await? > 0 {
            return Err(IdentityError::SetupCompleted);
        }
        let Some(code) = &c.config.setup_code else { return Err(IdentityError::SetupCompleted) };
        if !code.matches(&input.code) {
            c.record_failure(&keys, now);
            c.audit(
                AuditEntry::new(now, AuditAction::Setup.as_str(), Outcome::Failure, request)
                    .reason("wrong_code"),
            )
            .await?;
            return Err(IdentityError::InvalidSetupCode);
        }
        let email = Email::parse(&input.email)?;
        let name = DisplayName::parse(&input.name)?;
        let locale = input.locale.as_deref().map(Locale::parse).transpose()?.unwrap_or_default();
        let password = NewPassword::parse(input.password)?;
        let password_hash = c.ports.hasher.hash(password.secret()).await?;
        let account = NewAccount { email, name, role: Role::Admin, locale, password_hash, created_at: now };
        let Some(account) = c.ports.accounts.create_first(account).await? else {
            return Err(IdentityError::SetupCompleted);
        };
        c.record_success(&keys);
        c.audit(
            AuditEntry::new(now, AuditAction::Setup.as_str(), Outcome::Success, request).by(&account.actor()),
        )
        .await?;
        tracing::info!(account = %account.email, "first administrator created");
        c.start_session(&account, request).await
    }
}
