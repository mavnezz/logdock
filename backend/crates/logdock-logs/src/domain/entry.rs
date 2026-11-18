use std::fmt;

use logdock_kernel::{Day, Scope, Timestamp, Value};

use super::Level;

/// Where an entry is kept: the day segment it was written to, and its row
/// there. Written as `<days since the epoch>-<row>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogId {
    pub day: Day,
    pub row: i64,
}

impl LogId {
    pub fn parse(text: &str) -> Option<Self> {
        let (day, row) = text.split_once('-')?;
        Some(Self { day: Day::from_days(day.parse().ok()?), row: row.parse().ok()? })
    }
}

impl fmt::Display for LogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.day.days(), self.row)
    }
}

/// An entry as a client sent it, read from its protocol.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewLogEntry {
    /// When it was written; the time of receipt when the client sent none.
    pub time: Option<Timestamp>,
    pub level: Option<Level>,
    pub message: String,
    pub template: Option<String>,
    pub attributes: Vec<(String, Value)>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    /// web, horizon, scheduler, …
    pub service: Option<String>,
    pub host: Option<String>,
    pub container: Option<String>,
    pub release: Option<String>,
    pub user_id: Option<String>,
}

/// An entry as it is kept: limited, redacted, under its key's scope.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// Set once written.
    pub id: Option<LogId>,
    pub scope: Scope,
    pub time: Timestamp,
    pub received: Timestamp,
    pub level: Level,
    pub message: String,
    pub template: Option<String>,
    pub attributes: Vec<(String, Value)>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
    pub container: Option<String>,
    pub release: Option<String>,
    pub user_id: Option<String>,
    /// How many values redaction replaced or dropped.
    pub redactions: u32,
    /// Whether limits cut something short.
    pub truncated: bool,
}

/// How much one entry may hold (docs/architecture.md, 3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogLimits {
    pub message_bytes: usize,
    pub attributes: usize,
    pub value_bytes: usize,
    /// Service, host, container, release, IDs.
    pub label_bytes: usize,
    /// How far in the future an entry's time may be; later ones get the
    /// time of receipt.
    pub max_clock_skew_seconds: i64,
}

impl Default for LogLimits {
    fn default() -> Self {
        Self {
            message_bytes: 16 * 1024,
            attributes: 100,
            value_bytes: 4 * 1024,
            label_bytes: 200,
            max_clock_skew_seconds: 3600,
        }
    }
}

/// `text` cut to at most `max` bytes on a character boundary, with `…`.
pub(crate) fn cut(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push('…');
    true
}

impl LogLimits {
    /// Applies the limits to an entry's parts; returns whether anything was
    /// cut short or dropped.
    pub fn apply(&self, entry: &mut LogEntry) -> bool {
        let mut truncated = cut(&mut entry.message, self.message_bytes);
        if let Some(template) = &mut entry.template {
            truncated |= cut(template, self.message_bytes);
        }
        for label in [
            &mut entry.trace_id,
            &mut entry.span_id,
            &mut entry.service,
            &mut entry.host,
            &mut entry.container,
            &mut entry.release,
            &mut entry.user_id,
        ]
        .into_iter()
        .flatten()
        {
            truncated |= cut(label, self.label_bytes);
        }
        if entry.attributes.len() > self.attributes {
            entry.attributes.truncate(self.attributes);
            truncated = true;
        }
        for (key, value) in &mut entry.attributes {
            truncated |= cut(key, self.label_bytes);
            match value {
                Value::String(text) => truncated |= cut(text, self.value_bytes),
                other if other.size() > self.value_bytes => {
                    *other = Value::String("[too large]".to_owned());
                    truncated = true;
                }
                _ => {}
            }
        }
        truncated
    }
}

#[cfg(test)]
mod tests {
    use logdock_kernel::{EnvironmentId, ProjectId};

    use super::*;

    fn entry() -> LogEntry {
        LogEntry {
            id: None,
            scope: Scope { project: ProjectId::new(1), environment: EnvironmentId::new(1) },
            time: Timestamp::from_unix_seconds(1),
            received: Timestamp::from_unix_seconds(1),
            level: Level::Info,
            message: "hello".into(),
            template: None,
            attributes: Vec::new(),
            trace_id: None,
            span_id: None,
            service: Some("web".into()),
            host: None,
            container: None,
            release: None,
            user_id: None,
            redactions: 0,
            truncated: false,
        }
    }

    #[test]
    fn ids_are_written_and_read() {
        let id = LogId { day: Day::from_days(20_410), row: 17 };
        assert_eq!(id.to_string(), "20410-17");
        assert_eq!(LogId::parse("20410-17"), Some(id));
        for bad in ["", "20410", "x-1", "1-y"] {
            assert_eq!(LogId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn cuts_what_is_too_long() {
        let limits = LogLimits {
            message_bytes: 10,
            attributes: 2,
            value_bytes: 8,
            label_bytes: 4,
            max_clock_skew_seconds: 0,
        };
        let mut entry = entry();
        assert!(!limits.apply(&mut entry));
        entry.message = "ääääääääää".into();
        entry.service = Some("horizon".into());
        entry.attributes = vec![
            ("a".into(), Value::String("0123456789".into())),
            ("b".into(), Value::Array(vec![Value::String("long enough".into())])),
            ("c".into(), Value::Int(1)),
        ];
        assert!(limits.apply(&mut entry));
        assert!(entry.message.len() <= 10 && entry.message.ends_with('…'), "{}", entry.message);
        assert_eq!(entry.service.as_deref(), Some("h…"));
        assert_eq!(entry.attributes.len(), 2);
        assert!(entry.attributes[0].1.as_str().unwrap().ends_with('…'));
        assert_eq!(entry.attributes[1].1.as_str(), Some("[too large]"));
    }
}
