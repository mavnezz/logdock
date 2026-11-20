//! Tokens an account makes for programs (scripts, Claude through MCP) that
//! read Logdock's API on its behalf.

use std::time::Duration;

use logdock_kernel::Timestamp;

use super::{AccountId, Permission, TokenHash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApiTokenId(i64);

impl ApiTokenId {
    pub const fn new(id: i64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

/// What a token may do, at most what its account may: reading always,
/// changing only issues' state, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenScope {
    /// Logs, issues, traces, monitoring and projects.
    Read,
    /// Also resolve, mute and reopen issues.
    Triage,
}

impl TokenScope {
    pub fn allows(self, permission: Permission) -> bool {
        match permission {
            Permission::ReadData => true,
            Permission::TriageIssues => self == Self::Triage,
            Permission::ManageProjects | Permission::ManageAccounts | Permission::Administer => false,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Triage => "triage",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "read" => Some(Self::Read),
            "triage" => Some(Self::Triage),
            _ => None,
        }
    }
}

/// What a token is for, such as `claude on my laptop`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenName(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a token's name has 1 to {max} characters")]
pub struct InvalidTokenName {
    pub max: usize,
}

impl TokenName {
    pub const MAX_CHARS: usize = 64;

    pub fn parse(text: &str) -> Result<Self, InvalidTokenName> {
        let name = text.trim();
        if name.is_empty() || name.chars().count() > Self::MAX_CHARS || name.chars().any(char::is_control) {
            return Err(InvalidTokenName { max: Self::MAX_CHARS });
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiToken {
    pub id: ApiTokenId,
    pub account_id: AccountId,
    pub name: TokenName,
    pub scope: TokenScope,
    /// The token's first characters, to tell tokens apart.
    pub prefix: String,
    pub created_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
}

impl ApiToken {
    /// `ldt_` and the first random characters.
    pub const PREFIX_LEN: usize = 12;

    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }

    /// Whether `last_used_at` is old enough to be written again: once a
    /// minute is precise enough to see whether a token is in use.
    pub fn needs_touch(&self, now: Timestamp) -> bool {
        self.last_used_at.is_none_or(|at| now.duration_since(at) >= Duration::from_secs(60))
    }
}

#[derive(Debug, Clone)]
pub struct NewApiToken {
    pub account_id: AccountId,
    pub name: TokenName,
    pub scope: TokenScope,
    pub prefix: String,
    pub hash: TokenHash,
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_reads_and_at_most_triages() {
        for scope in [TokenScope::Read, TokenScope::Triage] {
            assert!(scope.allows(Permission::ReadData));
            assert!(!scope.allows(Permission::ManageProjects));
            assert!(!scope.allows(Permission::ManageAccounts));
            assert!(!scope.allows(Permission::Administer));
            assert_eq!(TokenScope::parse(scope.as_str()), Some(scope));
        }
        assert!(!TokenScope::Read.allows(Permission::TriageIssues));
        assert!(TokenScope::Triage.allows(Permission::TriageIssues));
        assert_eq!(TokenScope::parse("admin"), None);
    }

    #[test]
    fn names_are_short_and_printable() {
        assert_eq!(TokenName::parse("  claude  ").unwrap().as_str(), "claude");
        for bad in ["", "   ", "a\nb"] {
            assert!(TokenName::parse(bad).is_err(), "{bad:?}");
        }
        assert!(TokenName::parse(&"x".repeat(65)).is_err());
    }

    #[test]
    fn its_last_use_is_written_once_a_minute() {
        let token = ApiToken {
            id: ApiTokenId::new(1),
            account_id: AccountId::new(1),
            name: TokenName::parse("t").unwrap(),
            scope: TokenScope::Read,
            prefix: "ldt_abcd1234".into(),
            created_at: Timestamp::from_unix_seconds(0),
            last_used_at: None,
            revoked_at: None,
        };
        assert!(token.is_active() && token.needs_touch(Timestamp::from_unix_seconds(1)));
        let used = ApiToken { last_used_at: Some(Timestamp::from_unix_seconds(100)), ..token.clone() };
        assert!(!used.needs_touch(Timestamp::from_unix_seconds(159)));
        assert!(used.needs_touch(Timestamp::from_unix_seconds(160)));
        assert!(!ApiToken { revoked_at: Some(Timestamp::from_unix_seconds(5)), ..token }.is_active());
        assert_eq!(ApiTokenId::new(4).get(), 4);
    }
}
