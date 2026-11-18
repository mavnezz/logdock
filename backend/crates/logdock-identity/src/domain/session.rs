use std::net::IpAddr;
use std::time::Duration;

use logdock_kernel::{Secret, Timestamp};

use super::AccountId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(i64);

impl SessionId {
    pub const fn new(id: i64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

/// The hash of a session token, as stored: the token itself is known to the
/// browser only.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenHash(String);

impl TokenHash {
    pub fn new(hash: String) -> Self {
        Self(hash)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How long sessions last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionPolicy {
    /// Without a request for this long, the session ends.
    pub idle: Duration,
    /// After this long, it ends whatever happens.
    pub lifetime: Duration,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self { idle: Duration::from_secs(12 * 3600), lifetime: Duration::from_secs(7 * 86_400) }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: SessionId,
    pub account_id: AccountId,
    pub token_hash: TokenHash,
    /// Sent back with every change; ties requests to this session.
    pub csrf_token: Secret,
    pub created_at: Timestamp,
    pub last_seen_at: Timestamp,
    pub client_ip: Option<IpAddr>,
    pub user_agent: Option<String>,
}

impl Session {
    /// When the session ends unless used again before.
    pub fn expires_at(&self, policy: &SessionPolicy) -> Timestamp {
        let idle = self.last_seen_at.saturating_add(policy.idle);
        let lifetime = self.created_at.saturating_add(policy.lifetime);
        idle.min(lifetime)
    }

    pub fn is_valid(&self, policy: &SessionPolicy, now: Timestamp) -> bool {
        now < self.expires_at(policy)
    }

    /// Whether `last_seen_at` is old enough to be written again: once a
    /// minute is enough for the idle timeout and spares the database.
    pub fn needs_touch(&self, now: Timestamp) -> bool {
        now.duration_since(self.last_seen_at) >= Duration::from_secs(60)
    }
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub account_id: AccountId,
    pub token_hash: TokenHash,
    pub csrf_token: Secret,
    pub created_at: Timestamp,
    pub client_ip: Option<IpAddr>,
    pub user_agent: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(created: i64, seen: i64) -> Session {
        Session {
            id: SessionId::new(1),
            account_id: AccountId::new(1),
            token_hash: TokenHash::new("h".into()),
            csrf_token: Secret::new("c"),
            created_at: Timestamp::from_unix_seconds(created),
            last_seen_at: Timestamp::from_unix_seconds(seen),
            client_ip: None,
            user_agent: None,
        }
    }

    #[test]
    fn ends_when_idle_or_too_old() {
        let policy = SessionPolicy { idle: Duration::from_secs(100), lifetime: Duration::from_secs(1000) };
        let fresh = session(0, 0);
        assert!(fresh.is_valid(&policy, Timestamp::from_unix_seconds(99)));
        assert!(!fresh.is_valid(&policy, Timestamp::from_unix_seconds(100)));
        let used = session(0, 950);
        assert_eq!(used.expires_at(&policy), Timestamp::from_unix_seconds(1000));
        assert!(!used.is_valid(&policy, Timestamp::from_unix_seconds(1000)));
        assert!(used.needs_touch(Timestamp::from_unix_seconds(1010)));
        assert!(!used.needs_touch(Timestamp::from_unix_seconds(1009)));
        assert_eq!(fresh.token_hash.as_str(), "h");
        assert_eq!(SessionPolicy::default().idle, Duration::from_secs(43_200));
    }
}
