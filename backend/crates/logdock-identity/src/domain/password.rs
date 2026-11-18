use std::fmt;

use logdock_kernel::Secret;

/// Shorter passwords are refused.
pub const MIN_PASSWORD_CHARS: usize = 10;
/// Longer ones too: hashing has to stay cheap enough to answer quickly.
pub const MAX_PASSWORD_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasswordRejected {
    #[error("the password must have at least {MIN_PASSWORD_CHARS} characters")]
    TooShort,
    #[error("the password must have at most {MAX_PASSWORD_BYTES} bytes")]
    TooLong,
}

/// A password someone chose, checked against the rules.
#[derive(Debug, Clone)]
pub struct NewPassword(Secret);

impl NewPassword {
    pub fn parse(password: Secret) -> Result<Self, PasswordRejected> {
        if password.expose().len() > MAX_PASSWORD_BYTES {
            return Err(PasswordRejected::TooLong);
        }
        if password.expose().chars().count() < MIN_PASSWORD_CHARS {
            return Err(PasswordRejected::TooShort);
        }
        Ok(Self(password))
    }

    pub fn secret(&self) -> &Secret {
        &self.0
    }
}

/// A password hash in PHC string format (`$argon2id$…`).
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    pub fn new(hash: String) -> Self {
        Self(hash)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordHash(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_passwords_to_their_length() {
        assert!(NewPassword::parse(Secret::new("correct horse")).is_ok());
        assert_eq!(NewPassword::parse(Secret::new("short")).unwrap_err(), PasswordRejected::TooShort);
        // Ten characters, more bytes.
        assert!(NewPassword::parse(Secret::new("ääääääääää")).is_ok());
        let long = "x".repeat(MAX_PASSWORD_BYTES + 1);
        assert_eq!(NewPassword::parse(Secret::new(long)).unwrap_err(), PasswordRejected::TooLong);
        let hash = PasswordHash::new("$argon2id$secret".into());
        assert_eq!(format!("{hash:?}"), "PasswordHash(***)");
        assert_eq!(hash.as_str(), "$argon2id$secret");
        let password = NewPassword::parse(Secret::new("correct horse")).unwrap();
        assert_eq!(password.secret().expose(), "correct horse");
    }
}
