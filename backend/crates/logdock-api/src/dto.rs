//! Shapes shared by several routes.

use logdock_kernel::Timestamp;

/// A point in time as the API writes it: RFC 3339 in UTC with
/// microseconds, e.g. `2026-11-18T09:15:02.123456Z`.
pub(crate) fn time(at: Timestamp) -> String {
    at.utc().to_string()
}

pub(crate) fn optional_time(at: Option<Timestamp>) -> Option<String> {
    at.map(time)
}

/// A value of the contexts as JSON.
pub(crate) fn json(value: &logdock_kernel::Value) -> serde_json::Value {
    use logdock_kernel::Value;
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(flag) => serde_json::Value::Bool(*flag),
        Value::Int(number) => serde_json::Value::from(*number),
        Value::Float(number) => {
            serde_json::Number::from_f64(*number).map_or(serde_json::Value::Null, serde_json::Value::Number)
        }
        Value::String(text) => serde_json::Value::String(text.clone()),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(json).collect()),
        Value::Object(fields) => {
            serde_json::Value::Object(fields.iter().map(|(k, v)| (k.clone(), json(v))).collect())
        }
    }
}
