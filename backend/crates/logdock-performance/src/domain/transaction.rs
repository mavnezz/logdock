//! Transactions and their spans as Sentry's SDKs send them, and what they
//! are measured by.

use logdock_kernel::{Timestamp, Value};

/// What is measured: a transaction's kind, or what its spans did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// An HTTP request (`http.server`).
    Request,
    /// A queued job (`queue.process`).
    Job,
    /// An Artisan command or scheduled task (`console.*`).
    Command,
    /// Any other transaction.
    Other,
    /// SQL statements, by their normalized text.
    Query,
    /// Outgoing HTTP requests, by method and host.
    Outgoing,
    /// Cache operations, by operation and key.
    Cache,
    /// How long jobs waited in a queue before they ran, by queue; flagged
    /// when the job was a retry.
    QueueWait,
    /// How long jobs of a queue ran, by queue; flagged when they failed.
    QueueProcess,
    /// Jobs put into a queue (`queue.publish` spans), by queue.
    QueuePublish,
    /// Requests by the release that answered them; flagged when they failed.
    Release,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::Request,
        Kind::Job,
        Kind::Command,
        Kind::Other,
        Kind::Query,
        Kind::Outgoing,
        Kind::Cache,
        Kind::QueueWait,
        Kind::QueueProcess,
        Kind::QueuePublish,
        Kind::Release,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Request => "request",
            Kind::Job => "job",
            Kind::Command => "command",
            Kind::Other => "other",
            Kind::Query => "query",
            Kind::Outgoing => "outgoing",
            Kind::Cache => "cache",
            Kind::QueueWait => "queue_wait",
            Kind::QueueProcess => "queue_process",
            Kind::QueuePublish => "queue_publish",
            Kind::Release => "release",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.as_str() == text)
    }

    /// Whether transactions are of this kind, rather than spans.
    pub fn is_transaction(self) -> bool {
        matches!(self, Kind::Request | Kind::Job | Kind::Command | Kind::Other)
    }

    fn of_operation(op: &str) -> Kind {
        match op {
            "http.server" => Kind::Request,
            "queue.process" | "queue.task" | "queue.job" => Kind::Job,
            op if op.starts_with("console") => Kind::Command,
            _ => Kind::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub op: String,
    pub description: Option<String>,
    pub start: Timestamp,
    pub duration_us: u64,
    pub status: Option<String>,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    /// The route as the framework names it (`/orders/{order}`), the job's
    /// class or the task's name.
    pub name: String,
    pub op: String,
    pub kind: Kind,
    pub status: Option<String>,
    pub http_method: Option<String>,
    pub http_status: Option<i64>,
    pub start: Timestamp,
    pub duration_us: u64,
    pub release: Option<String>,
    pub environment: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
    /// The container, as the SDK names its server.
    pub container: Option<String>,
    /// For a queued job: its queue, how long it waited, how often it ran.
    pub queue: Option<QueueRun>,
    pub spans: Vec<Span>,
}

/// What sentry-laravel says of a queued job's run (`messaging.*`).
#[derive(Debug, Clone, PartialEq)]
pub struct QueueRun {
    /// `messaging.destination.name`: `default`, `emails`, …
    pub name: String,
    /// `messaging.message.receive.latency`: from being queued to running.
    pub wait_us: Option<u64>,
    /// `messaging.message.retry.count`.
    pub retries: u32,
}

/// One thing measured: a transaction, or what one of its spans did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    pub kind: Kind,
    pub name: String,
    pub duration_us: u64,
    /// Failed, or for the cache missed.
    pub flagged: bool,
}

const NAME_CHARS: usize = 300;
const MAX_SPANS: usize = 1000;

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::to_text).filter(|t| !t.is_empty() && t != "null")
}

fn seconds(value: Option<&Value>) -> Option<Timestamp> {
    match value? {
        Value::String(s) => {
            Timestamp::parse_rfc3339(s).or_else(|| s.parse().ok().and_then(Timestamp::from_unix_seconds_f64))
        }
        other => other.as_f64().and_then(Timestamp::from_unix_seconds_f64),
    }
}

/// A tag from `{"service": "web"}` or `[["service", "web"]]`.
fn tag(tags: Option<&Value>, key: &str) -> Option<String> {
    match tags? {
        Value::Object(_) => text(tags?.get(key)),
        Value::Array(pairs) => pairs.iter().find_map(|pair| {
            let pair = pair.as_array()?;
            (pair.first()?.as_str()? == key).then(|| text(pair.get(1))).flatten()
        }),
        _ => None,
    }
}

fn cut(mut text: String) -> String {
    if text.chars().count() > NAME_CHARS {
        text = text.chars().take(NAME_CHARS).collect::<String>() + "…";
    }
    text
}

/// Whether a span's or transaction's status says it went wrong. Sentry's
/// statuses: `ok`, `cancelled`, `not_found`, `internal_error`, …; a missing
/// status is no failure.
fn failed(status: Option<&str>) -> bool {
    matches!(
        status,
        Some(
            "internal_error"
                | "unknown_error"
                | "unknown"
                | "deadline_exceeded"
                | "unavailable"
                | "data_loss"
                | "aborted"
                | "resource_exhausted"
        )
    )
}

impl Transaction {
    /// Reads a `transaction` item's payload; `None` without the times it is
    /// measured by.
    pub fn from_payload(payload: &Value) -> Option<Transaction> {
        let trace = payload.pointer(&["contexts", "trace"]);
        let start = seconds(payload.get("start_timestamp"))?;
        let end = seconds(payload.get("timestamp"))?;
        let op = text(trace.and_then(|t| t.get("op"))).unwrap_or_else(|| "default".to_owned());
        let data = trace.and_then(|t| t.get("data"));
        let tags = payload.get("tags");
        let mut spans: Vec<Span> = payload
            .get("spans")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .take(MAX_SPANS)
            .filter_map(|span| {
                let start = seconds(span.get("start_timestamp"))?;
                let end = seconds(span.get("timestamp")).unwrap_or(start);
                Some(Span {
                    span_id: text(span.get("span_id")).unwrap_or_default(),
                    parent_span_id: text(span.get("parent_span_id")),
                    op: text(span.get("op")).unwrap_or_else(|| "default".to_owned()),
                    description: text(span.get("description")),
                    start,
                    duration_us: end.duration_since(start).as_micros() as u64,
                    status: text(span.get("status")),
                    data: span.get("data").cloned().unwrap_or(Value::Null),
                })
            })
            .collect();
        spans.sort_by_key(|s| s.start);
        Some(Transaction {
            trace_id: text(trace.and_then(|t| t.get("trace_id"))).unwrap_or_default(),
            span_id: text(trace.and_then(|t| t.get("span_id"))).unwrap_or_default(),
            parent_span_id: text(trace.and_then(|t| t.get("parent_span_id"))),
            name: cut(text(payload.get("transaction")).unwrap_or_else(|| "(unnamed)".to_owned())),
            kind: Kind::of_operation(&op),
            op,
            status: text(trace.and_then(|t| t.get("status"))),
            http_method: text(data.and_then(|d| d.get("http.request.method"))),
            http_status: data.and_then(|d| d.get("http.response.status_code")).and_then(Value::as_i64),
            start,
            duration_us: end.duration_since(start).as_micros() as u64,
            release: text(payload.get("release")),
            environment: text(payload.get("environment")),
            service: tag(tags, "service"),
            host: tag(tags, "host"),
            container: text(payload.get("server_name")),
            queue: data.and_then(|d| {
                Some(QueueRun {
                    name: cut(text(d.get("messaging.destination.name"))?),
                    wait_us: d
                        .get("messaging.message.receive.latency")
                        .and_then(Value::as_f64)
                        .filter(|s| s.is_finite() && *s >= 0.0)
                        .map(|s| (s * 1_000_000.0) as u64),
                    retries: d
                        .get("messaging.message.retry.count")
                        .and_then(Value::as_i64)
                        .map_or(0, |n| n.clamp(0, i64::from(u32::MAX)) as u32),
                })
            }),
            spans,
        })
    }

    /// The name a request is counted under: method and route.
    pub fn display_name(&self) -> String {
        match (&self.http_method, self.kind) {
            (Some(method), Kind::Request) => format!("{method} {}", self.name),
            _ => self.name.clone(),
        }
    }

    /// Whether it went wrong: a 5xx answer, or a failed status.
    pub fn failed(&self) -> bool {
        match self.http_status {
            Some(status) => status >= 500,
            None => failed(self.status.as_deref()),
        }
    }

    /// What the rollups count: the transaction itself, and its queries,
    /// outgoing requests and cache operations.
    pub fn measurements(&self) -> Vec<Measurement> {
        let mut all = vec![Measurement {
            kind: self.kind,
            name: self.display_name(),
            duration_us: self.duration_us,
            flagged: self.failed(),
        }];
        if let Some(release) = self.release.as_ref().filter(|_| self.kind == Kind::Request) {
            all.push(Measurement {
                kind: Kind::Release,
                name: cut(release.clone()),
                duration_us: self.duration_us,
                flagged: self.failed(),
            });
        }
        if let Some(queue) = self.queue.as_ref().filter(|_| self.kind == Kind::Job) {
            all.push(Measurement {
                kind: Kind::QueueProcess,
                name: queue.name.clone(),
                duration_us: self.duration_us,
                flagged: self.failed(),
            });
            if let Some(wait_us) = queue.wait_us {
                all.push(Measurement {
                    kind: Kind::QueueWait,
                    name: queue.name.clone(),
                    duration_us: wait_us,
                    flagged: queue.retries > 0,
                });
            }
        }
        all.extend(self.spans.iter().filter_map(measure_span));
        all
    }
}

fn measure_span(span: &Span) -> Option<Measurement> {
    let op = span.op.as_str();
    if op == "queue.publish" {
        let queue = text(span.data.get("messaging.destination.name"))?;
        let flagged = failed(span.status.as_deref());
        return Some(Measurement {
            kind: Kind::QueuePublish,
            name: cut(queue),
            duration_us: span.duration_us,
            flagged,
        });
    }
    let status_failed = failed(span.status.as_deref());
    let (kind, name, flagged) = if op.starts_with("db.sql") || op == "db.query" || op == "db" {
        (Kind::Query, normalize_sql(span.description.as_deref()?), status_failed)
    } else if op == "http.client" {
        let url = text(span.data.get("url")).or_else(|| {
            span.description.as_deref().and_then(|d| d.split_whitespace().nth(1)).map(str::to_owned)
        })?;
        let method = text(span.data.get("http.request.method"))
            .or_else(|| {
                span.description.as_deref().and_then(|d| d.split_whitespace().next()).map(str::to_owned)
            })
            .unwrap_or_else(|| "GET".to_owned());
        let http_status = span.data.get("http.response.status_code").and_then(Value::as_i64);
        (
            Kind::Outgoing,
            format!("{method} {}", host_of(&url)),
            http_status.map_or(status_failed, |s| s >= 500),
        )
    } else {
        let operation = op.strip_prefix("cache.")?;
        let key = match span.data.get("cache.key") {
            Some(Value::Array(keys)) => keys.first().and_then(Value::to_text),
            other => text(other),
        }
        .or_else(|| span.description.clone())
        .unwrap_or_default();
        let missed = operation == "get" && span.data.get("cache.hit").and_then(Value::as_bool) == Some(false);
        (Kind::Cache, format!("{operation} {}", normalize_key(&key)), missed)
    };
    Some(Measurement { kind, name: cut(name), duration_us: span.duration_us, flagged })
}

/// `https://api.stripe.com:443/v1/charges?x=1` → `api.stripe.com:443`.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.rsplit('@').next().unwrap_or(host).to_ascii_lowercase()
}

/// A cache key without what varies: `user:42:cart` → `user:<n>:cart`.
fn normalize_key(key: &str) -> String {
    key.split_inclusive([':', '.', '_', '-', '/'])
        .map(|part| {
            let (word, separator) = match part.char_indices().last() {
                Some((at, c)) if [':', '.', '_', '-', '/'].contains(&c) => (&part[..at], &part[at..]),
                _ => (part, ""),
            };
            let variable = (!word.is_empty() && word.chars().all(|c| c.is_ascii_digit()))
                || (word.len() >= 16 && word.chars().all(|c| c.is_ascii_hexdigit()));
            if variable { format!("<n>{separator}") } else { part.to_owned() }
        })
        .collect()
}

/// A statement without its values, so that one statement is one name:
/// literals become `?`, lists of them `(...)`, whitespace one space.
pub fn normalize_sql(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut previous_word = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                // A string literal; '' inside it is an escaped quote.
                while let Some(next) = chars.next() {
                    if next == '\'' {
                        if chars.peek() == Some(&'\'') {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                out.push('?');
                previous_word = false;
            }
            c if c.is_ascii_digit() && !previous_word => {
                while chars.peek().is_some_and(|n| n.is_ascii_digit() || *n == '.') {
                    chars.next();
                }
                out.push('?');
            }
            c if c.is_whitespace() => {
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
                previous_word = false;
            }
            c => {
                previous_word = c.is_alphanumeric() || c == '_' || c == '"' || c == '`';
                out.push(c);
            }
        }
    }
    collapse_lists(out.trim())
}

/// `in (?, ?, ?)` → `in (...)`; `values (?, ?), (?, ?)` → `values (...)`.
fn collapse_lists(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else { break };
        let inner = &rest[open + 1..open + close];
        let only_values = !inner.is_empty() && inner.split(',').all(|part| part.trim() == "?");
        out.push_str(&rest[..open]);
        if only_values {
            out.push_str("(...)");
            rest = &rest[open + close + 1..];
            // Further groups of values after the first: dropped.
            while let Some(more) = rest.strip_prefix(", (").or_else(|| rest.strip_prefix(",(")) {
                match more.find(')') {
                    Some(end) if more[..end].split(',').all(|p| p.trim() == "?") => rest = &more[end + 1..],
                    _ => break,
                }
            }
        } else {
            out.push_str(&rest[open..=open + close]);
            rest = &rest[open + close + 1..];
        }
    }
    out.push_str(rest);
    out
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

    fn span(op: &str, description: &str, start: f64, end: f64, data: Value) -> Value {
        object(vec![
            ("span_id", s("aaaa")),
            ("parent_span_id", s("root")),
            ("op", s(op)),
            ("description", s(description)),
            ("start_timestamp", Value::Float(start)),
            ("timestamp", Value::Float(end)),
            ("data", data),
        ])
    }

    /// A request as sentry-laravel 4.30 sends it (dev/laravel, /scenario/work).
    #[test]
    fn measures_queues_by_their_jobs_and_what_queued_them() {
        // A job as sentry-laravel 4.30 sends it (fixture transaction-job).
        let job = object(vec![
            ("transaction", s("App\\Jobs\\ChargeOrder")),
            ("start_timestamp", Value::Float(1_795_050_500.0)),
            ("timestamp", Value::Float(1_795_050_500.2)),
            (
                "contexts",
                object(vec![(
                    "trace",
                    object(vec![
                        ("op", s("queue.process")),
                        ("status", s("internal_error")),
                        (
                            "data",
                            object(vec![
                                ("messaging.destination.name", s("payments")),
                                ("messaging.message.receive.latency", Value::Float(0.53)),
                                ("messaging.message.retry.count", Value::Int(2)),
                            ]),
                        ),
                    ]),
                )]),
            ),
        ]);
        let job = Transaction::from_payload(&job).unwrap();
        assert_eq!(job.queue, Some(QueueRun { name: "payments".into(), wait_us: Some(530_000), retries: 2 }));
        let measured = job.measurements();
        let of = |kind| measured.iter().find(|m| m.kind == kind).unwrap();
        assert_eq!(
            (of(Kind::QueueProcess).name.as_str(), of(Kind::QueueProcess).flagged),
            ("payments", true)
        );
        assert_eq!(of(Kind::QueueProcess).duration_us, 200_000);
        assert_eq!(
            (of(Kind::QueueWait).duration_us, of(Kind::QueueWait).flagged),
            (530_000, true),
            "a retry"
        );

        let mut request = request("/checkout", 200, 0.1);
        let publish = span(
            "queue.publish",
            "App\\Jobs\\ChargeOrder",
            1_795_050_499.01,
            1_795_050_499.02,
            object(vec![("messaging.destination.name", s("payments"))]),
        );
        // Added to the spans the request has already.
        if let Value::Object(fields) = &mut request
            && let Some((_, Value::Array(spans))) = fields.iter_mut().find(|(k, _)| k == "spans")
        {
            spans.push(publish);
        }
        let request = Transaction::from_payload(&request).unwrap();
        assert_eq!(request.queue, None, "a request is in no queue");
        let published = request.measurements().into_iter().find(|m| m.kind == Kind::QueuePublish).unwrap();
        assert_eq!(published.name, "payments");
        for kind in [Kind::QueueWait, Kind::QueueProcess, Kind::QueuePublish] {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
            assert!(!kind.is_transaction());
        }
    }

    pub(crate) fn request(name: &str, status: i64, duration: f64) -> Value {
        object(vec![
            ("transaction", s(name)),
            ("start_timestamp", Value::Float(1_795_050_499.0)),
            ("timestamp", Value::Float(1_795_050_499.0 + duration)),
            ("release", s("dev-1")),
            ("environment", s("production")),
            ("server_name", s("orders-web-1")),
            ("tags", object(vec![("service", s("web")), ("host", s("dev-host"))])),
            (
                "contexts",
                object(vec![(
                    "trace",
                    object(vec![
                        ("trace_id", s("414460eb7b5c4bc1913034f6998e4ce8")),
                        ("span_id", s("e433bdbd768d41e5")),
                        ("op", s("http.server")),
                        ("status", s(if status >= 500 { "internal_error" } else { "ok" })),
                        (
                            "data",
                            object(vec![
                                ("http.request.method", s("GET")),
                                ("http.response.status_code", Value::Int(status)),
                            ]),
                        ),
                    ]),
                )]),
            ),
            (
                "spans",
                Value::Array(vec![
                    span(
                        "db.sql.query",
                        "select * from orders where id = 42 and status in ('open', 'paid')",
                        1_795_050_499.01,
                        1_795_050_499.012,
                        object(vec![("db.system", s("mysql"))]),
                    ),
                    span(
                        "cache.get",
                        "user:42:cart",
                        1_795_050_499.02,
                        1_795_050_499.020_5,
                        object(vec![
                            ("cache.key", Value::Array(vec![s("user:42:cart")])),
                            ("cache.hit", Value::Bool(false)),
                        ]),
                    ),
                    span(
                        "http.client",
                        "POST https://api.stripe.com/v1/charges",
                        1_795_050_499.03,
                        1_795_050_499.33,
                        object(vec![
                            ("url", s("https://api.stripe.com/v1/charges")),
                            ("http.request.method", s("POST")),
                            ("http.response.status_code", Value::Int(502)),
                        ]),
                    ),
                    span("view.render", "orders.show", 1_795_050_499.34, 1_795_050_499.35, Value::Null),
                ]),
            ),
        ])
    }

    #[test]
    fn reads_a_request_and_measures_it_and_its_spans() {
        let transaction = Transaction::from_payload(&request("/orders/{order}", 200, 0.5)).unwrap();
        assert_eq!(transaction.kind, Kind::Request);
        assert_eq!(transaction.display_name(), "GET /orders/{order}");
        assert_eq!(transaction.duration_us, 500_000);
        assert_eq!(transaction.service.as_deref(), Some("web"));
        assert_eq!(transaction.container.as_deref(), Some("orders-web-1"));
        assert_eq!(transaction.spans.len(), 4);
        assert!(!transaction.failed());

        let measured = transaction.measurements();
        let names: Vec<(Kind, &str, bool)> =
            measured.iter().map(|m| (m.kind, m.name.as_str(), m.flagged)).collect();
        assert_eq!(
            names,
            [
                (Kind::Request, "GET /orders/{order}", false),
                // The request again, by the release that answered it.
                (Kind::Release, "dev-1", false),
                (Kind::Query, "select * from orders where id = ? and status in (...)", false),
                (Kind::Cache, "get user:<n>:cart", true),
                (Kind::Outgoing, "POST api.stripe.com", true),
            ]
        );
        assert_eq!(measured[1].duration_us, 500_000, "the release counts the whole request");
        assert_eq!(measured[2].duration_us, 2_000);
        assert!(Transaction::from_payload(&request("/x", 503, 0.1)).unwrap().failed());
        assert!(Transaction::from_payload(&object(vec![("transaction", s("x"))])).is_none(), "no times");
    }

    #[test]
    fn knows_jobs_and_commands_by_their_operation() {
        let job = object(vec![
            ("transaction", s("App\\Jobs\\ChargeOrder")),
            ("start_timestamp", Value::Float(10.0)),
            ("timestamp", Value::Float(10.5)),
            ("tags", Value::Array(vec![Value::Array(vec![s("service"), s("horizon")])])),
            (
                "contexts",
                object(vec![(
                    "trace",
                    object(vec![("op", s("queue.process")), ("status", s("internal_error"))]),
                )]),
            ),
        ]);
        let job = Transaction::from_payload(&job).unwrap();
        assert_eq!((job.kind, job.failed(), job.service.as_deref()), (Kind::Job, true, Some("horizon")));
        assert_eq!(job.display_name(), "App\\Jobs\\ChargeOrder");
        let command = object(vec![
            ("transaction", s("heartbeat")),
            ("start_timestamp", s("1970-01-01T00:00:10Z")),
            ("timestamp", s("1970-01-01T00:00:11Z")),
            ("contexts", object(vec![("trace", object(vec![("op", s("console.command.scheduled"))]))])),
        ]);
        let command = Transaction::from_payload(&command).unwrap();
        assert_eq!((command.kind, command.duration_us), (Kind::Command, 1_000_000));
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        assert!(Kind::Job.is_transaction() && !Kind::Query.is_transaction());
    }

    #[test]
    fn normalizes_statements_hosts_and_keys() {
        assert_eq!(
            normalize_sql("insert into \"jobs\" (\"queue\", \"attempts\") values (?, ?), (?, ?)"),
            "insert into \"jobs\" (\"queue\", \"attempts\") values (...)"
        );
        assert_eq!(
            normalize_sql("select  *\n from t1 where a = 'it''s' and b >= 3.5"),
            "select * from t1 where a = ? and b >= ?"
        );
        assert_eq!(
            normalize_sql("select * from users where id in (1, 2, 3) limit 10"),
            "select * from users where id in (...) limit ?"
        );
        assert_eq!(normalize_sql("select count(*) from t"), "select count(*) from t");
        assert_eq!(host_of("http://user:pw@Logdock:8080/health?x"), "logdock:8080");
        assert_eq!(host_of("api.example.com/v1"), "api.example.com");
        assert_eq!(normalize_key("orders.9ec79c33ec9942ab"), "orders.<n>");
        assert_eq!(normalize_key("exchange-rate"), "exchange-rate");
    }
}
