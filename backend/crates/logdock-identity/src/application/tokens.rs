//! API tokens: an account makes them for programs that read Logdock on its
//! behalf, and sees and revokes only its own.

use std::sync::Arc;

use logdock_kernel::{AuditEntry, Outcome, RequestContext, Secret};

use crate::application::context::Context;
use crate::application::{AccountSummary, AuthContext, Credential, IdentityError, TokenCredential};
use crate::domain::{ApiToken, ApiTokenId, AuditAction, NewApiToken, Permission, TokenName, TokenScope};

/// A token just made: the secret is shown once, then only its prefix.
#[derive(Debug)]
pub struct CreatedApiToken {
    pub token: ApiToken,
    pub secret: Secret,
}

/// Tokens are made and revoked in the interface only, never by a token.
fn signed_in(auth: &AuthContext) -> Result<(), IdentityError> {
    if auth.session().is_some() { Ok(()) } else { Err(IdentityError::Forbidden) }
}

/// Command: a new token of the caller's account.
#[derive(Clone)]
pub struct CreateApiToken {
    context: Arc<Context>,
}

impl CreateApiToken {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        name: &str,
        scope: TokenScope,
        request: &RequestContext,
    ) -> Result<CreatedApiToken, IdentityError> {
        signed_in(auth)?;
        Context::require(auth, Permission::ReadData)?;
        if scope == TokenScope::Triage {
            Context::require(auth, Permission::TriageIssues)?;
        }
        let name = TokenName::parse(name)?;
        let c = &self.context;
        let secret = c.ports.tokens.api_token();
        let token = c
            .ports
            .api_tokens
            .create(NewApiToken {
                account_id: auth.account.id,
                name: name.clone(),
                scope,
                prefix: secret.expose().chars().take(ApiToken::PREFIX_LEN).collect(),
                hash: c.ports.tokens.hash(&secret),
                created_at: c.now(),
            })
            .await?;
        let entry = AuditEntry::new(c.now(), AuditAction::TokenCreate.as_str(), Outcome::Success, request);
        c.audit(entry.by(&auth.actor()).target(format!("{} ({})", name.as_str(), scope.as_str()))).await?;
        Ok(CreatedApiToken { token, secret })
    }
}

/// Query: the caller's tokens, newest first.
#[derive(Clone)]
pub struct ListApiTokens {
    context: Arc<Context>,
}

impl ListApiTokens {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(&self, auth: &AuthContext) -> Result<Vec<ApiToken>, IdentityError> {
        Ok(self.context.ports.api_tokens.list(auth.account.id).await?)
    }
}

/// Command: one of the caller's tokens stops working at once.
#[derive(Clone)]
pub struct RevokeApiToken {
    context: Arc<Context>,
}

impl RevokeApiToken {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        id: ApiTokenId,
        request: &RequestContext,
    ) -> Result<(), IdentityError> {
        signed_in(auth)?;
        let c = &self.context;
        let tokens = c.ports.api_tokens.list(auth.account.id).await?;
        let token = tokens.iter().find(|t| t.id == id).ok_or(IdentityError::TokenNotFound)?;
        if token.is_active() {
            c.ports.api_tokens.revoke(id, c.now()).await?;
        }
        let entry = AuditEntry::new(c.now(), AuditAction::TokenRevoke.as_str(), Outcome::Success, request);
        c.audit(entry.by(&auth.actor()).target(token.name.as_str())).await
    }
}

/// Query: the account an API token acts for, if the token is valid and its
/// account enabled. Writes its last use at most once a minute.
#[derive(Clone)]
pub struct AuthenticateToken {
    context: Arc<Context>,
}

impl AuthenticateToken {
    pub(crate) fn new(context: Arc<Context>) -> Self {
        Self { context }
    }

    pub async fn execute(&self, secret: &Secret) -> Result<AuthContext, IdentityError> {
        let c = &self.context;
        if !secret.expose().starts_with("ldt_") || secret.expose().len() > 128 {
            return Err(IdentityError::Unauthenticated);
        }
        let hash = c.ports.tokens.hash(secret);
        let token = c.ports.api_tokens.find_by_hash(&hash).await?;
        let Some(token) = token.filter(ApiToken::is_active) else {
            return Err(IdentityError::Unauthenticated);
        };
        let account = c.ports.accounts.find_by_id(token.account_id).await?;
        let Some(account) = account.filter(|a| !a.disabled) else {
            return Err(IdentityError::Unauthenticated);
        };
        let now = c.now();
        if token.needs_touch(now) {
            c.ports.api_tokens.touch(token.id, now).await?;
        }
        Ok(AuthContext {
            account: AccountSummary::from(&account),
            credential: Credential::Token(TokenCredential {
                id: token.id,
                name: token.name.as_str().to_owned(),
                scope: token.scope,
            }),
        })
    }
}
