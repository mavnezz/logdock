//! Use cases of the Identity context and the ports they need.

pub mod commands;
mod context;
mod error;
pub mod ports;
pub mod queries;
mod tokens;

use std::sync::Arc;

use logdock_kernel::{Actor, AuditLog, Clock, Secret, Timestamp};

use crate::domain::{
    Account, AccountId, ApiTokenId, Email, Locale, Permission, Role, SessionId, SessionPolicy, SetupCode,
    ThrottlePolicy, TokenScope,
};
use context::Context;
pub use error::IdentityError;
use ports::{AccountRepository, ApiTokenRepository, AuditReader, PasswordHasher, SessionRepository, Tokens};
pub use tokens::{AuthenticateToken, CreateApiToken, CreatedApiToken, ListApiTokens, RevokeApiToken};

/// Adapters for every port of this context, plugged in by the composition
/// root.
#[derive(Clone)]
pub struct IdentityPorts {
    pub accounts: Arc<dyn AccountRepository>,
    pub sessions: Arc<dyn SessionRepository>,
    pub api_tokens: Arc<dyn ApiTokenRepository>,
    pub hasher: Arc<dyn PasswordHasher>,
    pub tokens: Arc<dyn Tokens>,
    pub audit: Arc<dyn AuditLog>,
    pub audit_reader: Arc<dyn AuditReader>,
    pub clock: Arc<dyn Clock>,
}

#[derive(Debug, Clone, Default)]
pub struct IdentityConfig {
    pub sessions: SessionPolicy,
    pub throttle: ThrottlePolicy,
    /// The code of the first setup, while there is no account yet.
    pub setup_code: Option<SetupCode>,
}

/// All use cases of this context, sharing one set of ports and one sign-in
/// throttle.
#[derive(Clone)]
pub struct IdentityUseCases {
    pub setup_status: queries::GetSetupStatus,
    pub complete_setup: commands::CompleteSetup,
    pub login: commands::Login,
    pub logout: commands::Logout,
    pub authenticate: queries::Authenticate,
    pub change_password: commands::ChangePassword,
    pub update_profile: commands::UpdateProfile,
    pub list_accounts: queries::ListAccounts,
    pub create_account: commands::CreateAccount,
    pub update_account: commands::UpdateAccount,
    pub reset_password: commands::ResetPassword,
    pub delete_account: commands::DeleteAccount,
    pub purge_sessions: commands::PurgeExpiredSessions,
    pub audit_log: queries::ListAudit,
    pub create_token: CreateApiToken,
    pub list_tokens: ListApiTokens,
    pub revoke_token: RevokeApiToken,
    pub authenticate_token: AuthenticateToken,
}

impl IdentityUseCases {
    pub fn new(ports: IdentityPorts, config: IdentityConfig) -> Self {
        let context = Arc::new(Context::new(ports, config));
        Self {
            setup_status: queries::GetSetupStatus::new(Arc::clone(&context)),
            complete_setup: commands::CompleteSetup::new(Arc::clone(&context)),
            login: commands::Login::new(Arc::clone(&context)),
            logout: commands::Logout::new(Arc::clone(&context)),
            authenticate: queries::Authenticate::new(Arc::clone(&context)),
            change_password: commands::ChangePassword::new(Arc::clone(&context)),
            update_profile: commands::UpdateProfile::new(Arc::clone(&context)),
            list_accounts: queries::ListAccounts::new(Arc::clone(&context)),
            create_account: commands::CreateAccount::new(Arc::clone(&context)),
            update_account: commands::UpdateAccount::new(Arc::clone(&context)),
            reset_password: commands::ResetPassword::new(Arc::clone(&context)),
            delete_account: commands::DeleteAccount::new(Arc::clone(&context)),
            purge_sessions: commands::PurgeExpiredSessions::new(Arc::clone(&context)),
            audit_log: queries::ListAudit::new(Arc::clone(&context)),
            create_token: CreateApiToken::new(Arc::clone(&context)),
            list_tokens: ListApiTokens::new(Arc::clone(&context)),
            revoke_token: RevokeApiToken::new(Arc::clone(&context)),
            authenticate_token: AuthenticateToken::new(context),
        }
    }
}

/// The parts of an account a signed-in client may see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSummary {
    pub id: AccountId,
    pub email: Email,
    pub name: String,
    pub role: Role,
    pub locale: Locale,
    pub disabled: bool,
    pub created_at: Timestamp,
}

impl From<&Account> for AccountSummary {
    fn from(account: &Account) -> Self {
        Self {
            id: account.id,
            email: account.email.clone(),
            name: account.name.as_str().to_owned(),
            role: account.role,
            locale: account.locale,
            disabled: account.disabled,
            created_at: account.created_at,
        }
    }
}

/// A session that was just started. The token goes into the cookie and is
/// never shown again.
#[derive(Debug)]
pub struct SignedIn {
    pub token: Secret,
    pub session: AuthContext,
}

/// The authenticated caller of a request: an account, through a session of
/// the interface or an API token.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub account: AccountSummary,
    pub credential: Credential,
}

#[derive(Debug, Clone)]
pub enum Credential {
    Session(SessionCredential),
    Token(TokenCredential),
}

#[derive(Debug, Clone)]
pub struct SessionCredential {
    pub id: SessionId,
    /// Sent back with every change; ties requests to this session.
    pub csrf_token: Secret,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone)]
pub struct TokenCredential {
    pub id: ApiTokenId,
    pub name: String,
    pub scope: TokenScope,
}

impl AuthContext {
    /// What the account may do, narrowed to the token's scope.
    pub fn can(&self, permission: Permission) -> bool {
        self.account.role.grants(permission)
            && match &self.credential {
                Credential::Session(_) => true,
                Credential::Token(token) => token.scope.allows(permission),
            }
    }

    pub fn session(&self) -> Option<&SessionCredential> {
        match &self.credential {
            Credential::Session(session) => Some(session),
            Credential::Token(_) => None,
        }
    }

    pub fn token(&self) -> Option<&TokenCredential> {
        match &self.credential {
            Credential::Session(_) => None,
            Credential::Token(token) => Some(token),
        }
    }

    /// The signed-in account as the actor of an audit entry, with the
    /// token it acted through.
    pub fn actor(&self) -> Actor {
        let email = self.account.email.as_str();
        let name = match &self.credential {
            Credential::Session(_) => email.to_owned(),
            Credential::Token(token) => format!("{email} (token {})", token.name),
        };
        Actor { account_id: self.account.id.get(), name }
    }
}

#[cfg(test)]
mod tests;
