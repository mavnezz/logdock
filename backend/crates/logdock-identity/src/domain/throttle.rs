use std::collections::HashMap;
use std::time::Duration;

use logdock_kernel::Timestamp;

/// How many failed sign-ins are allowed before a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrottlePolicy {
    /// Failures within `window` before attempts are refused.
    pub max_failures: u32,
    pub window: Duration,
    /// How long attempts are refused then.
    pub lockout: Duration,
}

impl Default for ThrottlePolicy {
    fn default() -> Self {
        Self { max_failures: 5, window: Duration::from_secs(15 * 60), lockout: Duration::from_secs(5 * 60) }
    }
}

#[derive(Debug, Clone, Copy)]
struct Failures {
    count: u32,
    first_at: Timestamp,
    locked_until: Option<Timestamp>,
}

/// Failed sign-ins per key (an address, an e-mail address), kept in memory:
/// a restart forgives them, which is acceptable for a pause of minutes.
#[derive(Debug, Default)]
pub struct LoginThrottle {
    policy: ThrottlePolicy,
    failures: HashMap<String, Failures>,
}

impl LoginThrottle {
    pub fn new(policy: ThrottlePolicy) -> Self {
        Self { policy, failures: HashMap::new() }
    }

    /// `Err` with the time left while `key` is locked out.
    pub fn check(&self, key: &str, now: Timestamp) -> Result<(), Duration> {
        match self.failures.get(key).and_then(|f| f.locked_until) {
            Some(until) if now < until => Err(until.duration_since(now)),
            _ => Ok(()),
        }
    }

    pub fn fail(&mut self, key: &str, now: Timestamp) {
        let policy = self.policy;
        let entry = self.failures.entry(key.to_owned()).or_insert(Failures {
            count: 0,
            first_at: now,
            locked_until: None,
        });
        let window_over = now.duration_since(entry.first_at) >= policy.window;
        let lock_over = entry.locked_until.is_some_and(|until| now >= until);
        if window_over || lock_over {
            *entry = Failures { count: 0, first_at: now, locked_until: None };
        }
        entry.count += 1;
        if entry.count >= policy.max_failures {
            entry.locked_until = Some(now.saturating_add(policy.lockout));
        }
        // Bounded memory: forget what has run out.
        if self.failures.len() > 10_000 {
            self.failures.retain(|_, f| {
                now.duration_since(f.first_at) < policy.window || f.locked_until.is_some_and(|u| now < u)
            });
        }
    }

    pub fn succeed(&mut self, key: &str) {
        self.failures.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_unix_seconds(seconds)
    }

    #[test]
    fn locks_out_after_too_many_failures_for_a_while() {
        let policy = ThrottlePolicy {
            max_failures: 3,
            window: Duration::from_secs(60),
            lockout: Duration::from_secs(30),
        };
        let mut throttle = LoginThrottle::new(policy);
        for second in 0..2 {
            throttle.fail("ip", at(second));
            assert!(throttle.check("ip", at(second)).is_ok());
        }
        throttle.fail("ip", at(2));
        assert_eq!(throttle.check("ip", at(12)), Err(Duration::from_secs(20)));
        assert!(throttle.check("other", at(12)).is_ok());
        assert!(throttle.check("ip", at(32)).is_ok());
        // After the lockout, counting starts over.
        throttle.fail("ip", at(40));
        assert!(throttle.check("ip", at(40)).is_ok());
    }

    #[test]
    fn forgives_old_failures_and_successes() {
        let policy = ThrottlePolicy {
            max_failures: 2,
            window: Duration::from_secs(60),
            lockout: Duration::from_secs(30),
        };
        let mut throttle = LoginThrottle::new(policy);
        throttle.fail("a", at(0));
        throttle.fail("a", at(61));
        assert!(throttle.check("a", at(61)).is_ok(), "the first failure fell out of the window");
        throttle.succeed("a");
        throttle.fail("a", at(62));
        assert!(throttle.check("a", at(62)).is_ok());
        assert_eq!(LoginThrottle::default().check("x", at(0)), Ok(()));
    }
}
