//! When a project's requests, jobs and queries become performance issues.
//! Kept with the project's settings, applied by the Performance context.

/// The thresholds and checks of one project. A threshold of `None` is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformanceRules {
    /// A request slower than this many milliseconds is an issue.
    pub request_ms: Option<u32>,
    /// A queued job slower than this.
    pub job_ms: Option<u32>,
    /// Thresholds of single routes or jobs by name (`POST /payments`,
    /// `App\Jobs\ChargeOrder`), before the two above; 0 turns one off.
    pub overrides: Vec<(String, u32)>,
    /// The same query run again and again in a row (N+1).
    pub n_plus_one: bool,
    /// A single `SELECT` slower than this.
    pub slow_query_ms: Option<u32>,
}

impl Default for PerformanceRules {
    /// As Nightwatch and Sentry start: requests over 2 s, N+1 queries,
    /// queries over 1 s; jobs may take long.
    fn default() -> Self {
        Self {
            request_ms: Some(2_000),
            job_ms: None,
            overrides: Vec::new(),
            n_plus_one: true,
            slow_query_ms: Some(1_000),
        }
    }
}

impl PerformanceRules {
    /// The threshold of a route or job named `name`, in milliseconds.
    pub fn threshold(&self, name: &str, job: bool) -> Option<u32> {
        match self.overrides.iter().find(|(n, _)| n == name) {
            Some((_, 0)) => None,
            Some((_, ms)) => Some(*ms),
            None if job => self.job_ms,
            None => self.request_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_s_own_threshold_comes_first() {
        let rules = PerformanceRules {
            overrides: vec![("POST /payments".into(), 5_000), ("GET /health".into(), 0)],
            ..PerformanceRules::default()
        };
        assert_eq!(rules.threshold("POST /payments", false), Some(5_000));
        assert_eq!(rules.threshold("GET /health", false), None, "0 is off");
        assert_eq!(rules.threshold("GET /orders", false), Some(2_000));
        assert_eq!(rules.threshold("App\\Jobs\\ChargeOrder", true), None, "jobs have none by default");
    }
}
