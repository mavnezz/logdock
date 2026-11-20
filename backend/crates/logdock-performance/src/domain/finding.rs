//! Performance problems a single transaction shows, as Sentry and Nightwatch
//! report them: the same query again and again (N+1), a slow query, and a
//! request or job over its threshold.

use logdock_kernel::performance::PerformanceRules;

use super::transaction::{Kind, Span, Transaction, normalize_sql};

/// An N+1 query is at least this many of the same query in a row…
pub const N_PLUS_ONE_REPEATS: usize = 5;
/// …that took at least this long together, as Sentry detects it.
pub const N_PLUS_ONE_TOTAL_US: u64 = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    NPlusOne,
    SlowQuery,
    SlowRequest,
    SlowJob,
}

impl FindingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NPlusOne => "n_plus_one",
            Self::SlowQuery => "slow_query",
            Self::SlowRequest => "slow_request",
            Self::SlowJob => "slow_job",
        }
    }
}

/// One problem in one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: FindingKind,
    /// The request or job it happened in, as the rollups name it.
    pub transaction: String,
    /// What it is about: the statement, or the transaction itself.
    pub subject: String,
    /// The span to look at first; `None` for the transaction.
    pub span_id: Option<String>,
    /// How many spans it covers.
    pub count: u32,
    /// The time they took together, or the transaction's.
    pub duration_us: u64,
    pub threshold_us: Option<u64>,
}

impl Finding {
    /// The issue's title: the same for every occurrence.
    pub fn title(&self) -> String {
        let ms = |us: u64| us / 1_000;
        match self.kind {
            FindingKind::NPlusOne => format!("N+1 query: {}", self.subject),
            FindingKind::SlowQuery => format!("Slow query: {}", self.subject),
            FindingKind::SlowRequest => {
                format!("Slow request: {} over {} ms", self.transaction, ms(self.threshold_us.unwrap_or(0)))
            }
            FindingKind::SlowJob => {
                format!("Slow job: {} over {} ms", self.transaction, ms(self.threshold_us.unwrap_or(0)))
            }
        }
    }

    /// What groups occurrences into one issue.
    pub fn fingerprint(&self) -> Vec<String> {
        vec!["performance".into(), self.kind.as_str().into(), self.transaction.clone(), self.subject.clone()]
    }
}

fn is_query(span: &Span) -> bool {
    span.op.starts_with("db.sql") || span.op == "db.query" || span.op == "db"
}

/// What `transaction` shows of the problems `rules` look for.
pub fn findings(transaction: &Transaction, rules: &PerformanceRules) -> Vec<Finding> {
    let name = transaction.display_name();
    let mut found = Vec::new();
    let job = transaction.kind == Kind::Job;
    if (transaction.kind == Kind::Request || job)
        && let Some(ms) = rules.threshold(&name, job)
        && transaction.duration_us > u64::from(ms) * 1_000
    {
        found.push(Finding {
            kind: if job { FindingKind::SlowJob } else { FindingKind::SlowRequest },
            transaction: name.clone(),
            subject: name.clone(),
            span_id: None,
            count: 1,
            duration_us: transaction.duration_us,
            threshold_us: Some(u64::from(ms) * 1_000),
        });
    }
    if rules.n_plus_one {
        found.extend(n_plus_one(transaction, &name));
    }
    if let Some(ms) = rules.slow_query_ms {
        let limit = u64::from(ms) * 1_000;
        let mut seen: Vec<String> = Vec::new();
        for span in transaction.spans.iter().filter(|s| is_query(s) && s.duration_us >= limit) {
            let Some(sql) = span.description.as_deref() else { continue };
            if !sql.trim_start().get(..6).is_some_and(|w| w.eq_ignore_ascii_case("select")) {
                continue;
            }
            let statement = normalize_sql(sql);
            if seen.contains(&statement) {
                continue;
            }
            seen.push(statement.clone());
            found.push(Finding {
                kind: FindingKind::SlowQuery,
                transaction: name.clone(),
                subject: statement,
                span_id: Some(span.span_id.clone()),
                count: 1,
                duration_us: span.duration_us,
                threshold_us: Some(limit),
            });
        }
    }
    found
}

/// Runs of the same statement under one parent, each after the one before.
fn n_plus_one(transaction: &Transaction, name: &str) -> Vec<Finding> {
    let mut parents: Vec<&str> = Vec::new();
    for span in &transaction.spans {
        let parent = span.parent_span_id.as_deref().unwrap_or(&transaction.span_id);
        if !parents.contains(&parent) {
            parents.push(parent);
        }
    }
    let mut found: Vec<Finding> = Vec::new();
    for parent in parents {
        let mut queries: Vec<&Span> = transaction
            .spans
            .iter()
            .filter(|s| s.parent_span_id.as_deref().unwrap_or(&transaction.span_id) == parent && is_query(s))
            .collect();
        queries.sort_by_key(|s| s.start.unix_micros());
        let mut i = 0;
        while i < queries.len() {
            let statement = queries[i].description.as_deref().map(normalize_sql);
            let mut j = i + 1;
            while j < queries.len() {
                let previous = queries[j - 1];
                let after = queries[j].start.unix_micros()
                    >= previous.start.unix_micros() + previous.duration_us as i64 - 1;
                if !after || queries[j].description.as_deref().map(normalize_sql) != statement {
                    break;
                }
                j += 1;
            }
            let run = &queries[i..j];
            let total: u64 = run.iter().map(|s| s.duration_us).sum();
            if let Some(statement) = statement
                && run.len() >= N_PLUS_ONE_REPEATS
                && total >= N_PLUS_ONE_TOTAL_US
            {
                // The same statement under another parent is the same problem.
                match found.iter_mut().find(|f| f.subject == statement) {
                    Some(known) if known.count >= run.len() as u32 => {}
                    Some(known) => {
                        known.count = run.len() as u32;
                        known.duration_us = total;
                        known.span_id = Some(run[0].span_id.clone());
                    }
                    None => found.push(Finding {
                        kind: FindingKind::NPlusOne,
                        transaction: name.to_owned(),
                        subject: statement,
                        span_id: Some(run[0].span_id.clone()),
                        count: run.len() as u32,
                        duration_us: total,
                        threshold_us: None,
                    }),
                }
            }
            i = j;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use logdock_kernel::{Timestamp, Value};

    use super::*;

    fn query(id: &str, parent: &str, start_ms: i64, ms: u64, sql: &str) -> Span {
        Span {
            span_id: id.into(),
            parent_span_id: Some(parent.into()),
            op: "db.sql.query".into(),
            description: Some(sql.into()),
            start: Timestamp::from_unix_micros(1_800_000_000_000_000 + start_ms * 1_000),
            duration_us: ms * 1_000,
            status: None,
            data: Value::Null,
        }
    }

    fn transaction(kind: Kind, ms: u64, spans: Vec<Span>) -> Transaction {
        Transaction {
            trace_id: "t".repeat(32),
            span_id: "root".into(),
            parent_span_id: None,
            name: if kind == Kind::Job { "App\\Jobs\\ChargeOrder".into() } else { "/orders/{order}".into() },
            op: "http.server".into(),
            kind,
            status: Some("ok".into()),
            http_method: (kind == Kind::Request).then(|| "GET".into()),
            http_status: None,
            start: Timestamp::from_unix_micros(1_800_000_000_000_000),
            duration_us: ms * 1_000,
            release: None,
            environment: None,
            service: None,
            host: None,
            container: None,
            queue: None,
            spans,
        }
    }

    fn loop_of(n: i64, ms: u64) -> Vec<Span> {
        (0..n)
            .map(|i| {
                query(&format!("q{i}"), "root", i * 20, ms, &format!("select * from items where id = {i}"))
            })
            .collect()
    }

    #[test]
    fn finds_the_same_query_in_a_row_once_it_costs() {
        let rules = PerformanceRules::default();
        let found = findings(&transaction(Kind::Request, 300, loop_of(12, 10)), &rules);
        assert_eq!(found.len(), 1, "{found:?}");
        let n1 = &found[0];
        assert_eq!(n1.kind, FindingKind::NPlusOne);
        assert_eq!((n1.count, n1.duration_us, n1.span_id.as_deref()), (12, 120_000, Some("q0")));
        assert_eq!(n1.title(), "N+1 query: select * from items where id = ?");
        assert_eq!(n1.fingerprint()[..3], ["performance", "n_plus_one", "GET /orders/{order}"]);

        // Too few, or too fast together: no finding.
        assert!(findings(&transaction(Kind::Request, 300, loop_of(4, 30)), &rules).is_empty());
        assert!(findings(&transaction(Kind::Request, 300, loop_of(12, 1)), &rules).is_empty());
        let off = PerformanceRules { n_plus_one: false, ..PerformanceRules::default() };
        assert!(findings(&transaction(Kind::Request, 300, loop_of(12, 10)), &off).is_empty());
    }

    #[test]
    fn a_run_ends_where_another_statement_or_an_overlap_comes() {
        let mut spans = loop_of(3, 20);
        spans.push(query("other", "root", 60, 20, "select * from users"));
        spans.extend(
            (3..6).map(|i| {
                query(&format!("q{i}"), "root", 80 + i * 20, 20, "select * from items where id = 1")
            }),
        );
        // Five in all, but not in a row.
        assert!(findings(&transaction(Kind::Request, 300, spans), &PerformanceRules::default()).is_empty());
        let parallel: Vec<Span> =
            (0..6).map(|i| query(&format!("p{i}"), "root", 0, 20, "select 1")).collect();
        assert!(
            findings(&transaction(Kind::Request, 300, parallel), &PerformanceRules::default()).is_empty()
        );
    }

    #[test]
    fn finds_slow_selects_once_per_statement() {
        let spans = vec![
            query("a", "root", 0, 1_200, "select * from reports where day = '2026-11-20'"),
            query("b", "root", 1_300, 1_500, "select * from reports where day = '2026-11-21'"),
            query("c", "root", 2_900, 1_500, "update reports set seen = 1"),
        ];
        let found = findings(&transaction(Kind::Command, 5_000, spans), &PerformanceRules::default());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].title(), "Slow query: select * from reports where day = ?");
        assert_eq!(found[0].span_id.as_deref(), Some("a"));
    }

    #[test]
    fn finds_requests_and_jobs_over_their_threshold() {
        let rules = PerformanceRules {
            job_ms: Some(10_000),
            overrides: vec![("GET /orders/{order}".into(), 500)],
            ..PerformanceRules::default()
        };
        let slow = findings(&transaction(Kind::Request, 800, Vec::new()), &rules);
        assert_eq!(slow[0].title(), "Slow request: GET /orders/{order} over 500 ms");
        assert!(findings(&transaction(Kind::Request, 400, Vec::new()), &rules).is_empty());
        let job = findings(&transaction(Kind::Job, 12_000, Vec::new()), &rules);
        assert_eq!(job[0].kind, FindingKind::SlowJob);
        assert_eq!(job[0].title(), "Slow job: App\\Jobs\\ChargeOrder over 10000 ms");
        assert!(
            findings(&transaction(Kind::Command, 60_000, Vec::new()), &rules).is_empty(),
            "commands have none"
        );
        for kind in
            [FindingKind::NPlusOne, FindingKind::SlowQuery, FindingKind::SlowRequest, FindingKind::SlowJob]
        {
            assert!(!kind.as_str().is_empty());
        }
    }
}
