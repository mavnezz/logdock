mod account;
mod api_token;
mod password;
mod role;
mod session;
mod setup_code;
mod throttle;

pub use account::{Account, AccountId, DisplayName, Email, InvalidAccount, Locale, NewAccount};
pub use api_token::{ApiToken, ApiTokenId, InvalidTokenName, NewApiToken, TokenName, TokenScope};
pub use password::{MAX_PASSWORD_BYTES, MIN_PASSWORD_CHARS, NewPassword, PasswordHash, PasswordRejected};
pub use role::{Permission, Role};
pub use session::{NewSession, Session, SessionId, SessionPolicy, TokenHash};
pub use setup_code::SetupCode;
pub use throttle::{LoginThrottle, ThrottlePolicy};

/// What the audit log records of this context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    Setup,
    Login,
    Logout,
    PasswordChange,
    AccountCreate,
    AccountUpdate,
    AccountPasswordReset,
    AccountDelete,
    TokenCreate,
    TokenRevoke,
}

impl AuditAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Login => "login",
            Self::Logout => "logout",
            Self::PasswordChange => "password.change",
            Self::AccountCreate => "account.create",
            Self::AccountUpdate => "account.update",
            Self::AccountPasswordReset => "account.password_reset",
            Self::AccountDelete => "account.delete",
            Self::TokenCreate => "token.create",
            Self::TokenRevoke => "token.revoke",
        }
    }
}
