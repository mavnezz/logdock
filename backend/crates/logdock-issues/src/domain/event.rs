//! Error events as Sentry's SDKs send them
//! (<https://develop.sentry.dev/sdk/foundations/envelopes/event-payloads/>),
//! read from the payload into what Logdock groups, lists and shows. The
//! payload itself is kept, redacted, for everything else.

use logdock_kernel::redaction::{RedactionRules, Redactor};
use logdock_kernel::{Timestamp, Value};

/// Sentry's event levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventLevel {
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

impl EventLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "debug" | "trace" => Self::Debug,
            "info" | "log" | "notice" => Self::Info,
            "warning" | "warn" => Self::Warning,
            "error" => Self::Error,
            "fatal" | "critical" => Self::Fatal,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    pub filename: Option<String>,
    pub abs_path: Option<String>,
    pub function: Option<String>,
    pub module: Option<String>,
    pub lineno: Option<i64>,
    pub colno: Option<i64>,
    pub in_app: bool,
    pub context_line: Option<String>,
    pub pre_context: Vec<String>,
    pub post_context: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExceptionInfo {
    /// The class, such as `Illuminate\Database\QueryException`.
    pub kind: String,
    pub value: String,
    pub module: Option<String>,
    pub handled: Option<bool>,
    /// Oldest call first; the last frame is where it was thrown.
    pub frames: Vec<Frame>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Breadcrumb {
    pub time: Option<Timestamp>,
    pub kind: Option<String>,
    pub category: Option<String>,
    pub level: Option<String>,
    pub message: Option<String>,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RequestInfo {
    pub method: Option<String>,
    pub url: Option<String>,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct UserInfo {
    pub id: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErrorEvent {
    /// The SDK's event ID: 32 hexadecimal characters.
    pub event_id: String,
    pub time: Option<Timestamp>,
    pub level: EventLevel,
    pub platform: Option<String>,
    pub release: Option<String>,
    /// The stage the event names; checked against the key's by ingest.
    pub environment: Option<String>,
    /// The container, as sentry-php names it (`gethostname()`).
    pub server_name: Option<String>,
    /// The route or job class.
    pub transaction: Option<String>,
    pub logger: Option<String>,
    pub message: Option<String>,
    pub message_template: Option<String>,
    /// Oldest cause first; the last one is the exception that was thrown.
    pub exceptions: Vec<ExceptionInfo>,
    /// The stack of an event without an exception.
    pub frames: Vec<Frame>,
    pub tags: Vec<(String, String)>,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub request: Option<RequestInfo>,
    pub user: Option<UserInfo>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub fingerprint: Vec<String>,
    pub sdk: Option<String>,
    /// The whole payload, as kept.
    pub payload: Value,
}

/// Request headers that are kept; all others are dropped before anything
/// is stored (docs/architecture.md, 6).
const KEPT_HEADERS: &[&str] = &[
    "user-agent",
    "content-type",
    "accept",
    "accept-language",
    "referer",
    "host",
    "x-request-id",
    "x-forwarded-for",
];

/// How much of an event is kept.
pub const MAX_BREADCRUMBS: usize = 100;
pub const MAX_FRAMES: usize = 250;
const MAX_TEXT: usize = 8 * 1024;

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Int(n) => Some(n.to_string()),
        Value::Float(n) => Some(n.to_string()),
        _ => None,
    }
}

fn time(value: Option<&Value>) -> Option<Timestamp> {
    match value? {
        Value::String(s) => Timestamp::parse_rfc3339(s)
            .or_else(|| s.parse::<f64>().ok().and_then(Timestamp::from_unix_seconds_f64)),
        other => other.as_f64().and_then(Timestamp::from_unix_seconds_f64),
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|v| text(Some(v))).collect())
        .unwrap_or_default()
}

/// A list of objects, or Sentry's `{"values": [...]}` around one.
fn values(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::Array(items)) => items,
        Some(object @ Value::Object(_)) => object.get("values").and_then(Value::as_array).unwrap_or(&[]),
        _ => &[],
    }
}

fn frames(stacktrace: Option<&Value>) -> Vec<Frame> {
    stacktrace
        .and_then(|s| s.get("frames"))
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .map(|frame| Frame {
            filename: text(frame.get("filename")),
            abs_path: text(frame.get("abs_path")),
            function: text(frame.get("function")),
            module: text(frame.get("module")),
            lineno: frame.get("lineno").and_then(Value::as_i64),
            colno: frame.get("colno").and_then(Value::as_i64),
            in_app: frame.get("in_app").and_then(Value::as_bool).unwrap_or(false),
            context_line: text(frame.get("context_line")),
            pre_context: strings(frame.get("pre_context")),
            post_context: strings(frame.get("post_context")),
        })
        .collect()
}

fn pairs(value: Option<&Value>) -> Vec<(String, String)> {
    match value {
        // sentry-php writes headers as PSR-7 has them: {"Host": ["a"]}.
        Some(Value::Object(fields)) => fields
            .iter()
            .filter_map(|(k, v)| text(Some(v)).or_else(|| joined(v)).map(|v| (k.clone(), v)))
            .collect(),
        // Sentry also allows [["key", "value"], …].
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|pair| {
                let pair = pair.as_array()?;
                Some((text(pair.first())?, text(pair.get(1))?))
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A list of texts as one, the way HTTP joins repeated headers.
fn joined(value: &Value) -> Option<String> {
    let Value::Array(items) = value else { return None };
    let parts: Vec<String> = items.iter().filter_map(|item| text(Some(item))).collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

impl ErrorEvent {
    /// Prepares a payload to be kept: the request's body and cookies are
    /// dropped, its headers limited to a few, everything redacted by the
    /// rules, breadcrumbs and frames limited. Returns how many values
    /// redaction replaced.
    pub fn prepare(payload: Value, rules: &RedactionRules) -> (Value, u32) {
        let Value::Object(mut fields) = payload else { return (Value::Object(Vec::new()), 0) };
        for (key, value) in &mut fields {
            match key.as_str() {
                "request" => {
                    if let Value::Object(request) = value {
                        request.retain(|(k, _)| !matches!(k.as_str(), "data" | "cookies" | "env"));
                        for (k, v) in request.iter_mut() {
                            if k == "headers" {
                                *v = keep_headers(std::mem::replace(v, Value::Null));
                            }
                        }
                    }
                }
                "breadcrumbs" => limit_breadcrumbs(value),
                "exception" => {
                    if let Value::Object(exception) = value {
                        for (k, v) in exception.iter_mut() {
                            if k == "values"
                                && let Value::Array(items) = v
                            {
                                for item in items.iter_mut() {
                                    limit_frames(item);
                                }
                            }
                        }
                    }
                }
                "stacktrace" => limit_frames_of(value),
                _ => {}
            }
        }
        let mut redactor = Redactor::new(rules);
        let redacted = redactor.value(Value::Object(fields));
        (redacted, redactor.count())
    }

    /// Reads a prepared payload. `event_id` is the envelope's, for payloads
    /// without one of their own.
    pub fn from_payload(payload: Value, event_id: Option<&str>) -> Self {
        let p = &payload;
        let event_id = text(p.get("event_id"))
            .or_else(|| event_id.map(str::to_owned))
            .unwrap_or_default()
            .replace('-', "")
            .to_ascii_lowercase();
        let (message, message_template) = match p.get("message").or(p.get("logentry")) {
            Some(Value::String(message)) => (Some(message.clone()), None),
            Some(object @ Value::Object(_)) => (
                text(object.get("formatted")).or_else(|| text(object.get("message"))),
                text(object.get("message")),
            ),
            _ => (None, None),
        };
        let exceptions = values(p.get("exception"))
            .iter()
            .map(|exception| ExceptionInfo {
                kind: text(exception.get("type")).unwrap_or_else(|| "Error".to_owned()),
                value: text(exception.get("value")).unwrap_or_default(),
                module: text(exception.get("module")),
                handled: exception.pointer(&["mechanism", "handled"]).and_then(Value::as_bool),
                frames: frames(exception.get("stacktrace")),
            })
            .collect();
        let breadcrumbs = values(p.get("breadcrumbs"))
            .iter()
            .map(|crumb| Breadcrumb {
                time: time(crumb.get("timestamp")),
                kind: text(crumb.get("type")),
                category: text(crumb.get("category")),
                level: text(crumb.get("level")),
                message: text(crumb.get("message")),
                data: crumb.get("data").cloned().unwrap_or(Value::Null),
            })
            .collect();
        let request = p.get("request").map(|request| RequestInfo {
            method: text(request.get("method")),
            url: text(request.get("url")),
            query: text(request.get("query_string")),
            headers: pairs(request.get("headers")),
        });
        let user =
            p.get("user").filter(|u| u.as_object().is_some_and(|f| !f.is_empty())).map(|user| UserInfo {
                id: text(user.get("id")),
                email: text(user.get("email")),
                username: text(user.get("username")),
                ip_address: text(user.get("ip_address")),
            });
        let sdk = p.get("sdk").and_then(|sdk| {
            let name = text(sdk.get("name"))?;
            Some(match text(sdk.get("version")) {
                Some(version) => format!("{name} {version}"),
                None => name,
            })
        });
        Self {
            event_id,
            time: time(p.get("timestamp")),
            level: text(p.get("level")).and_then(|l| EventLevel::parse(&l)).unwrap_or(EventLevel::Error),
            platform: text(p.get("platform")),
            release: text(p.get("release")),
            environment: text(p.get("environment")),
            server_name: text(p.get("server_name")),
            transaction: text(p.get("transaction")),
            logger: text(p.get("logger")),
            message: message.map(|m| cut(m, MAX_TEXT)),
            message_template,
            exceptions,
            frames: frames(p.get("stacktrace")),
            tags: pairs(p.get("tags")),
            breadcrumbs,
            request,
            user,
            trace_id: text(p.pointer(&["contexts", "trace", "trace_id"])),
            span_id: text(p.pointer(&["contexts", "trace", "span_id"])),
            fingerprint: strings(p.get("fingerprint")),
            sdk,
            payload,
        }
    }

    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// The exception that was thrown: the last of the chain.
    pub fn primary(&self) -> Option<&ExceptionInfo> {
        self.exceptions.last()
    }
}

fn cut(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    text
}

fn keep_headers(headers: Value) -> Value {
    let keep = |name: &str| KEPT_HEADERS.contains(&name.to_ascii_lowercase().as_str());
    match headers {
        Value::Object(fields) => Value::Object(fields.into_iter().filter(|(k, _)| keep(k)).collect()),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .filter(|pair| {
                    pair.as_array().and_then(|p| p.first()).and_then(Value::as_str).is_some_and(keep)
                })
                .collect(),
        ),
        _ => Value::Null,
    }
}

fn limit_breadcrumbs(breadcrumbs: &mut Value) {
    let list = match breadcrumbs {
        Value::Array(items) => items,
        Value::Object(fields) => match fields.iter_mut().find(|(k, _)| k == "values") {
            Some((_, Value::Array(items))) => items,
            _ => return,
        },
        _ => return,
    };
    if list.len() > MAX_BREADCRUMBS {
        // The newest ones tell what led to the error.
        list.drain(..list.len() - MAX_BREADCRUMBS);
    }
}

fn limit_frames_of(stacktrace: &mut Value) {
    if let Value::Object(fields) = stacktrace
        && let Some((_, Value::Array(frames))) = fields.iter_mut().find(|(k, _)| k == "frames")
        && frames.len() > MAX_FRAMES
    {
        // Keep where it started and, above all, where it was thrown.
        let keep_start = 50;
        let drop = frames.len() - MAX_FRAMES;
        frames.drain(keep_start..keep_start + drop);
    }
}

fn limit_frames(exception: &mut Value) {
    if let Value::Object(fields) = exception
        && let Some((_, stacktrace)) = fields.iter_mut().find(|(k, _)| k == "stacktrace")
    {
        limit_frames_of(stacktrace);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn object(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    pub(crate) fn s(text: &str) -> Value {
        Value::String(text.to_owned())
    }

    pub(crate) fn frame(file: &str, function: &str, line: i64, in_app: bool) -> Value {
        object(vec![
            ("filename", s(file)),
            ("function", s(function)),
            ("lineno", Value::Int(line)),
            ("in_app", Value::Bool(in_app)),
            ("context_line", s("throw new RuntimeException();")),
            ("pre_context", Value::Array(vec![s("// before")])),
        ])
    }

    /// An event as sentry-php 4.x serializes one (EventItem.php).
    pub(crate) fn payload() -> Value {
        object(vec![
            ("timestamp", Value::Float(1_794_993_302.5)),
            ("platform", s("php")),
            ("sdk", object(vec![("name", s("sentry.php.laravel")), ("version", s("4.30.0"))])),
            ("level", s("error")),
            ("transaction", s("POST /orders")),
            ("server_name", s("orders-web-1")),
            ("release", s("4f2c1e9")),
            ("environment", s("production")),
            ("tags", object(vec![("service", s("web")), ("host", s("docker-host-02"))])),
            ("user", object(vec![("id", Value::Int(7)), ("email", s("ada@example.com"))])),
            (
                "contexts",
                object(vec![(
                    "trace",
                    object(vec![("trace_id", s("5b8efff798038103d269b633813fc60c")), ("span_id", s("a1b2"))]),
                )]),
            ),
            (
                "breadcrumbs",
                object(vec![(
                    "values",
                    Value::Array(vec![object(vec![
                        ("type", s("default")),
                        ("category", s("db.sql.query")),
                        ("level", s("info")),
                        ("timestamp", Value::Float(1_794_993_302.1)),
                        ("message", s("select * from orders where id = ?")),
                    ])]),
                )]),
            ),
            (
                "request",
                object(vec![
                    ("method", s("POST")),
                    ("url", s("https://shop.example.com/orders")),
                    ("query_string", s("token=abc")),
                    ("data", object(vec![("card", s("4111 1111 1111 1111"))])),
                    ("cookies", object(vec![("session", s("x"))])),
                    (
                        "headers",
                        object(vec![
                            ("User-Agent", s("curl")),
                            ("Authorization", s("Bearer abcdefgh12345678")),
                            ("Cookie", s("a=b")),
                        ]),
                    ),
                ]),
            ),
            (
                "exception",
                object(vec![(
                    "values",
                    Value::Array(vec![
                        object(vec![
                            ("type", s("PDOException")),
                            ("value", s("SQLSTATE[40001]: Deadlock found")),
                            (
                                "stacktrace",
                                object(vec![(
                                    "frames",
                                    Value::Array(vec![frame(
                                        "/vendor/laravel/Connection.php",
                                        "runQuery",
                                        10,
                                        false,
                                    )]),
                                )]),
                            ),
                        ]),
                        object(vec![
                            ("type", s("Illuminate\\Database\\QueryException")),
                            (
                                "value",
                                s("SQLSTATE[40001]: Deadlock found when trying to get lock (order 42)"),
                            ),
                            (
                                "mechanism",
                                object(vec![("type", s("generic")), ("handled", Value::Bool(false))]),
                            ),
                            (
                                "stacktrace",
                                object(vec![(
                                    "frames",
                                    Value::Array(vec![
                                        frame("/public/index.php", "{main}", 20, false),
                                        frame(
                                            "/app/Http/Controllers/OrderController.php",
                                            "OrderController::store",
                                            31,
                                            true,
                                        ),
                                        frame("/app/Services/Payment.php", "Payment::charge", 88, true),
                                        frame("/vendor/laravel/Connection.php", "runQuery", 120, false),
                                    ]),
                                )]),
                            ),
                        ]),
                    ]),
                )]),
            ),
        ])
    }

    #[test]
    fn reads_what_sentry_php_sends() {
        let (prepared, redactions) = ErrorEvent::prepare(payload(), &RedactionRules::default());
        let event = ErrorEvent::from_payload(prepared, Some("9EC79C33-EC99-42AB-8353-589FCB2E04DC"));
        assert_eq!(event.event_id, "9ec79c33ec9942ab8353589fcb2e04dc");
        assert_eq!(event.level, EventLevel::Error);
        assert_eq!(event.time, Some(Timestamp::from_unix_micros(1_794_993_302_500_000)));
        assert_eq!(event.release.as_deref(), Some("4f2c1e9"));
        assert_eq!(event.environment.as_deref(), Some("production"));
        assert_eq!(event.server_name.as_deref(), Some("orders-web-1"));
        assert_eq!(event.tag("service"), Some("web"));
        assert_eq!(event.trace_id.as_deref(), Some("5b8efff798038103d269b633813fc60c"));
        assert_eq!(event.sdk.as_deref(), Some("sentry.php.laravel 4.30.0"));
        assert_eq!(event.exceptions.len(), 2);
        let primary = event.primary().unwrap();
        assert_eq!(primary.kind, "Illuminate\\Database\\QueryException");
        assert_eq!(primary.handled, Some(false));
        assert_eq!(primary.frames.len(), 4);
        assert!(primary.frames[1].in_app);
        assert_eq!(primary.frames[1].pre_context, ["// before"]);
        assert_eq!(event.breadcrumbs[0].category.as_deref(), Some("db.sql.query"));
        assert_eq!(event.user.as_ref().and_then(|u| u.id.as_deref()), Some("7"));

        let request = event.request.as_ref().unwrap();
        assert_eq!(request.headers, [("User-Agent".to_owned(), "curl".to_owned())], "only kept headers");
        assert_eq!(request.query.as_deref(), Some("token=[redacted]"));
        assert!(event.payload.pointer(&["request", "data"]).is_none(), "no bodies");
        assert!(event.payload.pointer(&["request", "cookies"]).is_none());
        assert!(redactions >= 1);
    }

    #[test]
    fn reads_message_events_and_odd_shapes() {
        let payload = object(vec![
            ("event_id", s("abc")),
            ("message", object(vec![("message", s("Order %s failed")), ("formatted", s("Order 42 failed"))])),
            ("level", s("warning")),
            ("tags", Value::Array(vec![Value::Array(vec![s("service"), s("horizon")])])),
            (
                "stacktrace",
                object(vec![("frames", Value::Array(vec![frame("/app/Jobs/Sync.php", "handle", 5, true)]))]),
            ),
            ("timestamp", s("2026-11-18T09:15:02Z")),
            ("breadcrumbs", Value::Array(vec![object(vec![("message", s("x"))])])),
            ("fingerprint", Value::Array(vec![s("{{ default }}"), s("tenant-3")])),
        ]);
        let event = ErrorEvent::from_payload(payload, None);
        assert_eq!(event.event_id, "abc");
        assert_eq!(event.message.as_deref(), Some("Order 42 failed"));
        assert_eq!(event.message_template.as_deref(), Some("Order %s failed"));
        assert_eq!(event.level, EventLevel::Warning);
        assert_eq!(event.tag("service"), Some("horizon"));
        assert_eq!(event.frames.len(), 1);
        assert_eq!(event.breadcrumbs.len(), 1);
        assert_eq!(event.fingerprint, ["{{ default }}", "tenant-3"]);
        assert!(event.time.is_some());
        let empty = ErrorEvent::from_payload(Value::Null, None);
        assert_eq!(empty.level, EventLevel::Error);
        assert!(empty.exceptions.is_empty());
        assert_eq!(
            ErrorEvent::prepare(Value::Int(1), &RedactionRules::default()).0,
            Value::Object(Vec::new())
        );
        for level in
            [EventLevel::Debug, EventLevel::Info, EventLevel::Warning, EventLevel::Error, EventLevel::Fatal]
        {
            assert_eq!(EventLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(EventLevel::parse("loud"), None);
    }

    #[test]
    fn limits_breadcrumbs_and_frames() {
        let crumbs: Vec<Value> = (0..150).map(|i| object(vec![("message", s(&i.to_string()))])).collect();
        let frames: Vec<Value> = (0..400).map(|i| frame("/app/x.php", &format!("f{i}"), i, true)).collect();
        let payload = object(vec![
            ("breadcrumbs", object(vec![("values", Value::Array(crumbs))])),
            ("stacktrace", object(vec![("frames", Value::Array(frames))])),
        ]);
        let (prepared, _) = ErrorEvent::prepare(payload, &RedactionRules::default());
        let event = ErrorEvent::from_payload(prepared, None);
        assert_eq!(event.breadcrumbs.len(), MAX_BREADCRUMBS);
        assert_eq!(event.breadcrumbs[0].message.as_deref(), Some("50"), "the newest are kept");
        assert_eq!(event.frames.len(), MAX_FRAMES);
        assert_eq!(
            event.frames.last().and_then(|f| f.function.as_deref()),
            Some("f399"),
            "where it was thrown is kept"
        );
        assert_eq!(event.frames[49].function.as_deref(), Some("f49"));
    }

    #[test]
    fn reads_headers_as_psr7_writes_them() {
        let headers = object(vec![
            ("Accept", Value::Array(vec![s("text/html"), s("application/json")])),
            ("Cookie", Value::Array(vec![s("a=b")])),
        ]);
        let payload = object(vec![("request", object(vec![("headers", headers)]))]);
        let (prepared, _) = ErrorEvent::prepare(payload, &RedactionRules::default());
        let request = ErrorEvent::from_payload(prepared, None).request.unwrap();
        assert_eq!(request.headers, [("Accept".to_owned(), "text/html, application/json".to_owned())]);
    }
}
