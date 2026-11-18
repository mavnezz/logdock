use std::time::{SystemTime, UNIX_EPOCH};

use logdock_kernel::{Clock, Timestamp};

/// The system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        Timestamp::from_unix_micros(i64::try_from(since.as_micros()).unwrap_or(i64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_time_after_2026() {
        assert!(SystemClock.now() > Timestamp::from_unix_seconds(1_767_225_600));
    }
}
