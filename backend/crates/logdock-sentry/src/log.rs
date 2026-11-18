//! `log` items: a batch of structured log entries
//! (<https://develop.sentry.dev/sdk/telemetry/logs/>).

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::Timestamp;

/// One attribute: `{"value": …, "type": "string" | "integer" | "double" |
/// "boolean"}`; a bare value is accepted too.
#[derive(Debug, Clone, PartialEq)]
pub struct LogAttribute {
    pub value: Value,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SentryLog {
    pub timestamp: Option<Timestamp>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub level: String,
    pub body: String,
    pub severity_number: Option<i64>,
    pub attributes: Vec<(String, LogAttribute)>,
}

/// The payload of a `log` item.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LogItems {
    pub items: Vec<SentryLog>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    items: Vec<Value>,
}

impl LogItems {
    /// Reads the payload; entries that are no objects are skipped and
    /// counted in the second value.
    pub fn parse(payload: &[u8]) -> Result<(Self, usize), serde_json::Error> {
        let raw: Raw = serde_json::from_slice(payload)?;
        let mut skipped = 0;
        let items = raw
            .items
            .into_iter()
            .filter_map(|item| {
                let parsed = SentryLog::from_value(item);
                if parsed.is_none() {
                    skipped += 1;
                }
                parsed
            })
            .collect();
        Ok((Self { items }, skipped))
    }
}

impl SentryLog {
    fn from_value(value: Value) -> Option<Self> {
        let Value::Object(mut map) = value else { return None };
        let text = |map: &mut Map<String, Value>, key: &str| match map.remove(key) {
            Some(Value::String(s)) => Some(s),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        let attributes = match map.remove("attributes") {
            Some(Value::Object(attributes)) => attributes
                .into_iter()
                .map(|(key, attribute)| {
                    let attribute = match attribute {
                        Value::Object(mut fields) if fields.contains_key("value") => LogAttribute {
                            value: fields.remove("value").unwrap_or(Value::Null),
                            kind: fields.remove("type").and_then(|t| t.as_str().map(str::to_owned)),
                        },
                        other => LogAttribute { value: other, kind: None },
                    };
                    (key, attribute)
                })
                .collect(),
            _ => Vec::new(),
        };
        Some(Self {
            timestamp: map.get("timestamp").and_then(Timestamp::from_value),
            trace_id: text(&mut map, "trace_id"),
            span_id: text(&mut map, "span_id"),
            level: text(&mut map, "level").unwrap_or_else(|| "info".to_owned()),
            body: text(&mut map, "body").unwrap_or_default(),
            severity_number: map.get("severity_number").and_then(Value::as_i64),
            attributes,
        })
    }

    /// An attribute's value, if there is one of that name.
    pub fn attribute(&self, key: &str) -> Option<&Value> {
        self.attributes.iter().find(|(name, _)| name == key).map(|(_, a)| &a.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_sentry_php_sends() {
        let payload = br#"{"items":[
            {"timestamp":1794993302.123456,"trace_id":"5b8efff798038103d269b633813fc60c","level":"warn",
             "body":"Payment declined","attributes":{
               "sentry.release":{"type":"string","value":"4f2c1e9"},
               "order_id":{"type":"integer","value":42},
               "ratio":{"type":"double","value":0.5},
               "bare":"value"}},
            {"level":"info","body":"no time"},
            "not an object"
        ]}"#;
        let (logs, skipped) = LogItems::parse(payload).unwrap();
        assert_eq!(skipped, 1);
        assert_eq!(logs.items.len(), 2);
        let first = &logs.items[0];
        assert_eq!(first.timestamp, Some(Timestamp::Seconds(1_794_993_302.123456)));
        assert_eq!(first.level, "warn");
        assert_eq!(first.body, "Payment declined");
        assert_eq!(first.attribute("order_id"), Some(&Value::from(42)));
        assert_eq!(first.attribute("bare"), Some(&Value::from("value")));
        assert_eq!(first.attributes[0].1.kind.as_deref(), Some("string"));
        assert_eq!(logs.items[1].timestamp, None);
        assert!(LogItems::parse(b"not json").is_err());
        assert_eq!(LogItems::parse(b"{}").unwrap().0.items.len(), 0);
    }

    #[test]
    fn takes_timestamps_as_text_and_numbers_as_ids() {
        let payload =
            br#"{"items":[{"timestamp":"2026-11-18T09:15:02Z","trace_id":12,"severity_number":13}]}"#;
        let (logs, _) = LogItems::parse(payload).unwrap();
        let log = &logs.items[0];
        assert_eq!(log.timestamp, Some(Timestamp::Text("2026-11-18T09:15:02Z".into())));
        assert_eq!(log.trace_id.as_deref(), Some("12"));
        assert_eq!(log.severity_number, Some(13));
        assert_eq!(log.level, "info");
    }
}
