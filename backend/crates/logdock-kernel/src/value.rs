/// Structured data as clients send it: attributes, contexts, tags. The
/// contexts cannot use a JSON library, so adapters convert to and from this.
///
/// Objects keep their order, as clients wrote them.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(number) => Some(*number),
            Self::Float(number) if number.fract() == 0.0 && number.abs() < 9.0e15 => Some(*number as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(number) => Some(*number as f64),
            Self::Float(number) => Some(*number),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Object(fields) => Some(fields),
            _ => None,
        }
    }

    /// The field `key` of an object; the first one if it is there twice.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.iter().find(|(name, _)| name == key).map(|(_, value)| value)
    }

    /// A nested field: `path` names one key per level.
    pub fn pointer(&self, path: &[&str]) -> Option<&Value> {
        path.iter().try_fold(self, |value, key| value.get(key))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// A scalar as text (`true`, `42`, `1.5`, the string itself); `None` for
    /// null, arrays and objects.
    pub fn to_text(&self) -> Option<String> {
        match self {
            Self::Null | Self::Array(_) | Self::Object(_) => None,
            Self::Bool(flag) => Some(flag.to_string()),
            Self::Int(number) => Some(number.to_string()),
            Self::Float(number) => Some(number.to_string()),
            Self::String(text) => Some(text.clone()),
        }
    }

    /// Nested objects as dotted keys (`user.id`), as the log attributes are
    /// stored and searched. Arrays stay values.
    pub fn flatten(fields: Vec<(String, Value)>) -> Vec<(String, Value)> {
        let mut flat = Vec::with_capacity(fields.len());
        flatten_into(&mut flat, String::new(), fields);
        flat
    }

    /// Rough size in bytes, for limits.
    pub fn size(&self) -> usize {
        match self {
            Self::Null | Self::Bool(_) => 4,
            Self::Int(_) | Self::Float(_) => 8,
            Self::String(text) => text.len() + 2,
            Self::Array(items) => 2 + items.iter().map(Self::size).sum::<usize>(),
            Self::Object(fields) => {
                2 + fields.iter().map(|(key, value)| key.len() + 3 + value.size()).sum::<usize>()
            }
        }
    }
}

fn flatten_into(flat: &mut Vec<(String, Value)>, prefix: String, fields: Vec<(String, Value)>) {
    for (key, value) in fields {
        let key = if prefix.is_empty() { key } else { format!("{prefix}.{key}") };
        match value {
            Value::Object(inner) if !inner.is_empty() => flatten_into(flat, key, inner),
            other => flat.push((key, other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(fields: &[(&str, Value)]) -> Value {
        Value::Object(fields.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect())
    }

    #[test]
    fn reads_fields_and_scalars() {
        let value = object(&[
            ("user", object(&[("id", Value::Int(7))])),
            ("name", Value::String("x".into())),
            ("ok", Value::Bool(true)),
            ("ratio", Value::Float(2.0)),
        ]);
        assert_eq!(value.pointer(&["user", "id"]).and_then(Value::as_i64), Some(7));
        assert_eq!(value.get("name").and_then(Value::as_str), Some("x"));
        assert_eq!(value.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(value.get("ratio").and_then(Value::as_i64), Some(2));
        assert_eq!(value.get("ratio").and_then(Value::as_f64), Some(2.0));
        assert!(value.get("missing").is_none());
        assert!(Value::Null.is_null());
        assert_eq!(Value::Array(vec![Value::Null]).as_array().map(<[Value]>::len), Some(1));
    }

    #[test]
    fn prints_scalars_only() {
        assert_eq!(Value::Int(42).to_text().as_deref(), Some("42"));
        assert_eq!(Value::Bool(false).to_text().as_deref(), Some("false"));
        assert_eq!(Value::Float(1.5).to_text().as_deref(), Some("1.5"));
        assert_eq!(Value::Null.to_text(), None);
        assert_eq!(Value::Array(vec![]).to_text(), None);
    }

    #[test]
    fn flattens_nested_objects_into_dotted_keys() {
        let fields = vec![
            ("user".to_owned(), object(&[("id", Value::Int(7)), ("tags", Value::Array(vec![]))])),
            ("empty".to_owned(), Value::Object(vec![])),
            ("order_id".to_owned(), Value::Int(42)),
        ];
        let flat = Value::flatten(fields);
        let keys: Vec<&str> = flat.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["user.id", "user.tags", "empty", "order_id"]);
        assert!(object(&[("a", Value::String("bc".into()))]).size() > 4);
    }
}
