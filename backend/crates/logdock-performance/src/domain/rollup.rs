//! What a minute of one kind and name adds up to: counts and a histogram of
//! durations, from which percentiles are estimated.

/// Upper bounds of the histogram's buckets in microseconds; the last bucket
/// holds everything above.
pub const BUCKET_BOUNDS_US: [u64; BUCKETS - 1] = [
    1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000, 2_500_000, 5_000_000,
    10_000_000, 30_000_000,
];
pub const BUCKETS: usize = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rollup {
    pub count: u64,
    /// Failed requests, jobs, commands, queries or outgoing requests; cache
    /// misses.
    pub flagged: u64,
    pub total_us: u64,
    pub max_us: u64,
    pub buckets: [u64; BUCKETS],
}

impl Rollup {
    /// Counts one measurement `weight` times: a sampled transaction stands
    /// for the ones that were not sent.
    pub fn add(&mut self, duration_us: u64, flagged: bool, weight: u64) {
        self.count += weight;
        if flagged {
            self.flagged += weight;
        }
        self.total_us = self.total_us.saturating_add(duration_us.saturating_mul(weight));
        self.max_us = self.max_us.max(duration_us);
        let bucket = BUCKET_BOUNDS_US.iter().position(|bound| duration_us <= *bound).unwrap_or(BUCKETS - 1);
        self.buckets[bucket] += weight;
    }

    pub fn merge(&mut self, other: &Rollup) {
        self.count += other.count;
        self.flagged += other.flagged;
        self.total_us = self.total_us.saturating_add(other.total_us);
        self.max_us = self.max_us.max(other.max_us);
        for (mine, theirs) in self.buckets.iter_mut().zip(other.buckets) {
            *mine += theirs;
        }
    }

    pub fn average_us(&self) -> u64 {
        self.total_us.checked_div(self.count).unwrap_or(0)
    }

    /// The `q`-quantile (0.0 to 1.0), interpolated within its bucket; the
    /// last bucket reaches up to the longest duration seen.
    pub fn percentile_us(&self, q: f64) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let rank = (q.clamp(0.0, 1.0) * self.count as f64).max(1.0);
        let mut seen = 0.0;
        for (index, count) in self.buckets.iter().enumerate() {
            if *count == 0 {
                continue;
            }
            let before = seen;
            seen += *count as f64;
            if seen >= rank {
                let lower = if index == 0 { 0 } else { BUCKET_BOUNDS_US[index - 1] };
                let upper =
                    BUCKET_BOUNDS_US.get(index).copied().unwrap_or(self.max_us).min(self.max_us).max(lower);
                let within = (rank - before) / *count as f64;
                return lower + ((upper - lower) as f64 * within).round() as u64;
            }
        }
        self.max_us
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_durations_into_buckets_and_estimates_percentiles() {
        let mut rollup = Rollup::default();
        for ms in [3, 4, 4, 8, 9, 40, 45, 120, 900, 4_000] {
            rollup.add(ms * 1000, ms > 1000, 1);
        }
        assert_eq!(rollup.count, 10);
        assert_eq!(rollup.flagged, 1);
        assert_eq!(rollup.max_us, 4_000_000);
        assert_eq!(rollup.average_us(), 513_300);
        let p50 = rollup.percentile_us(0.5);
        assert!((5_000..=10_000).contains(&p50), "{p50}");
        let p99 = rollup.percentile_us(0.99);
        assert!((2_500_000..=4_000_000).contains(&p99), "{p99}");
        assert_eq!(Rollup::default().percentile_us(0.5), 0);
    }

    #[test]
    fn weighs_sampled_measurements_and_merges() {
        let mut a = Rollup::default();
        a.add(2_000, false, 10);
        let mut b = Rollup::default();
        b.add(60_000_000, true, 1);
        a.merge(&b);
        assert_eq!(a.count, 11);
        assert_eq!(a.flagged, 1);
        assert_eq!(a.buckets[1], 10);
        assert_eq!(a.buckets[BUCKETS - 1], 1);
        assert_eq!(a.percentile_us(1.0), 60_000_000);
        assert_eq!(a.total_us, 60_020_000);
    }
}
