//! The kernel's [`Value`] as JSON, for storage.

use logdock_kernel::Value;
use serde_json::{Map, Number, Value as Json};

pub fn to_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Int(number) => Json::from(*number),
        Value::Float(number) => Number::from_f64(*number).map_or(Json::Null, Json::Number),
        Value::String(text) => Json::String(text.clone()),
        Value::Array(items) => Json::Array(items.iter().map(to_json).collect()),
        Value::Object(fields) => Json::Object(fields.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()),
    }
}

pub fn from_json(json: Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(flag),
        Json::Number(number) => match number.as_i64() {
            Some(int) => Value::Int(int),
            None => number.as_f64().map_or(Value::Null, Value::Float),
        },
        Json::String(text) => Value::String(text),
        Json::Array(items) => Value::Array(items.into_iter().map(from_json).collect()),
        Json::Object(fields) => Value::Object(fields.into_iter().map(|(k, v)| (k, from_json(v))).collect()),
    }
}

/// Attributes as one JSON object, in their order.
pub fn fields_to_json(fields: &[(String, Value)]) -> String {
    let map: Map<String, Json> = fields.iter().map(|(k, v)| (k.clone(), to_json(v))).collect();
    Json::Object(map).to_string()
}

pub fn fields_from_json(text: &str) -> Vec<(String, Value)> {
    match serde_json::from_str::<Json>(text) {
        Ok(Json::Object(map)) => map.into_iter().map(|(k, v)| (k, from_json(v))).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_values_in_their_order() {
        let fields = vec![
            ("z".to_owned(), Value::Int(1)),
            ("a".to_owned(), Value::Float(1.5)),
            ("n".to_owned(), Value::Null),
            ("list".to_owned(), Value::Array(vec![Value::Bool(true), Value::String("x".into())])),
            ("obj".to_owned(), Value::Object(vec![("k".into(), Value::Int(2))])),
        ];
        let json = fields_to_json(&fields);
        assert!(json.starts_with(r#"{"z":1,"a":1.5"#), "{json}");
        assert_eq!(fields_from_json(&json), fields);
        assert!(fields_from_json("[1]").is_empty());
        assert_eq!(to_json(&Value::Float(f64::NAN)), Json::Null);
    }
}
