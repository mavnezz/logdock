//! Which issue an event belongs to (docs/architecture.md, 4.2). The rules
//! follow the order Sentry's documentation describes: the SDK's
//! fingerprint, then the stack trace's in-app frames, then the exception,
//! then the message. Numbers and IDs in texts are replaced first, so an
//! error about order 41 and one about order 42 are one issue.

use super::ErrorEvent;

/// What groups an event, and how its issue is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grouping {
    /// 32 hexadecimal characters.
    pub hash: String,
    pub title: String,
    /// Where it happened: the innermost in-app frame, or the transaction.
    pub culprit: Option<String>,
}

const TITLE_CHARS: usize = 200;

pub fn fingerprint(event: &ErrorEvent) -> Grouping {
    let default = default_components(event);
    let components: Vec<String> = if event.fingerprint.is_empty() {
        default
    } else {
        event
            .fingerprint
            .iter()
            .flat_map(|part| {
                if part == "{{ default }}" || part == "{{default}}" {
                    default.clone()
                } else {
                    vec![part.clone()]
                }
            })
            .collect()
    };
    Grouping { hash: hash(&components.join("\u{1f}")), title: title(event), culprit: culprit(event) }
}

fn default_components(event: &ErrorEvent) -> Vec<String> {
    let in_app: Vec<_> = event.exceptions.iter().flat_map(|e| e.frames.iter().filter(|f| f.in_app)).collect();
    if !in_app.is_empty() {
        // Exception types and in-app frames: file and function, not lines,
        // so an unrelated edit does not split the issue.
        let mut parts = Vec::new();
        for exception in &event.exceptions {
            parts.push(exception.kind.clone());
            for frame in exception.frames.iter().filter(|f| f.in_app) {
                parts.push(format!(
                    "{}|{}",
                    normalize_path(frame.filename.as_deref().or(frame.module.as_deref()).unwrap_or("")),
                    frame.function.as_deref().unwrap_or("")
                ));
            }
        }
        return parts;
    }
    if !event.exceptions.is_empty() {
        return event
            .exceptions
            .iter()
            .flat_map(|e| [e.kind.clone(), normalize(e.value.lines().next().unwrap_or(""))])
            .collect();
    }
    let in_app_frames: Vec<String> = event
        .frames
        .iter()
        .filter(|f| f.in_app)
        .map(|f| {
            format!(
                "{}|{}",
                normalize_path(f.filename.as_deref().unwrap_or("")),
                f.function.as_deref().unwrap_or("")
            )
        })
        .collect();
    let message = event.message_template.as_deref().or(event.message.as_deref()).unwrap_or("");
    let mut parts = vec![normalize(message.lines().next().unwrap_or(""))];
    parts.extend(in_app_frames);
    parts
}

fn title(event: &ErrorEvent) -> String {
    let title = match event.primary() {
        Some(exception) => {
            let value = exception.value.lines().next().unwrap_or("");
            let kind = exception.kind.rsplit('\\').next().unwrap_or(&exception.kind);
            if value.is_empty() { exception.kind.clone() } else { format!("{kind}: {value}") }
        }
        None => event.message.as_deref().and_then(|m| m.lines().next()).unwrap_or("(no message)").to_owned(),
    };
    if title.chars().count() > TITLE_CHARS {
        format!("{}…", title.chars().take(TITLE_CHARS).collect::<String>())
    } else {
        title
    }
}

fn culprit(event: &ErrorEvent) -> Option<String> {
    let frames = event.primary().map_or(&event.frames, |e| &e.frames);
    frames
        .iter()
        .rev()
        .find(|f| f.in_app)
        .map(|f| match (&f.function, &f.filename) {
            (Some(function), Some(file)) => format!("{function} in {file}"),
            (Some(function), None) => function.clone(),
            (None, Some(file)) => file.clone(),
            (None, None) => String::new(),
        })
        .filter(|c| !c.is_empty())
        .or_else(|| event.transaction.clone())
}

/// A path without what differs between deployments: a release folder
/// such as `/var/www/releases/42/` or a leading `./`.
fn normalize_path(path: &str) -> String {
    let path = path.trim_start_matches("./");
    for anchor in ["/app/", "/src/", "/routes/", "/database/", "/resources/", "/config/", "/vendor/"] {
        if let Some(at) = path.find(anchor) {
            return path[at..].to_owned();
        }
    }
    path.to_owned()
}

/// A text with numbers, hexadecimal IDs and UUIDs replaced.
pub(crate) fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        let uuid_like = word.len() >= 32 && word.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        let hex_like = word.len() >= 8
            && word.chars().all(|c| c.is_ascii_hexdigit())
            && word.chars().any(|c| c.is_ascii_digit());
        let number = word.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',')
            && word.chars().any(|c| c.is_ascii_digit());
        if uuid_like || hex_like {
            out.push_str("<id>");
        } else if number {
            out.push_str("<n>");
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_alphanumeric() || matches!(c, '-' | '.' | ',' | '_') {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    // Numbers inside words (order42, v2) stay; only free-standing ones go.
    out
}

/// 128 bits of two FNV-1a hashes with different bases, as hex. Not
/// cryptographic: it names groups, it guards nothing.
fn hash(text: &str) -> String {
    let fnv = |basis: u64| {
        text.bytes().fold(basis, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3))
    };
    format!("{:016x}{:016x}", fnv(0xcbf2_9ce4_8422_2325), fnv(0x6c62_272e_07bb_0142))
}

#[cfg(test)]
mod tests {
    use logdock_kernel::Value;
    use logdock_kernel::redaction::RedactionRules;

    use super::*;
    use crate::domain::event::tests::{frame, object, payload, s};

    fn event(payload: Value) -> ErrorEvent {
        ErrorEvent::from_payload(ErrorEvent::prepare(payload, &RedactionRules::default()).0, Some("e"))
    }

    fn with(mut payload: Value, key: &str, value: Value) -> Value {
        if let Value::Object(fields) = &mut payload {
            fields.retain(|(k, _)| k != key);
            fields.push((key.to_owned(), value));
        }
        payload
    }

    #[test]
    fn groups_by_exception_types_and_in_app_frames() {
        let first = fingerprint(&event(payload()));
        assert_eq!(first.hash.len(), 32);
        assert_eq!(
            first.title,
            "QueryException: SQLSTATE[40001]: Deadlock found when trying to get lock (order 42)"
        );
        assert_eq!(first.culprit.as_deref(), Some("Payment::charge in /app/Services/Payment.php"));

        // Another order, another line, another release: one issue.
        let other = walk(payload(), &|value| match value {
            Value::String(text) => Value::String(text.replace("order 42", "order 43")),
            Value::Int(88) => Value::Int(90),
            other => other,
        });
        let other = with(other, "release", s("5a1b2c3"));
        assert_eq!(fingerprint(&event(other)).hash, first.hash);

        // Another in-app function: another issue.
        let elsewhere = walk(payload(), &|value| match value {
            Value::String(text) if text == "Payment::charge" => Value::String("Payment::refund".into()),
            other => other,
        });
        assert_ne!(fingerprint(&event(elsewhere)).hash, first.hash);
    }

    fn walk(value: Value, change: &dyn Fn(Value) -> Value) -> Value {
        let value = match value {
            Value::Array(items) => Value::Array(items.into_iter().map(|v| walk(v, change)).collect()),
            Value::Object(fields) => {
                Value::Object(fields.into_iter().map(|(k, v)| (k, walk(v, change))).collect())
            }
            other => other,
        };
        change(value)
    }

    #[test]
    fn falls_back_to_types_and_values_then_to_the_message() {
        let no_app = object(vec![(
            "exception",
            object(vec![(
                "values",
                Value::Array(vec![object(vec![
                    ("type", s("RuntimeException")),
                    ("value", s("Timeout after 30 seconds for job 8f3a9c1d2e")),
                    (
                        "stacktrace",
                        object(vec![("frames", Value::Array(vec![frame("/vendor/x.php", "f", 1, false)]))]),
                    ),
                ])]),
            )]),
        )]);
        let a = fingerprint(&event(no_app.clone()));
        let other_text = object(vec![(
            "exception",
            object(vec![(
                "values",
                Value::Array(vec![object(vec![
                    ("type", s("RuntimeException")),
                    ("value", s("Timeout after 60 seconds for job 0b1c2d3e4f")),
                ])]),
            )]),
        )]);
        assert_eq!(fingerprint(&event(other_text)).hash, a.hash, "numbers and IDs do not split it");
        assert_eq!(a.title, "RuntimeException: Timeout after 30 seconds for job 8f3a9c1d2e");
        assert_eq!(a.culprit, None);

        let message = |text: &str| object(vec![("message", s(text)), ("transaction", s("GET /x"))]);
        let m1 = fingerprint(&event(message("Order 41 could not be paid")));
        let m2 = fingerprint(&event(message("Order 42 could not be paid")));
        assert_eq!(m1.hash, m2.hash);
        assert_eq!(m1.culprit.as_deref(), Some("GET /x"));
        assert_ne!(m1.hash, fingerprint(&event(message("Order 42 was refunded"))).hash);
        assert_eq!(fingerprint(&event(object(vec![]))).title, "(no message)");
    }

    #[test]
    fn the_sdks_fingerprint_comes_first() {
        let custom = with(payload(), "fingerprint", Value::Array(vec![s("payment-provider-down")]));
        let also = with(
            object(vec![("message", s("anything"))]),
            "fingerprint",
            Value::Array(vec![s("payment-provider-down")]),
        );
        assert_eq!(fingerprint(&event(custom)).hash, fingerprint(&event(also)).hash);
        let extended = with(payload(), "fingerprint", Value::Array(vec![s("{{ default }}"), s("tenant-3")]));
        assert_ne!(fingerprint(&event(extended)).hash, fingerprint(&event(payload())).hash);
    }

    #[test]
    fn normalizes_texts_and_paths() {
        assert_eq!(normalize("Order 42 failed after 1.5 s"), "Order <n> failed after <n> s");
        assert_eq!(normalize("user 9ec79c33-ec99-42ab-8353-589fcb2e04dc"), "user <id>");
        assert_eq!(normalize("token deadbeef01 and v2"), "token <id> and v2");
        assert_eq!(normalize_path("/var/www/releases/42/app/Models/Order.php"), "/app/Models/Order.php");
        assert_eq!(normalize_path("./index.php"), "index.php");
        let long = "x".repeat(300);
        let title = fingerprint(&event(object(vec![("message", s(&long))]))).title;
        assert_eq!(title.chars().count(), TITLE_CHARS + 1);
    }
}
