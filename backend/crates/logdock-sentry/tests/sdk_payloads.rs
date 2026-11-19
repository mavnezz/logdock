//! What a real SDK sent: envelopes captured from the Laravel test app
//! (tests/fixtures/*/README.md).

use logdock_sentry::{Envelope, Limits, LogItems, key_from_dsn};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sentry-laravel-4.30.0");

fn envelope(name: &str) -> Envelope {
    let bytes = std::fs::read(format!("{DIR}/{name}")).unwrap();
    Envelope::parse(&bytes, Limits::default()).unwrap()
}

#[test]
fn reads_sentry_laravels_log_items() {
    let envelope = envelope("logs.envelope");
    assert!(envelope.event_id().is_none(), "log envelopes carry no event ID");
    assert_eq!(envelope.dsn().and_then(key_from_dsn).map(|k| k.len()), Some(32));
    let [item] = envelope.items.as_slice() else { panic!("one item") };
    assert_eq!(item.kind, "log");
    let (logs, broken) = LogItems::parse(&item.payload).unwrap();
    assert_eq!(broken, 0);
    let [placed, slow] = logs.items.as_slice() else { panic!("two entries") };
    assert_eq!(placed.level, "info");
    assert_eq!(placed.body, "Order 42 placed", "the SDK fills in the placeholders");
    assert_eq!(placed.attribute("service").and_then(|v| v.as_str()), Some("web"));
    assert_eq!(placed.attribute("order").and_then(serde_json::Value::as_i64), Some(42));
    assert_eq!(slow.level, "warn");
    assert!(placed.trace_id.as_deref().is_some_and(|t| t.len() == 32));
}

#[test]
fn reads_sentry_laravels_events() {
    for name in ["event-request.envelope", "event-job.envelope"] {
        let envelope = envelope(name);
        let id = envelope.event_id().unwrap();
        assert_eq!(id.len(), 32, "{name}: the event ID is in the envelope's header only");
        let [item] = envelope.items.as_slice() else { panic!("{name}: one item") };
        assert_eq!(item.kind, "event");
        let event = item.json().unwrap();
        assert!(event.get("event_id").is_none(), "{name}: sentry-php leaves it out of the event");
        assert_eq!(event["platform"], "php");
        assert!(event["exception"]["values"].as_array().is_some_and(|v| !v.is_empty()));
        assert!(event["contexts"]["trace"]["trace_id"].is_string());
    }
}
