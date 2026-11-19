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

/// IDs from a comma-separated query parameter.
pub(crate) fn ids(text: &Option<String>) -> Result<Vec<i64>, crate::error::ApiError> {
    text.iter()
        .flat_map(|t| t.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|id| id.parse().map_err(|_| crate::error::ApiError::BadRequest(format!("{id:?} is no ID"))))
        .collect()
}

/// The time now, for ranges a request leaves open.
pub(crate) fn now() -> Timestamp {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    Timestamp::from_unix_micros(since.as_micros() as i64)
}

/// A time from a query parameter, RFC 3339.
pub(crate) fn query_time(
    name: &str,
    text: Option<&str>,
) -> Result<Option<Timestamp>, crate::error::ApiError> {
    match text.filter(|t| !t.is_empty()) {
        Some(text) => Timestamp::parse_rfc3339(text)
            .map(Some)
            .ok_or_else(|| crate::error::ApiError::BadRequest(format!("{name} is no time"))),
        None => Ok(None),
    }
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
