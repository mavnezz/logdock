use std::time::Duration;

use logdock_kernel::PortError;

use crate::domain::{InvalidAccount, PasswordRejected};

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error(transparent)]
    InvalidAccount(#[from] InvalidAccount),
    #[error(transparent)]
    PasswordRejected(#[from] PasswordRejected),
    /// Deliberately does not say whether the address or the password was
    /// wrong.
    #[error("wrong e-mail address or password")]
    InvalidCredentials,
    #[error("the current password is wrong")]
    WrongPassword,
    #[error("too many failed attempts")]
    TooManyAttempts { retry_after: Duration },
    #[error("the first setup has already been completed")]
    SetupCompleted,
    #[error("wrong setup code")]
    InvalidSetupCode,
    #[error("not signed in")]
    Unauthenticated,
    #[error("the account may not do this")]
    Forbidden,
    #[error("there is no such account")]
    AccountNotFound,
    #[error("the e-mail address is already taken")]
    EmailTaken,
    #[error("at least one enabled administrator must remain")]
    LastAdmin,
    #[error("change your own account in your profile")]
    OwnAccount,
    #[error(transparent)]
    Port(#[from] PortError),
}
