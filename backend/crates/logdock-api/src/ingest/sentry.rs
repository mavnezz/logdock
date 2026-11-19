//! Sentry's `log` items as the Logs context's entries.

use logdock_logs::domain::{Level, NewLogEntry};
use logdock_sentry::{SentryLog, Timestamp as SentryTime};

use super::value::{text, value};

/// Attributes that become fields of their own, or that only the SDK needs.
const LIFTED: &[&str] = &[
    "sentry.release",
    "sentry.environment",
    "server.address",
    "service",
    "host",
    "user.id",
    "sentry.message.template",
    "sentry.trace.parent_span_id",
    "sentry.sdk.name",
    "sentry.sdk.version",
    "sentry.origin",
];

/// The entry, and the stage it says it comes from.
pub(crate) fn entry(log: SentryLog) -> (NewLogEntry, Option<String>) {
    let attribute = |key: &str| log.attribute(key).and_then(text);
    let time = match &log.timestamp {
        Some(SentryTime::Seconds(seconds)) => logdock_kernel::Timestamp::from_unix_seconds_f64(*seconds),
        Some(SentryTime::Text(text)) => logdock_kernel::Timestamp::parse_rfc3339(text),
        None => None,
    };
    let level = Level::parse(&log.level).or_else(|| log.severity_number.and_then(Level::from_severity));
    let environment = attribute("sentry.environment");
    let entry = NewLogEntry {
        time,
        level,
        template: attribute("sentry.message.template"),
        trace_id: log.trace_id.clone().filter(|t| !t.is_empty() && t.chars().any(|c| c != '0')),
        // sentry-php writes a missing parent span as the text "null".
        span_id: log
            .span_id
            .clone()
            .or_else(|| attribute("sentry.trace.parent_span_id"))
            .filter(|s| !s.is_empty() && s != "null"),
        service: attribute("service"),
        host: attribute("host"),
        container: attribute("server.address"),
        release: attribute("sentry.release"),
        user_id: attribute("user.id"),
        attributes: log
            .attributes
            .iter()
            .filter(|(key, _)| {
                !LIFTED.contains(&key.as_str()) && !key.starts_with("sentry.message.parameter.")
            })
            .map(|(key, attribute)| (key.clone(), value(attribute.value.clone())))
            .collect(),
        message: log.body,
    };
    (entry, environment)
}

#[cfg(test)]
mod tests {
    use logdock_sentry::LogItems;

    use super::*;

    #[test]
    fn lifts_what_has_a_field_of_its_own() {
        let payload = br#"{"items":[{"timestamp":1794993302.5,"trace_id":"5b8efff798038103d269b633813fc60c","level":"warn",
            "body":"Payment declined for order 42","attributes":{
              "sentry.release":{"type":"string","value":"4f2c1e9"},
              "sentry.environment":{"type":"string","value":"production"},
              "server.address":{"type":"string","value":"orders-horizon-1"},
              "service":{"type":"string","value":"horizon"},
              "host":{"type":"string","value":"docker-host-02"},
              "sentry.message.template":{"type":"string","value":"Payment declined for order {id}"},
              "sentry.message.parameter.id":{"type":"integer","value":42},
              "sentry.sdk.name":{"type":"string","value":"sentry.php"},
              "order_id":{"type":"integer","value":42},
              "channel":{"type":"string","value":"production"}}}]}"#;
        let (logs, _) = LogItems::parse(payload).unwrap();
        let (entry, environment) = entry(logs.items.into_iter().next().unwrap());
        assert_eq!(environment.as_deref(), Some("production"));
        assert_eq!(entry.level, Some(Level::Warn));
        assert_eq!(entry.time, Some(logdock_kernel::Timestamp::from_unix_micros(1_794_993_302_500_000)));
        assert_eq!(entry.release.as_deref(), Some("4f2c1e9"));
        assert_eq!(entry.container.as_deref(), Some("orders-horizon-1"));
        assert_eq!(entry.service.as_deref(), Some("horizon"));
        assert_eq!(entry.host.as_deref(), Some("docker-host-02"));
        assert_eq!(entry.template.as_deref(), Some("Payment declined for order {id}"));
        let keys: Vec<&str> = entry.attributes.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["order_id", "channel"]);
    }

    #[test]
    fn takes_text_times_severities_and_drops_empty_traces() {
        let payload = br#"{"items":[{"timestamp":"2026-11-18T09:15:02Z","trace_id":"00000000000000000000000000000000","level":"loud","severity_number":17,"body":"x"}]}"#;
        let (logs, _) = LogItems::parse(payload).unwrap();
        let (entry, environment) = entry(logs.items.into_iter().next().unwrap());
        assert_eq!(entry.level, Some(Level::Error));
        assert_eq!(entry.trace_id, None);
        assert!(entry.time.is_some());
        assert_eq!(environment, None);
    }
}
