use logdock_kernel::Day;
pub use logdock_kernel::retention::StageRetention;

/// The kinds of data kept in day segments (docs/architecture.md, 5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Logs,
    Events,
    Traces,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Logs, Kind::Events, Kind::Traces];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Logs => "logs",
            Self::Events => "events",
            Self::Traces => "traces",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }

    /// The segments a stage's data of this kind goes into: those of the
    /// days it keeps it, 0 for the kind's own.
    pub fn kept_by(self, stage: &StageRetention) -> u32 {
        match self {
            Self::Logs => stage.logs_days,
            Self::Events => stage.events_days,
            Self::Traces => stage.traces_days,
        }
        .unwrap_or(0)
    }

    /// Under the storage limit, what goes first among segments of the same
    /// day: traces are the largest and the least needed afterwards, error
    /// events the most.
    fn sacrifice_order(self) -> u8 {
        match self {
            Self::Traces => 0,
            Self::Logs => 1,
            Self::Events => 2,
        }
    }
}

/// How long each kind is kept and how much space all of it may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub logs_days: u32,
    pub events_days: u32,
    pub traces_days: u32,
    /// Counts per minute (charts, monitoring): kept longer than what they
    /// count.
    pub rollups_days: u32,
    /// All segments together; `0` means no limit.
    pub max_storage_bytes: u64,
    /// Below this much free space ingest is refused.
    pub min_free_bytes: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            logs_days: 30,
            events_days: 90,
            traces_days: 7,
            rollups_days: 90,
            max_storage_bytes: 50 * 1024 * 1024 * 1024,
            min_free_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

impl RetentionPolicy {
    pub fn days(&self, kind: Kind) -> u32 {
        match kind {
            Kind::Logs => self.logs_days,
            Kind::Events => self.events_days,
            Kind::Traces => self.traces_days,
        }
    }

    /// How long a segment of `kind` is kept: `keep` days, which its stages
    /// chose, or the kind's own (`keep` 0).
    pub fn days_kept(&self, kind: Kind, keep: u32) -> u32 {
        if keep > 0 { keep } else { self.days(kind) }
    }

    /// The oldest day of a segment of `kind` and `keep` still kept on
    /// `today`: its days including today.
    pub fn oldest_kept(&self, kind: Kind, keep: u32, today: Day) -> Day {
        let days = i64::from(self.days_kept(kind, keep).max(1));
        Day::from_days(today.days() - days + 1)
    }
}

/// One day's segment of one kind. The data of stages with a retention of
/// their own is kept in segments of their own, so that it goes when its
/// days are over: `keep` days, or 0 for the kind's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub kind: Kind,
    pub day: Day,
    pub keep: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionReason {
    /// Its day fell out of retention.
    Expired,
    /// All segments together were above the storage limit.
    StorageLimit,
}

impl DeletionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::StorageLimit => "storage_limit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deletion {
    pub segment: Segment,
    pub reason: DeletionReason,
}

/// What to delete on `today`: every segment older than its retention, then,
/// while all the rest is above the storage limit, the oldest ones. Today's
/// segments are never deleted: they are being written.
pub fn plan_deletions(policy: &RetentionPolicy, segments: &[Segment], today: Day) -> Vec<Deletion> {
    let mut deletions = Vec::new();
    let mut kept = Vec::new();
    for segment in segments {
        if segment.day < policy.oldest_kept(segment.kind, segment.keep, today) && segment.day < today {
            deletions.push(Deletion { segment: *segment, reason: DeletionReason::Expired });
        } else {
            kept.push(*segment);
        }
    }
    if policy.max_storage_bytes == 0 {
        return deletions;
    }
    let mut total: u64 = kept.iter().map(|segment| segment.bytes).sum();
    kept.sort_by_key(|segment| (segment.day, segment.kind.sacrifice_order()));
    for segment in kept {
        if total <= policy.max_storage_bytes {
            break;
        }
        if segment.day >= today {
            continue;
        }
        total -= segment.bytes;
        deletions.push(Deletion { segment, reason: DeletionReason::StorageLimit });
    }
    deletions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(offset: i64) -> Day {
        Day::from_days(20_000 + offset)
    }

    fn segment(kind: Kind, offset: i64, bytes: u64) -> Segment {
        Segment { kind, day: day(offset), keep: 0, bytes }
    }

    fn policy(max: u64) -> RetentionPolicy {
        RetentionPolicy {
            logs_days: 3,
            events_days: 5,
            traces_days: 1,
            rollups_days: 10,
            max_storage_bytes: max,
            min_free_bytes: 0,
        }
    }

    #[test]
    fn deletes_what_is_older_than_its_retention() {
        let segments = [
            segment(Kind::Logs, 0, 1),
            segment(Kind::Logs, -2, 1),
            segment(Kind::Logs, -3, 1),
            segment(Kind::Events, -4, 1),
            segment(Kind::Events, -5, 1),
            segment(Kind::Traces, 0, 1),
            segment(Kind::Traces, -1, 1),
        ];
        let deleted: Vec<_> = plan_deletions(&policy(0), &segments, day(0))
            .into_iter()
            .map(|d| (d.segment.kind, d.segment.day, d.reason))
            .collect();
        assert_eq!(
            deleted,
            [
                (Kind::Logs, day(-3), DeletionReason::Expired),
                (Kind::Events, day(-5), DeletionReason::Expired),
                (Kind::Traces, day(-1), DeletionReason::Expired),
            ]
        );
    }

    #[test]
    fn keeps_a_stages_segments_as_long_as_it_chose() {
        let short = |offset| Segment { keep: 2, ..segment(Kind::Logs, offset, 1) };
        let long = |offset| Segment { keep: 7, ..segment(Kind::Logs, offset, 1) };
        let segments = [short(-1), short(-2), segment(Kind::Logs, -2, 1), long(-6), long(-7)];
        let deleted: Vec<_> = plan_deletions(&policy(0), &segments, day(0))
            .into_iter()
            .map(|d| (d.segment.keep, d.segment.day))
            .collect();
        assert_eq!(deleted, [(2, day(-2)), (7, day(-7))], "the kind's own 3 days keep day -2");

        let stage = StageRetention { logs_days: Some(2), ..Default::default() };
        assert_eq!([Kind::Logs, Kind::Events, Kind::Traces].map(|k| k.kept_by(&stage)), [2, 0, 0]);
    }

    #[test]
    fn deletes_the_oldest_until_below_the_limit_but_never_today() {
        let segments = [
            segment(Kind::Logs, 0, 50),
            segment(Kind::Events, -1, 10),
            segment(Kind::Logs, -1, 20),
            segment(Kind::Traces, -1, 30),
            segment(Kind::Logs, -2, 40),
        ];
        let policy = RetentionPolicy { traces_days: 3, ..policy(80) };
        let deleted = plan_deletions(&policy, &segments, day(0));
        let order: Vec<_> = deleted.iter().map(|d| (d.segment.kind, d.segment.day)).collect();
        // 150 bytes: the day before yesterday goes, then yesterday's traces.
        assert_eq!(order, [(Kind::Logs, day(-2)), (Kind::Traces, day(-1))]);
        assert!(deleted.iter().all(|d| d.reason == DeletionReason::StorageLimit));

        let only_today = [segment(Kind::Logs, 0, 500)];
        assert!(plan_deletions(&policy, &only_today, day(0)).is_empty());
    }

    #[test]
    fn knows_its_kinds_and_defaults() {
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(Kind::parse("spans"), None);
        let defaults = RetentionPolicy::default();
        assert_eq!(defaults.days(Kind::Logs), 30);
        assert_eq!(defaults.oldest_kept(Kind::Logs, 0, day(0)), day(-29));
        assert_eq!(defaults.oldest_kept(Kind::Logs, 3, day(0)), day(-2), "a stage's own");
        assert_eq!(DeletionReason::Expired.as_str(), "expired");
        assert_eq!(DeletionReason::StorageLimit.as_str(), "storage_limit");
    }
}
