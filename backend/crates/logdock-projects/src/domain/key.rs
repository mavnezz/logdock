use logdock_kernel::{EnvironmentId, ProjectId, Timestamp};

use super::InvalidProject;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyId(i64);

impl KeyId {
    pub const fn new(id: i64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

/// The hash of an ingest key, as stored; the key is shown once.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyHash(String);

impl KeyHash {
    pub fn new(hash: String) -> Self {
        Self(hash)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a key is for, such as `orders web`; optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLabel(String);

impl KeyLabel {
    pub const MAX_CHARS: usize = 64;

    pub fn parse(text: &str) -> Result<Option<Self>, InvalidProject> {
        let label = text.trim();
        if label.is_empty() {
            return Ok(None);
        }
        if label.chars().count() > Self::MAX_CHARS || label.chars().any(char::is_control) {
            return Err(InvalidProject::Name { max: Self::MAX_CHARS });
        }
        Ok(Some(Self(label.to_owned())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A key that lets one project's one stage send data. It is the DSN's public
/// key; the project and stage it belongs to are what the data is stored
/// under, whatever the data says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestKey {
    pub id: KeyId,
    pub project: ProjectId,
    pub environment: EnvironmentId,
    pub label: Option<KeyLabel>,
    /// The key's first characters, to tell keys apart.
    pub prefix: String,
    pub created_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
}

impl IngestKey {
    pub const PREFIX_LEN: usize = 8;

    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_optional() {
        assert_eq!(KeyLabel::parse("  "), Ok(None));
        assert_eq!(KeyLabel::parse(" web ").unwrap().unwrap().as_str(), "web");
        assert!(KeyLabel::parse(&"x".repeat(65)).is_err());
        let key = IngestKey {
            id: KeyId::new(1),
            project: ProjectId::new(1),
            environment: EnvironmentId::new(1),
            label: None,
            prefix: "abcd1234".into(),
            created_at: Timestamp::from_unix_seconds(0),
            last_used_at: None,
            revoked_at: None,
        };
        assert!(key.is_active());
        assert!(!IngestKey { revoked_at: Some(key.created_at), ..key }.is_active());
        assert_eq!(KeyHash::new("h".into()).as_str(), "h");
    }
}
