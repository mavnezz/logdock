use logdock_kernel::{Day, EnvironmentId, ProjectId};

use super::{Kind, RetentionPolicy, Segment};

/// How much one kind of data takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindUsage {
    pub kind: Kind,
    pub bytes: u64,
    pub segments: u32,
    pub oldest: Option<Day>,
}

/// What one stage of a project takes of one kind, over the days kept:
/// its share of each segment, by the size of its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeUsage {
    pub project: ProjectId,
    pub environment: EnvironmentId,
    pub kind: Kind,
    pub rows: u64,
    pub bytes: u64,
}

/// Where the data is heading at the rate of the last full days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forecast {
    /// The full days the rate is taken from, at most [`Forecast::DAYS`].
    pub days: u32,
    /// What a day adds, on average.
    pub daily_bytes: u64,
    /// What is kept once each kind and stage keeps as many days as its
    /// retention says.
    pub settled_bytes: u64,
    /// Days until the storage limit is reached, if it will be; from then the
    /// oldest days go before their time.
    pub limit_in_days: Option<u32>,
    /// Days until the disk is down to its reserve, if it will be.
    pub disk_in_days: Option<u32>,
}

impl Forecast {
    /// The rate is taken from this many full days at most.
    pub const DAYS: usize = 7;

    /// From the segments of the full days before `today`; `None` without
    /// one.
    pub fn new(
        policy: &RetentionPolicy,
        segments: &[Segment],
        today: Day,
        free_bytes: Option<u64>,
    ) -> Option<Self> {
        let mut days: Vec<Day> = segments.iter().map(|s| s.day).filter(|d| *d < today).collect();
        days.sort();
        days.dedup();
        let days: Vec<Day> = days.into_iter().rev().take(Self::DAYS).collect();
        if days.is_empty() {
            return None;
        }
        let n = days.len() as u64;
        let mut classes: Vec<((Kind, u32), u64)> = Vec::new();
        for segment in segments.iter().filter(|s| days.contains(&s.day)) {
            let class = (segment.kind, segment.keep);
            match classes.iter_mut().find(|(c, _)| *c == class) {
                Some((_, bytes)) => *bytes += segment.bytes,
                None => classes.push((class, segment.bytes)),
            }
        }
        let daily_bytes: u64 = classes.iter().map(|(_, bytes)| bytes / n).sum();
        let settled_bytes: u64 = classes
            .iter()
            .map(|((kind, keep), bytes)| bytes / n * u64::from(policy.days_kept(*kind, *keep)))
            .sum();
        let now: u64 = segments.iter().map(|s| s.bytes).sum();
        let days_until = |bytes: u64| {
            u32::try_from(bytes.saturating_sub(now).div_ceil(daily_bytes.max(1))).unwrap_or(u32::MAX)
        };
        let limit = policy.max_storage_bytes;
        let limit_in_days = (limit > 0 && settled_bytes > limit).then(|| days_until(limit));
        // The data grows to the limit at most.
        let target = if limit > 0 { settled_bytes.min(limit) } else { settled_bytes };
        let disk_in_days = free_bytes.and_then(|free| {
            let room = now.saturating_add(free.saturating_sub(policy.min_free_bytes));
            (target > room).then(|| days_until(room))
        });
        Some(Self { days: n as u32, daily_bytes, settled_bytes, limit_in_days, disk_in_days })
    }
}

/// What the data directory holds and may hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageStatus {
    pub policy: RetentionPolicy,
    pub kinds: Vec<KindUsage>,
    /// Every segment, for the days' sizes.
    pub segments: Vec<Segment>,
    /// The main database: accounts, projects, issues, counts.
    pub database_bytes: u64,
    pub free_bytes: Option<u64>,
    pub forecast: Option<Forecast>,
    /// What each project's stage takes, as last measured.
    pub scopes: Vec<ScopeUsage>,
}

impl StorageStatus {
    pub fn new(
        policy: RetentionPolicy,
        segments: &[Segment],
        database_bytes: u64,
        free_bytes: Option<u64>,
        today: Day,
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
        let forecast = Forecast::new(&policy, segments, today, free_bytes);
        Self {
            policy,
            kinds,
            segments: segments.to_vec(),
            database_bytes,
            free_bytes,
            forecast,
            scopes: Vec::new(),
        }
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
            Segment { kind: Kind::Logs, day: Day::from_days(3), keep: 0, bytes: 10 },
            Segment { kind: Kind::Logs, day: Day::from_days(2), keep: 0, bytes: 5 },
            Segment { kind: Kind::Traces, day: Day::from_days(3), keep: 0, bytes: 1 },
        ];
        let status =
            StorageStatus::new(RetentionPolicy::default(), &segments, 7, Some(100), Day::from_days(4));
        assert_eq!(status.segments.len(), 3);
        assert_eq!(status.forecast.map(|f| f.days), Some(2));
        assert_eq!(status.segment_bytes(), 16);
        let logs = status.kinds[0];
        assert_eq!(
            (logs.kind, logs.bytes, logs.segments, logs.oldest),
            (Kind::Logs, 15, 2, Some(Day::from_days(2)))
        );
        assert_eq!(status.kinds[1].oldest, None);
        assert_eq!(status.kinds[2].segments, 1);
    }

    fn day(n: i64, kind: Kind, keep: u32, bytes: u64) -> Segment {
        Segment { kind, day: Day::from_days(n), keep, bytes }
    }

    #[test]
    fn tells_where_the_data_is_heading() {
        let policy = RetentionPolicy {
            logs_days: 10,
            events_days: 30,
            traces_days: 2,
            max_storage_bytes: 0,
            min_free_bytes: 50,
            ..RetentionPolicy::default()
        };
        // Two full days and today: logs 100 a day, a stage's own (3 days)
        // 10, events 20, traces 60.
        let mut segments = Vec::new();
        for n in [8, 9] {
            segments.extend([
                day(n, Kind::Logs, 0, 100),
                day(n, Kind::Logs, 3, 10),
                day(n, Kind::Events, 0, 20),
                day(n, Kind::Traces, 0, 60),
            ]);
        }
        segments.push(day(10, Kind::Logs, 0, 40));
        let today = Day::from_days(10);
        let forecast = Forecast::new(&policy, &segments, today, None).unwrap();
        assert_eq!((forecast.days, forecast.daily_bytes), (2, 190));
        assert_eq!(forecast.settled_bytes, 100 * 10 + 10 * 3 + 20 * 30 + 60 * 2);
        assert_eq!((forecast.limit_in_days, forecast.disk_in_days), (None, None));

        // Now: 420. The limit of 1,000 is reached in 4 days, the disk's
        // reserve (500 free less 50) in 3.
        let limited = RetentionPolicy { max_storage_bytes: 1_000, ..policy };
        let forecast = Forecast::new(&limited, &segments, today, Some(500)).unwrap();
        assert_eq!(forecast.limit_in_days, Some(4), "580 more at 190 a day");
        assert_eq!(forecast.disk_in_days, Some(3), "450 more");
        let roomy = Forecast::new(&limited, &segments, today, Some(10_000)).unwrap();
        assert_eq!(roomy.disk_in_days, None, "the limit comes first");
        assert!(Forecast::new(&policy, &segments[8..], today, None).is_none(), "today only");
    }
}
