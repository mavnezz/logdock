use std::net::IpAddr;

use async_trait::async_trait;

use crate::{PortError, RequestContext, Timestamp};

/// Who did something: a signed-in account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub account_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// Wrong input, such as a wrong password.
    Failure,
    /// Not allowed.
    Denied,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Denied => "denied",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "success" => Self::Success,
            "failure" => Self::Failure,
            "denied" => Self::Denied,
            _ => return None,
        })
    }
}

/// One line of the audit log: sign-ins and changes to accounts, projects,
/// keys and settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub at: Timestamp,
    /// What was done, e.g. `login`, `key.revoke`.
    pub action: String,
    pub outcome: Outcome,
    pub actor_id: Option<i64>,
    /// The account's name, or what was typed for an unknown one.
    pub actor_name: Option<String>,
    /// What it was done to, e.g. a project's name.
    pub target: Option<String>,
    /// Why it failed or was denied, as a short code.
    pub reason: Option<String>,
    pub client_ip: Option<IpAddr>,
    pub user_agent: Option<String>,
}

impl AuditEntry {
    pub fn new(at: Timestamp, action: &str, outcome: Outcome, request: &RequestContext) -> Self {
        Self {
            at,
            action: action.to_owned(),
            outcome,
            actor_id: None,
            actor_name: None,
            target: None,
            reason: None,
            client_ip: request.client_ip,
            user_agent: request.user_agent.clone(),
        }
    }

    pub fn by(mut self, actor: &Actor) -> Self {
        self.actor_id = Some(actor.account_id);
        self.actor_name = Some(actor.name.clone());
        self
    }

    /// A name that belongs to no account, as it was typed.
    pub fn actor(mut self, name: &str) -> Self {
        self.actor_name = Some(name.chars().take(128).collect());
        self
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.to_owned());
        self
    }
}

/// Where audit entries are kept.
#[async_trait]
pub trait AuditLog: Send + Sync {
    async fn record(&self, entry: AuditEntry) -> Result<(), PortError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_an_entry() {
        let request = RequestContext { client_ip: Some([10, 0, 0, 1].into()), ..Default::default() };
        let actor = Actor { account_id: 3, name: "ada".into() };
        let entry = AuditEntry::new(Timestamp::from_unix_seconds(5), "login", Outcome::Failure, &request)
            .by(&actor)
            .target("project")
            .reason("wrong_password");
        assert_eq!(entry.actor_id, Some(3));
        assert_eq!(entry.actor_name.as_deref(), Some("ada"));
        assert_eq!(entry.target.as_deref(), Some("project"));
        assert_eq!(entry.reason.as_deref(), Some("wrong_password"));
        assert_eq!(entry.client_ip, request.client_ip);
        let typed = AuditEntry::new(entry.at, "login", Outcome::Denied, &request).actor(&"x".repeat(200));
        assert_eq!(typed.actor_name.map(|n| n.len()), Some(128));
        for outcome in [Outcome::Success, Outcome::Failure, Outcome::Denied] {
            assert_eq!(Outcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(Outcome::parse("maybe"), None);
    }
}
