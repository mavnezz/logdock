use std::fmt;

use logdock_kernel::{Actor, Timestamp};

use super::{PasswordHash, Role};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(i64);

impl AccountId {
    pub const fn new(id: i64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidAccount {
    #[error("the e-mail address is not valid")]
    Email,
    #[error("the name must have 1 to {max} characters")]
    Name { max: usize },
    #[error("the language {0:?} is not offered")]
    Locale(String),
}

/// An account's e-mail address, which is also its sign-in name. Stored in
/// lower case, so `Ada@Example.com` and `ada@example.com` are one account.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

impl Email {
    pub const MAX_LEN: usize = 254;

    pub fn parse(text: &str) -> Result<Self, InvalidAccount> {
        let email = text.trim().to_lowercase();
        let Some((local, domain)) = email.split_once('@') else { return Err(InvalidAccount::Email) };
        let valid = !local.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !domain.contains('@')
            && email.len() <= Self::MAX_LEN
            && !email
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';'));
        if valid { Ok(Self(email)) } else { Err(InvalidAccount::Email) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The name shown for an account, such as `Ada Lovelace`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayName(String);

impl DisplayName {
    pub const MAX_CHARS: usize = 100;

    pub fn parse(text: &str) -> Result<Self, InvalidAccount> {
        let name = text.trim();
        let chars = name.chars().count();
        if chars == 0 || chars > Self::MAX_CHARS || name.chars().any(char::is_control) {
            return Err(InvalidAccount::Name { max: Self::MAX_CHARS });
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The language of the interface, kept with the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    En,
    De,
}

impl Locale {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::De => "de",
        }
    }

    pub fn parse(text: &str) -> Result<Self, InvalidAccount> {
        match text {
            "en" => Ok(Self::En),
            "de" => Ok(Self::De),
            other => Err(InvalidAccount::Locale(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub email: Email,
    pub name: DisplayName,
    pub role: Role,
    pub locale: Locale,
    pub password_hash: PasswordHash,
    /// A disabled account cannot sign in; its sessions are ended.
    pub disabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Account {
    pub fn actor(&self) -> Actor {
        Actor { account_id: self.id.get(), name: self.email.to_string() }
    }
}

/// An account to be stored.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: Email,
    pub name: DisplayName,
    pub role: Role,
    pub locale: Locale,
    pub password_hash: PasswordHash,
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_e_mail_addresses_in_lower_case() {
        assert_eq!(Email::parse(" Ada@Example.COM ").unwrap().as_str(), "ada@example.com");
        for bad in
            ["", "ada", "@example.com", "ada@", "ada@example", "ada@.com", "a b@example.com", "a@b@c.de"]
        {
            assert_eq!(Email::parse(bad), Err(InvalidAccount::Email), "{bad:?}");
        }
        let long = format!("{}@example.com", "a".repeat(250));
        assert!(Email::parse(&long).is_err());
    }

    #[test]
    fn reads_names_and_languages() {
        assert_eq!(DisplayName::parse("  Ada Lovelace ").unwrap().as_str(), "Ada Lovelace");
        assert!(DisplayName::parse("   ").is_err());
        assert!(DisplayName::parse(&"x".repeat(101)).is_err());
        assert!(DisplayName::parse("a\u{7}").is_err());
        assert_eq!(Locale::parse("de"), Ok(Locale::De));
        assert_eq!(Locale::parse(Locale::En.as_str()), Ok(Locale::En));
        assert!(Locale::parse("fr").is_err());
        assert_eq!(Locale::default(), Locale::En);
    }
}
