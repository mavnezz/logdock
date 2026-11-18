use logdock_kernel::{Day, Timestamp};

/// Why ingest refused data (docs/architecture.md, 3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rejection {
    /// No key, an unknown or a revoked one, or one of another project.
    Auth,
    /// The data named another stage than its key's.
    Environment,
    /// Too large, before or after decompression, or too many items.
    Size,
    /// The key sent faster than its rate.
    Rate,
    /// Not readable: broken envelope, JSON or compression.
    Invalid,
    /// Free disk below the minimum.
    Disk,
    /// The writer's queue was full.
    Overloaded,
}

impl Rejection {
    pub const ALL: [Rejection; 7] = [
        Rejection::Auth,
        Rejection::Environment,
        Rejection::Size,
        Rejection::Rate,
        Rejection::Invalid,
        Rejection::Disk,
        Rejection::Overloaded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Environment => "environment",
            Self::Size => "size",
            Self::Rate => "rate",
            Self::Invalid => "invalid",
            Self::Disk => "disk",
            Self::Overloaded => "overloaded",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == text)
    }
}

/// What one request brought: items accepted, and those refused with why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    pub accepted: u64,
    /// Items dropped because ingest does not keep their kind (yet).
    pub skipped: u64,
    pub rejected: Vec<(Rejection, u64)>,
    /// What the last rejection was about, for the project's page.
    pub detail: Option<String>,
}

impl Tally {
    pub fn accepted(count: u64) -> Self {
        Self { accepted: count, ..Self::default() }
    }

    pub fn rejected(reason: Rejection, count: u64, detail: impl Into<String>) -> Self {
        Self { rejected: vec![(reason, count.max(1))], detail: Some(detail.into()), ..Self::default() }
    }

    pub fn reject(&mut self, reason: Rejection, count: u64, detail: impl Into<String>) {
        match self.rejected.iter_mut().find(|(r, _)| *r == reason) {
            Some((_, n)) => *n += count,
            None => self.rejected.push((reason, count)),
        }
        self.detail = Some(detail.into());
    }

    pub fn is_empty(&self) -> bool {
        self.accepted == 0 && self.skipped == 0 && self.rejected.is_empty()
    }
}

/// Counts of one day, for one key (or none, for refused keys).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestCount {
    pub day: Day,
    pub key: Option<super::KeyId>,
    pub accepted: u64,
    pub skipped: u64,
    pub rejected: Vec<(Rejection, u64)>,
    pub last_rejection: Option<(Rejection, String, Timestamp)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_rejections_by_reason() {
        let mut tally = Tally::accepted(3);
        tally.reject(Rejection::Size, 1, "too large");
        tally.reject(Rejection::Size, 2, "too large again");
        tally.reject(Rejection::Rate, 1, "slow down");
        assert_eq!(tally.rejected, [(Rejection::Size, 3), (Rejection::Rate, 1)]);
        assert_eq!(tally.detail.as_deref(), Some("slow down"));
        assert!(!tally.is_empty());
        assert!(Tally::default().is_empty());
        assert_eq!(Tally::rejected(Rejection::Auth, 0, "x").rejected, [(Rejection::Auth, 1)]);
        for reason in Rejection::ALL {
            assert_eq!(Rejection::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(Rejection::parse("nope"), None);
    }
}
