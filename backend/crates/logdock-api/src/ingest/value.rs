//! JSON as the contexts' [`Value`].

use logdock_kernel::{Timestamp, Value};
use serde_json::Value as Json;

pub(crate) fn value(json: Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(flag),
        Json::Number(number) => match number.as_i64() {
            Some(int) => Value::Int(int),
            None => number.as_f64().map_or(Value::Null, Value::Float),
        },
        Json::String(text) => Value::String(text),
        Json::Array(items) => Value::Array(items.into_iter().map(value).collect()),
        Json::Object(fields) => Value::Object(fields.into_iter().map(|(k, v)| (k, value(v))).collect()),
    }
}

/// A timestamp as Sentry or Monolog write it: seconds as a number, or
/// RFC 3339.
pub(crate) fn timestamp(json: &Json) -> Option<Timestamp> {
    match json {
        Json::Number(n) => n.as_f64().and_then(Timestamp::from_unix_seconds_f64),
        Json::String(s) => Timestamp::parse_rfc3339(s)
            .or_else(|| s.parse::<f64>().ok().and_then(Timestamp::from_unix_seconds_f64)),
        _ => None,
    }
}

/// A short text: a string, or a number written as one.
pub(crate) fn text(json: &Json) -> Option<String> {
    match json {
        Json::String(s) if !s.is_empty() => Some(s.clone()),
        Json::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn converts_json() {
        assert_eq!(
            value(json!({"a": [1, 1.5, true, null, "x"]})),
            Value::Object(vec![(
                "a".into(),
                Value::Array(vec![
                    Value::Int(1),
                    Value::Float(1.5),
                    Value::Bool(true),
                    Value::Null,
                    Value::String("x".into())
                ])
            )])
        );
        assert_eq!(timestamp(&json!(1.5)), Some(Timestamp::from_unix_micros(1_500_000)));
        assert_eq!(timestamp(&json!("1970-01-01T00:00:02Z")), Some(Timestamp::from_unix_seconds(2)));
        assert_eq!(timestamp(&json!("3.5")), Some(Timestamp::from_unix_micros(3_500_000)));
        assert_eq!(timestamp(&json!(null)), None);
        assert_eq!(text(&json!(12)).as_deref(), Some("12"));
        assert_eq!(text(&json!("")), None);
        assert_eq!(text(&json!([])), None);
    }
}
