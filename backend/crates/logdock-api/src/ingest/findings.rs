//! Performance problems found in a transaction, as error events of their own
//! kind: one issue per problem, with the trace to look at.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use logdock_performance::domain::{Finding, Transaction};
use serde_json::{Value, json};

/// A problem is reported once a minute at most: a busy route with an N+1
/// query would otherwise store an event for every request.
const REPORT_EVERY: Duration = Duration::from_secs(60);

/// When each problem was reported last, by stage and fingerprint.
#[derive(Debug, Default)]
pub(crate) struct Reported(Mutex<HashMap<String, Instant>>);

impl Reported {
    /// Whether `key` may be reported now; if so, it counts as reported.
    pub(crate) fn take(&self, key: String) -> bool {
        let mut reported = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if reported.len() > 10_000 {
            reported.retain(|_, at| now.duration_since(*at) < REPORT_EVERY);
        }
        match reported.get(&key) {
            Some(at) if now.duration_since(*at) < REPORT_EVERY => false,
            _ => {
                reported.insert(key, now);
                true
            }
        }
    }
}

/// The same for the same occurrence, so a transaction sent twice counts once.
fn event_id(transaction: &Transaction, finding: &Finding) -> String {
    let parts = (&transaction.trace_id, &transaction.span_id, finding.kind.as_str(), &finding.subject);
    let mut first = DefaultHasher::new();
    parts.hash(&mut first);
    let mut second = DefaultHasher::new();
    ("logdock", parts).hash(&mut second);
    format!("{:016x}{:016x}", first.finish(), second.finish())
}

/// The event an issue of `finding` gets, as an SDK would send it.
pub(crate) fn event(transaction: &Transaction, finding: &Finding, stage: &str) -> Value {
    let ms = |us: u64| (us as f64 / 1_000.0 * 10.0).round() / 10.0;
    let mut details = json!({
        "kind": finding.kind.as_str(),
        "transaction": finding.transaction,
        "transaction_ms": ms(transaction.duration_us),
        "count": finding.count,
        "total_ms": ms(finding.duration_us),
    });
    if let Some(threshold) = finding.threshold_us {
        details["threshold_ms"] = json!(ms(threshold));
    }
    if finding.subject != finding.transaction {
        details["statement"] = json!(finding.subject);
    }
    // The message is the issue's title, the same every time; how often and
    // how long are in the context.
    let message = finding.title();
    json!({
        "event_id": event_id(transaction, finding),
        "timestamp": transaction.start.unix_micros() as f64 / 1_000_000.0,
        "level": "warning",
        "logger": "logdock.performance",
        "message": message,
        "transaction": finding.transaction,
        "release": transaction.release,
        "environment": stage,
        "server_name": transaction.container,
        "tags": {
            "issue.type": "performance",
            "performance": finding.kind.as_str(),
            "service": transaction.service,
            "host": transaction.host,
        },
        "contexts": {
            "trace": {
                "trace_id": transaction.trace_id,
                "span_id": finding.span_id.as_deref().unwrap_or(&transaction.span_id),
            },
            "performance": details,
        },
        "fingerprint": finding.fingerprint(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_problem_once_a_minute() {
        let reported = Reported::default();
        assert!(reported.take("a".into()));
        assert!(!reported.take("a".into()));
        assert!(reported.take("b".into()));
    }
}
