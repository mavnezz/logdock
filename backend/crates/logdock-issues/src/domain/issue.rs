use logdock_kernel::{Scope, Timestamp};

use super::EventLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IssueId(i64);

impl IssueId {
    pub const fn new(id: i64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Where an issue stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueStatus {
    Unresolved,
    /// Fixed. With `in_next_release`, only an event of another release than
    /// this one counts as a regression: it was fixed in the next one.
    Resolved {
        at: Timestamp,
        in_next_release: Option<String>,
    },
    /// Not to be bothered with: until a time, for a number of events, or for
    /// good.
    Muted {
        until: Option<Timestamp>,
        remaining: Option<u32>,
    },
}

impl IssueStatus {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unresolved => "unresolved",
            Self::Resolved { .. } => "resolved",
            Self::Muted { .. } => "muted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub id: IssueId,
    pub scope: Scope,
    pub fingerprint: String,
    pub title: String,
    pub culprit: Option<String>,
    pub level: EventLevel,
    pub status: IssueStatus,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    pub count: u64,
    pub first_release: Option<String>,
    pub last_release: Option<String>,
    /// When it last came back after being resolved.
    pub regressed_at: Option<Timestamp>,
    pub last_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue {
    pub scope: Scope,
    pub fingerprint: String,
    pub title: String,
    pub culprit: Option<String>,
    pub level: EventLevel,
    pub seen: Timestamp,
    pub release: Option<String>,
    pub event_id: String,
}

/// What an event did to its issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Observation {
    pub regressed: bool,
    /// The issue was muted and still is: nobody is told.
    pub muted: bool,
    pub unmuted: bool,
}

impl Issue {
    /// Counts an event and moves the issue's state on.
    pub fn observe(
        &mut self,
        seen: Timestamp,
        level: EventLevel,
        release: Option<&str>,
        event_id: &str,
    ) -> Observation {
        let mut observation = Observation::default();
        match &mut self.status {
            IssueStatus::Resolved { in_next_release, .. } => {
                let regressed = match in_next_release {
                    Some(fixed_after) => release.is_some_and(|r| r != fixed_after),
                    None => true,
                };
                if regressed {
                    self.status = IssueStatus::Unresolved;
                    self.regressed_at = Some(seen);
                    observation.regressed = true;
                }
            }
            IssueStatus::Muted { until, remaining } => {
                let time_over = until.is_some_and(|until| seen >= until);
                if let Some(left) = remaining {
                    *left = left.saturating_sub(1);
                }
                let count_over = remaining.is_some_and(|left| left == 0);
                if time_over || count_over {
                    self.status = IssueStatus::Unresolved;
                    observation.unmuted = true;
                } else {
                    observation.muted = true;
                }
            }
            IssueStatus::Unresolved => {}
        }
        self.count += 1;
        if seen > self.last_seen {
            self.last_seen = seen;
            self.last_event_id = Some(event_id.to_owned());
        }
        if seen < self.first_seen {
            self.first_seen = seen;
        }
        if let Some(release) = release {
            self.last_release = Some(release.to_owned());
            if self.first_release.is_none() {
                self.first_release = Some(release.to_owned());
            }
        }
        self.level = self.level.max(level);
        observation
    }
}

#[cfg(test)]
mod tests {
    use logdock_kernel::{EnvironmentId, ProjectId};

    use super::*;

    fn issue(status: IssueStatus) -> Issue {
        Issue {
            id: IssueId::new(1),
            scope: Scope { project: ProjectId::new(1), environment: EnvironmentId::new(1) },
            fingerprint: "f".into(),
            title: "t".into(),
            culprit: None,
            level: EventLevel::Warning,
            status,
            first_seen: Timestamp::from_unix_seconds(10),
            last_seen: Timestamp::from_unix_seconds(10),
            count: 1,
            first_release: None,
            last_release: None,
            regressed_at: None,
            last_event_id: None,
        }
    }

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_unix_seconds(seconds)
    }

    #[test]
    fn counts_events_and_keeps_their_range() {
        let mut issue = issue(IssueStatus::Unresolved);
        let seen = issue.observe(at(20), EventLevel::Error, Some("r1"), "e2");
        assert_eq!(seen, Observation::default());
        issue.observe(at(5), EventLevel::Info, Some("r2"), "e0");
        assert_eq!((issue.count, issue.first_seen, issue.last_seen), (3, at(5), at(20)));
        assert_eq!(issue.last_event_id.as_deref(), Some("e2"), "the newest event");
        assert_eq!((issue.first_release.as_deref(), issue.last_release.as_deref()), (Some("r1"), Some("r2")));
        assert_eq!(issue.level, EventLevel::Error, "the worst level seen");
    }

    #[test]
    fn a_resolved_issue_comes_back() {
        let mut now = issue(IssueStatus::Resolved { at: at(10), in_next_release: None });
        assert!(now.observe(at(20), EventLevel::Error, Some("r1"), "e").regressed);
        assert_eq!(now.status, IssueStatus::Unresolved);
        assert_eq!(now.regressed_at, Some(at(20)));

        let mut next = issue(IssueStatus::Resolved { at: at(10), in_next_release: Some("r1".into()) });
        assert!(
            !next.observe(at(20), EventLevel::Error, Some("r1"), "e").regressed,
            "the old release still runs"
        );
        assert!(!next.observe(at(21), EventLevel::Error, None, "e").regressed, "no release, no evidence");
        assert!(next.observe(at(22), EventLevel::Error, Some("r2"), "e").regressed);
    }

    #[test]
    fn a_muted_issue_stays_quiet_until_its_time_or_count_is_up() {
        let mut timed = issue(IssueStatus::Muted { until: Some(at(100)), remaining: None });
        assert!(timed.observe(at(50), EventLevel::Error, None, "e").muted);
        let after = timed.observe(at(100), EventLevel::Error, None, "e");
        assert!(after.unmuted && !after.muted);
        assert_eq!(timed.status.name(), "unresolved");

        let mut counted = issue(IssueStatus::Muted { until: None, remaining: Some(2) });
        assert!(counted.observe(at(11), EventLevel::Error, None, "e").muted);
        assert!(counted.observe(at(12), EventLevel::Error, None, "e").unmuted);

        let mut forever = issue(IssueStatus::Muted { until: None, remaining: None });
        for second in 0..5 {
            assert!(forever.observe(at(20 + second), EventLevel::Error, None, "e").muted);
        }
        assert_eq!(forever.status.name(), "muted");
        assert_eq!(IssueStatus::Resolved { at: at(1), in_next_release: None }.name(), "resolved");
        assert_eq!(IssueId::new(3).get(), 3);
    }
}
