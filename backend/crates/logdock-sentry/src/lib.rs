//! The Sentry ingest protocol as far as Logdock reads it
//! (<https://develop.sentry.dev/sdk/foundations/envelopes/>): envelopes and
//! their items, how SDKs authenticate, and the payloads of the item types
//! Logdock keeps. Nothing here knows Logdock's contexts.

mod auth;
mod envelope;
mod log;

pub use auth::{key_from_auth_header, key_from_dsn, key_from_query};
pub use envelope::{Envelope, EnvelopeError, Item, Limits};
pub use log::{LogAttribute, LogItems, SentryLog};

/// Seconds since the epoch, as Sentry writes timestamps: a number, or an
/// RFC 3339 string.
#[derive(Debug, Clone, PartialEq)]
pub enum Timestamp {
    Seconds(f64),
    Text(String),
}

impl Timestamp {
    fn from_value(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::Number(n) => n.as_f64().map(Self::Seconds),
            serde_json::Value::String(s) => Some(Self::Text(s.clone())),
            _ => None,
        }
    }
}
