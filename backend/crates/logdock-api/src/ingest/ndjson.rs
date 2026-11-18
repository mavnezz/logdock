//! Lines of the NDJSON endpoint: Logdock's own shape, or Monolog's
//! `JsonFormatter` (docs/architecture.md, 3.3).

use logdock_logs::domain::{Level, NewLogEntry};
use serde_json::{Map, Value as Json};

use super::value::{text, timestamp, value};

/// The entry, and the stage it says it comes from; `None` if the line is no
/// JSON object with a message.
pub(crate) fn entry(line: &[u8]) -> Option<(NewLogEntry, Option<String>)> {
    let Json::Object(mut map) = serde_json::from_slice::<Json>(line).ok()? else { return None };
    let message = map.remove("message").as_ref().and_then(text)?;
    let time = map.remove("timestamp").or_else(|| map.remove("datetime")).as_ref().and_then(timestamp);
    let level = match (map.remove("level"), map.remove("level_name")) {
        (Some(Json::Number(n)), _) => n.as_i64().and_then(Level::from_monolog),
        (Some(Json::String(name)), _) | (_, Some(Json::String(name))) => Level::parse(&name),
        _ => None,
    };
    // Monolog: context, then what processors added in extra.
    let mut attributes: Map<String, Json> = Map::new();
    for key in ["attributes", "context", "extra"] {
        if let Some(Json::Object(fields)) = map.remove(key) {
            attributes.extend(fields);
        }
    }
    let mut take = |key: &str| map.remove(key).or_else(|| attributes.remove(key)).as_ref().and_then(text);
    let entry = NewLogEntry {
        time,
        level,
        service: take("service"),
        host: take("host"),
        container: take("container"),
        release: take("release"),
        trace_id: take("trace_id"),
        span_id: take("span_id"),
        user_id: take("user_id"),
        template: None,
        message,
        attributes: Vec::new(),
    };
    let environment = take("environment");
    let mut entry = entry;
    if let Some(channel) = map.remove("channel").as_ref().and_then(text) {
        attributes.insert("channel".to_owned(), Json::String(channel));
    }
    entry.attributes = attributes.into_iter().map(|(k, v)| (k, value(v))).collect();
    Some((entry, environment))
}

#[cfg(test)]
mod tests {
    use logdock_kernel::Timestamp;

    use super::*;

    #[test]
    fn reads_logdocks_own_lines() {
        let line = br#"{"timestamp":"2026-11-18T09:15:02.123456+00:00","level":"warning","message":"Payment declined","service":"horizon","host":"docker-host-02","container":"orders-horizon-1","release":"4f2c1e9","trace_id":"abc","environment":"production","attributes":{"order_id":42}}"#;
        let (entry, environment) = entry(line).unwrap();
        assert_eq!(entry.time, Some(Timestamp::from_unix_micros(1_794_993_302_123_456)));
        assert_eq!(entry.level, Some(Level::Warn));
        assert_eq!(
            (entry.service.as_deref(), entry.host.as_deref()),
            (Some("horizon"), Some("docker-host-02"))
        );
        assert_eq!(entry.trace_id.as_deref(), Some("abc"));
        assert_eq!(environment.as_deref(), Some("production"));
        assert_eq!(entry.attributes[0].0, "order_id");
    }

    #[test]
    fn reads_monologs_json_formatter() {
        let line = br#"{"message":"Job failed","context":{"job":"SendMail","exception":{"class":"RuntimeException"}},"level":400,"level_name":"ERROR","channel":"production","datetime":"2026-11-18T09:15:02.000000+01:00","extra":{"service":"horizon","host":"h1"}}"#;
        let (entry, _) = entry(line).unwrap();
        assert_eq!(entry.level, Some(Level::Error));
        assert_eq!(entry.service.as_deref(), Some("horizon"));
        assert_eq!(entry.time, Some(Timestamp::from_unix_seconds(1_794_989_702)));
        let keys: Vec<&str> = entry.attributes.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["job", "exception", "channel"]);
        let by_name = br#"{"message":"x","level_name":"NOTICE"}"#;
        assert_eq!(entry_level(by_name), Some(Level::Info));
    }

    fn entry_level(line: &[u8]) -> Option<Level> {
        entry(line).unwrap().0.level
    }

    #[test]
    fn refuses_lines_without_a_message() {
        assert!(entry(b"not json").is_none());
        assert!(entry(b"[1]").is_none());
        assert!(entry(br#"{"level":"info"}"#).is_none());
        assert!(entry(br#"{"message":""}"#).is_none());
    }
}
