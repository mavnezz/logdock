use std::fmt;

use logdock_kernel::{EnvironmentId, ProjectId, Timestamp, Value};

use super::{Level, LogEntry, LogId};

/// One part of what was typed into the search field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// A word; with `prefix`, typed as `word*`.
    Word { text: String, prefix: bool },
    /// Words in this order: `"payment declined"`.
    Phrase(String),
    /// An attribute's value: `order_id:42`, `user.email:"a b"`.
    Attribute { key: String, value: String },
}

/// What was typed into the search field, parsed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchText {
    pub terms: Vec<Term>,
}

impl SearchText {
    pub const MAX_TERMS: usize = 20;

    pub fn parse(text: &str) -> Self {
        let mut terms = Vec::new();
        let mut chars = text.chars().peekable();
        while terms.len() < Self::MAX_TERMS {
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            let Some(&first) = chars.peek() else { break };
            if first == '"' {
                chars.next();
                let phrase: String = chars.by_ref().take_while(|c| *c != '"').collect();
                let phrase = phrase.trim().to_owned();
                if !phrase.is_empty() {
                    terms.push(Term::Phrase(phrase));
                }
                continue;
            }
            let mut token = String::new();
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                token.push(c);
                // key:"quoted value"
                if c == ':' && chars.peek() == Some(&'"') {
                    chars.next();
                    token.extend(chars.by_ref().take_while(|c| *c != '"'));
                    break;
                }
            }
            if let Some(term) = Self::token(&token) {
                terms.push(term);
            }
        }
        Self { terms }
    }

    fn token(token: &str) -> Option<Term> {
        if let Some((key, value)) = token.split_once(':')
            && !key.is_empty()
            && !value.is_empty()
            && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        {
            return Some(Term::Attribute { key: key.to_owned(), value: value.to_owned() });
        }
        let prefix = token.ends_with('*');
        let text = token.trim_end_matches('*').trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        (!text.is_empty()).then(|| Term::Word { text: text.to_owned(), prefix })
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The terms full-text search answers: words and phrases.
    pub fn text_terms(&self) -> impl Iterator<Item = &Term> {
        self.terms.iter().filter(|t| !matches!(t, Term::Attribute { .. }))
    }

    pub fn attributes(&self) -> impl Iterator<Item = (&str, &str)> {
        self.terms.iter().filter_map(|t| match t {
            Term::Attribute { key, value } => Some((key.as_str(), value.as_str())),
            _ => None,
        })
    }

    /// Removes and returns the value of an attribute term with one of these
    /// keys, so it can become a filter of its own (`level:error`).
    pub fn take(&mut self, keys: &[&str]) -> Option<String> {
        let at = self
            .terms
            .iter()
            .position(|t| matches!(t, Term::Attribute { key, .. } if keys.contains(&key.as_str())))?;
        match self.terms.remove(at) {
            Term::Attribute { value, .. } => Some(value),
            _ => None,
        }
    }
}

/// Where a page ends: the next page starts after this entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub time: Timestamp,
    pub id: LogId,
}

impl Cursor {
    pub fn parse(text: &str) -> Option<Self> {
        let (time, id) = text.split_once('_')?;
        Some(Self { time: Timestamp::from_unix_micros(time.parse().ok()?), id: LogId::parse(id)? })
    }

    /// Whether `time, id` comes after this cursor in newest-first order.
    pub fn is_before(&self, time: Timestamp, id: LogId) -> bool {
        (time, id.day, id.keep, id.row) < (self.time, self.id.day, self.id.keep, self.id.row)
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.time.unix_micros(), self.id)
    }
}

/// What a search asks for. Empty lists mean "all".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LogQuery {
    pub projects: Vec<ProjectId>,
    pub environments: Vec<EnvironmentId>,
    pub services: Vec<String>,
    pub hosts: Vec<String>,
    pub levels: Vec<Level>,
    pub from: Option<Timestamp>,
    pub to: Option<Timestamp>,
    pub text: SearchText,
    pub trace_id: Option<String>,
    pub before: Option<Cursor>,
    pub limit: u32,
}

impl LogQuery {
    pub const MAX_LIMIT: u32 = 500;

    /// Whether `entry` belongs to the result; the live view filters with
    /// this. Words match parts of words here, not only their beginnings.
    pub fn matches(&self, entry: &LogEntry) -> bool {
        let listed = |list: &[String], value: &Option<String>| {
            list.is_empty() || value.as_ref().is_some_and(|v| list.iter().any(|l| l == v))
        };
        let in_time =
            self.from.is_none_or(|from| entry.time >= from) && self.to.is_none_or(|to| entry.time < to);
        let scoped = (self.projects.is_empty() || self.projects.contains(&entry.scope.project))
            && (self.environments.is_empty() || self.environments.contains(&entry.scope.environment));
        let levels = self.levels.is_empty() || self.levels.contains(&entry.level);
        let trace = self.trace_id.as_ref().is_none_or(|t| entry.trace_id.as_ref() == Some(t));
        if !(in_time
            && scoped
            && levels
            && trace
            && listed(&self.services, &entry.service)
            && listed(&self.hosts, &entry.host))
        {
            return false;
        }
        let haystack = searchable(entry).to_lowercase();
        self.text.terms.iter().all(|term| match term {
            Term::Word { text, .. } | Term::Phrase(text) => haystack.contains(&text.to_lowercase()),
            Term::Attribute { key, value } => entry
                .attributes
                .iter()
                .any(|(k, v)| k == key && v.to_text().is_some_and(|t| t.eq_ignore_ascii_case(value))),
        })
    }
}

/// The text full-text search sees of an entry: its message and its
/// attributes as `key value` pairs.
pub fn searchable(entry: &LogEntry) -> String {
    let mut text = entry.message.clone();
    for (key, value) in &entry.attributes {
        text.push(' ');
        text.push_str(key);
        text.push(' ');
        text.push_str(&value_text(value));
    }
    text
}

fn value_text(value: &Value) -> String {
    match value {
        Value::Array(items) => items.iter().map(value_text).collect::<Vec<_>>().join(" "),
        Value::Object(fields) => {
            fields.iter().map(|(k, v)| format!("{k} {}", value_text(v))).collect::<Vec<_>>().join(" ")
        }
        other => other.to_text().unwrap_or_default(),
    }
}

/// One page of a search, newest first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LogPage {
    pub entries: Vec<LogEntry>,
    /// Where the next page starts; `None` on the last one.
    pub next: Option<Cursor>,
}

/// Counts per level over time, for the chart above the list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HistogramQuery {
    pub projects: Vec<ProjectId>,
    pub environments: Vec<EnvironmentId>,
    pub services: Vec<String>,
    pub levels: Vec<Level>,
    pub from: Timestamp,
    pub to: Timestamp,
    pub bucket_seconds: i64,
}

impl HistogramQuery {
    /// About this many buckets fit a chart.
    pub const BUCKETS: i64 = 60;

    /// A bucket size for the range: whole minutes, hours or days.
    pub fn bucket_for(from: Timestamp, to: Timestamp) -> i64 {
        let span = (to.unix_seconds() - from.unix_seconds()).max(60);
        let raw = span / Self::BUCKETS;
        [60, 300, 600, 900, 1800, 3600, 3 * 3600, 6 * 3600, 12 * 3600, 86_400]
            .into_iter()
            .find(|size| *size >= raw)
            .unwrap_or(86_400)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    /// Start, in Unix seconds.
    pub start: i64,
    /// Entries per level, `counts[level.number() - 1]`.
    pub counts: [u64; 6],
}

/// Where entries came from, for the filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub project: ProjectId,
    pub environment: EnvironmentId,
    pub service: Option<String>,
    pub host: Option<String>,
    pub last_seen: Timestamp,
}

#[cfg(test)]
mod tests {
    use logdock_kernel::{Day, Scope};

    use super::*;

    fn word(text: &str) -> Term {
        Term::Word { text: text.into(), prefix: false }
    }

    #[test]
    fn parses_words_phrases_prefixes_and_attributes() {
        let text = SearchText::parse(
            r#"  payment "card declined" Order* order_id:42 user.email:"ada l" level:error :x x: (§) "#,
        );
        assert_eq!(
            text.terms,
            [
                word("payment"),
                Term::Phrase("card declined".into()),
                Term::Word { text: "Order".into(), prefix: true },
                Term::Attribute { key: "order_id".into(), value: "42".into() },
                Term::Attribute { key: "user.email".into(), value: "ada l".into() },
                Term::Attribute { key: "level".into(), value: "error".into() },
                word("x"),
                word("x"),
            ]
        );
        assert_eq!(text.text_terms().count(), 5);
        assert_eq!(
            text.attributes().collect::<Vec<_>>(),
            [("order_id", "42"), ("user.email", "ada l"), ("level", "error")]
        );
        let mut text = text;
        assert_eq!(text.take(&["level"]).as_deref(), Some("error"));
        assert_eq!(text.take(&["level"]), None);
        assert!(SearchText::parse("   ").is_empty());
        assert_eq!(SearchText::parse(&"w ".repeat(50)).terms.len(), SearchText::MAX_TERMS);
        assert_eq!(SearchText::parse("\"unclosed phrase").terms, [Term::Phrase("unclosed phrase".into())]);
    }

    fn entry() -> LogEntry {
        LogEntry {
            id: None,
            scope: Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) },
            time: Timestamp::from_unix_seconds(100),
            received: Timestamp::from_unix_seconds(100),
            level: Level::Warn,
            message: "Payment declined for order".into(),
            template: None,
            attributes: vec![
                ("order_id".into(), Value::Int(42)),
                (
                    "user".into(),
                    Value::Object(vec![("email".into(), Value::String("ada@example.com".into()))]),
                ),
                ("tags".into(), Value::Array(vec![Value::String("vip".into())])),
            ],
            trace_id: Some("t1".into()),
            span_id: None,
            service: Some("horizon".into()),
            host: Some("docker-host-02".into()),
            container: None,
            release: None,
            user_id: None,
            redactions: 0,
            truncated: false,
        }
    }

    #[test]
    fn the_live_view_filters_like_the_search() {
        let entry = entry();
        let query = |text: &str| LogQuery { text: SearchText::parse(text), ..LogQuery::default() };
        assert!(query("").matches(&entry));
        assert!(query("payment DECLINED").matches(&entry));
        assert!(query("\"declined for\"").matches(&entry));
        assert!(query("order_id:42 vip").matches(&entry));
        assert!(query("ada@example").matches(&entry));
        assert!(!query("refund").matches(&entry));
        assert!(!query("order_id:43").matches(&entry));
        let filtered = LogQuery {
            projects: vec![ProjectId::new(1)],
            environments: vec![EnvironmentId::new(2)],
            services: vec!["horizon".into()],
            hosts: vec!["docker-host-02".into()],
            levels: vec![Level::Warn, Level::Error],
            from: Some(Timestamp::from_unix_seconds(100)),
            to: Some(Timestamp::from_unix_seconds(101)),
            trace_id: Some("t1".into()),
            ..LogQuery::default()
        };
        assert!(filtered.matches(&entry));
        assert!(!LogQuery { services: vec!["web".into()], ..filtered.clone() }.matches(&entry));
        assert!(!LogQuery { levels: vec![Level::Error], ..filtered.clone() }.matches(&entry));
        assert!(
            !LogQuery { to: Some(Timestamp::from_unix_seconds(100)), ..filtered.clone() }.matches(&entry)
        );
        assert!(!LogQuery { trace_id: Some("t2".into()), ..filtered }.matches(&entry));
    }

    #[test]
    fn cursors_are_written_read_and_ordered() {
        let id = LogId { day: Day::from_days(20_000), keep: 0, row: 5 };
        let cursor = Cursor { time: Timestamp::from_unix_micros(1_000), id };
        assert_eq!(cursor.to_string(), "1000_20000-5");
        assert_eq!(Cursor::parse("1000_20000-5"), Some(cursor));
        assert_eq!(Cursor::parse("1000"), None);
        assert!(cursor.is_before(Timestamp::from_unix_micros(999), id));
        assert!(cursor.is_before(Timestamp::from_unix_micros(1_000), LogId { row: 4, ..id }));
        assert!(!cursor.is_before(Timestamp::from_unix_micros(1_000), LogId { keep: 3, row: 4, ..id }));
        assert!(!cursor.is_before(Timestamp::from_unix_micros(1_000), id));
    }

    #[test]
    fn picks_bucket_sizes_that_fit_a_chart() {
        let at = Timestamp::from_unix_seconds;
        assert_eq!(HistogramQuery::bucket_for(at(0), at(3600)), 60);
        assert_eq!(HistogramQuery::bucket_for(at(0), at(86_400)), 1800);
        assert_eq!(HistogramQuery::bucket_for(at(0), at(30 * 86_400)), 43_200);
        assert_eq!(HistogramQuery::bucket_for(at(0), at(365 * 86_400)), 86_400);
    }
}
