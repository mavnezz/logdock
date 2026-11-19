use logdock_kernel::Day;

use super::{Kind, RetentionPolicy, Segment};

/// How much one kind of data takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindUsage {
    pub kind: Kind,
    pub bytes: u64,
    pub segments: u32,
    pub oldest: Option<Day>,
}

/// What the data directory holds and may hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageStatus {
    pub policy: RetentionPolicy,
    pub kinds: Vec<KindUsage>,
    /// The main database: accounts, projects, issues, counts.
    pub database_bytes: u64,
    pub free_bytes: Option<u64>,
}

impl StorageStatus {
    pub fn new(
        policy: RetentionPolicy,
        segments: &[Segment],
        database_bytes: u64,
        free_bytes: Option<u64>,
    ) -> Self {
        let kinds = Kind::ALL
            .into_iter()
            .map(|kind| {
                let of_kind = segments.iter().filter(|s| s.kind == kind);
                KindUsage {
                    kind,
                    bytes: of_kind.clone().map(|s| s.bytes).sum(),
                    segments: of_kind.clone().count() as u32,
                    oldest: of_kind.map(|s| s.day).min(),
                }
            })
            .collect();
        Self { policy, kinds, database_bytes, free_bytes }
    }

    /// All segments together, which the storage limit applies to.
    pub fn segment_bytes(&self) -> u64 {
        self.kinds.iter().map(|k| k.bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_up_by_kind() {
        let segments = [
            Segment { kind: Kind::Logs, day: Day::from_days(3), bytes: 10 },
            Segment { kind: Kind::Logs, day: Day::from_days(2), bytes: 5 },
            Segment { kind: Kind::Traces, day: Day::from_days(3), bytes: 1 },
        ];
        let status = StorageStatus::new(RetentionPolicy::default(), &segments, 7, Some(100));
        assert_eq!(status.segment_bytes(), 16);
        let logs = status.kinds[0];
        assert_eq!(
            (logs.kind, logs.bytes, logs.segments, logs.oldest),
            (Kind::Logs, 15, 2, Some(Day::from_days(2)))
        );
        assert_eq!(status.kinds[1].oldest, None);
        assert_eq!(status.kinds[2].segments, 1);
    }
}
