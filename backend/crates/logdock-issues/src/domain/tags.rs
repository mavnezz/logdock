//! An event's tags, as Sentry derives them, and searching issues by them
//! (docs/architecture.md, 4.2). Each issue counts the values of its events'
//! tags; the `user` tag's values are the users it affected.

use logdock_kernel::Value;

use super::ErrorEvent;

/// Values kept per issue and tag; further ones are counted for the tag, not
/// kept. Users are kept to a larger number, as they are what is counted.
pub const MAX_TAG_VALUES: u32 = 1_000;
pub const MAX_USER_VALUES: u32 = 10_000;
/// Tags kept per event, and their lengths, as Sentry limits them.
pub const MAX_TAGS: usize = 60;
pub const MAX_KEY_CHARS: usize = 64;
pub const MAX_VALUE_CHARS: usize = 200;

/// The tag whose values are the users.
pub const USER: &str = "user";

/// How many values of `key` an issue keeps.
pub fn values_kept(key: &str) -> u32 {
    if key == USER { MAX_USER_VALUES } else { MAX_TAG_VALUES }
}

/// The order an issue's page shows tags in; others follow by name.
pub const ORDER: &[&str] = &[
    USER,
    "transaction",
    "url",
    "release",
    "browser",
    "client_os",
    "service",
    "host",
    "server_name",
    "handled",
    "mechanism",
    "runtime",
];

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::to_text).map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

fn joined(parts: &[Option<String>]) -> Option<String> {
    let parts: Vec<&str> = parts.iter().flatten().map(String::as_str).collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The tags `event` is found by: the SDK's own, the user, the request's URL,
/// the browser and operating system its user agent names, and what the
/// event says about itself. Each key once.
pub fn tags_of(event: &ErrorEvent) -> Vec<(String, String)> {
    let mut tags: Vec<(String, String)> = Vec::new();
    let mut add = |key: &str, value: Option<String>| {
        let Some(value) = value else { return };
        let key = cut(key.trim(), MAX_KEY_CHARS);
        let value = cut(value.trim(), MAX_VALUE_CHARS);
        if key.is_empty() || value.is_empty() || tags.len() >= MAX_TAGS || tags.iter().any(|(k, _)| *k == key)
        {
            return;
        }
        tags.push((key, value));
    };
    if let Some(user) = &event.user {
        let canonical = [
            ("id", &user.id),
            ("email", &user.email),
            ("username", &user.username),
            ("ip", &user.ip_address),
        ]
        .into_iter()
        .find_map(|(kind, value)| value.as_ref().map(|v| format!("{kind}:{v}")));
        add(USER, canonical);
        add("user.id", user.id.clone());
        add("user.email", user.email.clone());
        add("user.username", user.username.clone());
        add("user.ip", user.ip_address.clone());
    }
    add("transaction", event.transaction.clone());
    if let Some(request) = &event.request {
        add("url", request.url.as_deref().map(|url| url.split(['?', '#']).next().unwrap_or(url).to_owned()));
    }
    add("release", event.release.clone());
    let p = &event.payload;
    let agent = event
        .request
        .as_ref()
        .and_then(|r| r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("user-agent")))
        .map(|(_, v)| UserAgent::parse(v));
    let browser = text(p.pointer(&["contexts", "browser", "name"]))
        .map(|name| (name, text(p.pointer(&["contexts", "browser", "version"]))))
        .or_else(|| agent.as_ref().and_then(|a| a.browser.clone()));
    if let Some((name, version)) = browser {
        let major = version.as_deref().map(|v| v.split('.').next().unwrap_or(v).to_owned());
        add("browser", joined(&[Some(name.clone()), major]));
        add("browser.name", Some(name));
    }
    if let Some((name, version)) = agent.as_ref().and_then(|a| a.os.clone()) {
        add("client_os", joined(&[Some(name.clone()), version]));
        add("client_os.name", Some(name));
    }
    if let Some(device) = agent.as_ref().and_then(|a| a.device) {
        add("device", Some(device.to_owned()));
    }
    for (key, value) in &event.tags {
        add(key, Some(value.clone()));
    }
    add("server_name", event.server_name.clone());
    if let Some(exception) = event.primary() {
        add("handled", exception.handled.map(|h| if h { "yes" } else { "no" }.to_owned()));
        add("mechanism", text(p.pointer(&["exception", "values"]).and_then(last_mechanism)));
    }
    add("logger", event.logger.clone());
    add(
        "runtime",
        joined(&[
            text(p.pointer(&["contexts", "runtime", "name"])),
            text(p.pointer(&["contexts", "runtime", "version"])),
        ]),
    );
    add("level", Some(event.level.as_str().to_owned()));
    tags
}

fn last_mechanism(values: &Value) -> Option<&Value> {
    values.as_array()?.last()?.pointer(&["mechanism", "type"])
}

/// What a user agent says about the browser, the system and the device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserAgent {
    /// Name and version.
    pub browser: Option<(String, Option<String>)>,
    pub os: Option<(String, Option<String>)>,
    /// `Mobile`, `Tablet` or `Desktop`; `Bot` for crawlers.
    pub device: Option<&'static str>,
}

impl UserAgent {
    /// Reads the common browsers, systems and bots; others are left out.
    pub fn parse(agent: &str) -> Self {
        let version_after = |marker: &str| -> Option<String> {
            let rest = &agent[agent.find(marker)? + marker.len()..];
            let version: String =
                rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '_').collect();
            (!version.is_empty()).then(|| version.replace('_', "."))
        };
        let lower = agent.to_ascii_lowercase();
        let bot = ["bot", "crawler", "spider", "curl/", "wget/", "python-requests", "go-http-client"]
            .iter()
            .any(|marker| lower.contains(marker));
        // The order matters: Edge and Opera say Chrome too, Chrome says Safari.
        let browsers: [(&str, &str); 8] = [
            ("Edg/", "Edge"),
            ("OPR/", "Opera"),
            ("SamsungBrowser/", "Samsung Internet"),
            ("Firefox/", "Firefox"),
            ("FxiOS/", "Firefox"),
            ("CriOS/", "Chrome"),
            ("Chrome/", "Chrome"),
            ("Version/", "Safari"),
        ];
        let browser = if bot {
            None
        } else {
            browsers
                .iter()
                .find(|(marker, name)| {
                    agent.contains(marker) && (*name != "Safari" || agent.contains("Safari/"))
                })
                .map(|(marker, name)| ((*name).to_owned(), version_after(marker)))
        };
        let os = if agent.contains("Windows NT") {
            let version = version_after("Windows NT ").map(|v| {
                match v.as_str() {
                    "10.0" => "10",
                    "6.3" => "8.1",
                    "6.2" => "8",
                    "6.1" => "7",
                    other => return other.to_owned(),
                }
                .to_owned()
            });
            Some(("Windows".to_owned(), version))
        } else if agent.contains("iPhone") || agent.contains("iPad") {
            Some(("iOS".to_owned(), version_after("OS ")))
        } else if agent.contains("Mac OS X") {
            Some(("macOS".to_owned(), version_after("Mac OS X ")))
        } else if agent.contains("Android") {
            Some(("Android".to_owned(), version_after("Android ")))
        } else if agent.contains("CrOS") {
            Some(("Chrome OS".to_owned(), None))
        } else if agent.contains("Linux") {
            Some(("Linux".to_owned(), None))
        } else {
            None
        };
        let device = if bot {
            Some("Bot")
        } else if agent.contains("iPad") || agent.contains("Tablet") {
            Some("Tablet")
        } else if agent.contains("Mobile") || agent.contains("iPhone") || agent.contains("Android") {
            Some("Mobile")
        } else if os.is_some() {
            Some("Desktop")
        } else {
            None
        };
        Self { browser, os, device }
    }
}

/// One `key:value` of a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFilter {
    pub key: String,
    pub value: String,
    /// `value*`: the values that start with it.
    pub prefix: bool,
    /// `!key:value`: issues without it.
    pub negated: bool,
}

/// A search of the issue list: words of the title or culprit, and tags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssueSearch {
    pub words: Vec<String>,
    pub tags: Vec<TagFilter>,
}

impl IssueSearch {
    /// Reads `user.email:ada@example.com url:"https://shop/cart" !browser:Safari Timeout`;
    /// double quotes keep spaces in a value, a trailing `*` matches a prefix.
    pub fn parse(text: &str) -> Self {
        let mut search = Self::default();
        for token in tokens(text) {
            match filter(&token) {
                Some(tag) => search.tags.push(tag),
                None => search.words.push(token.replace('"', "")),
            }
        }
        search.words.retain(|w| !w.is_empty());
        search
    }
}

/// Splits at spaces outside double quotes.
fn tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn filter(token: &str) -> Option<TagFilter> {
    let (negated, token) = match token.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    let (key, value) = token.split_once(':')?;
    let key_ok = !key.is_empty()
        && key.len() <= MAX_KEY_CHARS
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    // `https://…` is a word, not the tag `https`.
    if !key_ok || value.is_empty() || value.starts_with("//") {
        return None;
    }
    let quoted = value.len() >= 2 && value.starts_with('"') && value.ends_with('"');
    let mut value = if quoted { &value[1..value.len() - 1] } else { value }.to_owned();
    let prefix = !quoted && value.len() > 1 && value.ends_with('*');
    if prefix {
        value.pop();
    }
    (!value.is_empty()).then(|| TagFilter { key: key.to_owned(), value, prefix, negated })
}

#[cfg(test)]
mod tests {
    use logdock_kernel::Value;

    use super::*;

    fn s(text: &str) -> Value {
        Value::String(text.into())
    }

    fn object(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    const CHROME_WINDOWS: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.6778.86 Safari/537.36";

    fn event() -> ErrorEvent {
        ErrorEvent::from_payload(
            object(vec![
                ("event_id", s("a".repeat(32).as_str())),
                ("release", s("v2")),
                ("transaction", s("/orders/{order}")),
                ("server_name", s("web-1")),
                (
                    "tags",
                    object(vec![("service", s("web")), ("tenant", s("acme")), ("release", s("not this"))]),
                ),
                ("user", object(vec![("id", Value::Int(7)), ("email", s("ada@example.com"))])),
                (
                    "request",
                    object(vec![
                        ("url", s("https://shop.example.com/orders/42?page=2")),
                        ("headers", object(vec![("User-Agent", s(CHROME_WINDOWS))])),
                    ]),
                ),
                (
                    "exception",
                    object(vec![(
                        "values",
                        Value::Array(vec![object(vec![
                            ("type", s("RuntimeException")),
                            (
                                "mechanism",
                                object(vec![("type", s("generic")), ("handled", Value::Bool(false))]),
                            ),
                        ])]),
                    )]),
                ),
                (
                    "contexts",
                    object(vec![("runtime", object(vec![("name", s("php")), ("version", s("8.4.27"))]))]),
                ),
            ]),
            None,
        )
    }

    #[test]
    fn derives_the_tags_sentry_shows() {
        let tags = tags_of(&event());
        let tag = |key: &str| tags.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        assert_eq!(tag("user"), Some("id:7"));
        assert_eq!(tag("user.email"), Some("ada@example.com"));
        assert_eq!(tag("url"), Some("https://shop.example.com/orders/42"));
        assert_eq!(tag("browser"), Some("Chrome 131"));
        assert_eq!(tag("browser.name"), Some("Chrome"));
        assert_eq!(tag("client_os"), Some("Windows 10"));
        assert_eq!(tag("device"), Some("Desktop"));
        assert_eq!(tag("tenant"), Some("acme"));
        assert_eq!(tag("release"), Some("v2"), "the event's release, not a tag of that name");
        assert_eq!(tag("handled"), Some("no"));
        assert_eq!(tag("mechanism"), Some("generic"));
        assert_eq!(tag("runtime"), Some("php 8.4.27"));
        assert_eq!(tag("level"), Some("error"));
        assert_eq!(tag("server_name"), Some("web-1"));
        assert!(tags.len() <= MAX_TAGS);
    }

    #[test]
    fn names_a_user_by_what_the_sdk_sent() {
        let mut event = event();
        event.user =
            Some(super::super::UserInfo { ip_address: Some("10.0.0.1".into()), ..Default::default() });
        let tags = tags_of(&event);
        assert!(tags.contains(&("user".into(), "ip:10.0.0.1".into())));
        event.user = None;
        assert!(!tags_of(&event).iter().any(|(k, _)| k.starts_with("user")));
        assert_eq!(values_kept("user"), MAX_USER_VALUES);
        assert_eq!(values_kept("url"), MAX_TAG_VALUES);
    }

    #[test]
    fn reads_common_user_agents() {
        let read = |agent: &str| {
            let ua = UserAgent::parse(agent);
            (
                ua.browser.map(|(n, v)| format!("{n} {}", v.unwrap_or_default())),
                ua.os.map(|(n, v)| format!("{n} {}", v.unwrap_or_default())),
                ua.device,
            )
        };
        assert_eq!(
            read(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1"
            ),
            (Some("Safari 17.5".into()), Some("iOS 17.5".into()), Some("Mobile"))
        );
        assert_eq!(
            read(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.2903.86"
            ),
            (Some("Edge 131.0.2903.86".into()), Some("macOS 10.15.7".into()), Some("Desktop"))
        );
        assert_eq!(
            read("Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0"),
            (Some("Firefox 133.0".into()), Some("Linux ".into()), Some("Desktop"))
        );
        assert_eq!(
            read("Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0 Mobile Safari/537.36").0,
            Some("Samsung Internet 25.0".into())
        );
        assert_eq!(read("Mozilla/5.0 (iPad; CPU OS 16_0 like Mac OS X)").2, Some("Tablet"));
        assert_eq!(read("Mozilla/5.0 (Windows NT 6.1) OPR/100.0").1, Some("Windows 7".into()));
        assert_eq!(read("Mozilla/5.0 (X11; CrOS x86_64 14541.0.0)").1, Some("Chrome OS ".into()));
        assert_eq!(read("Googlebot/2.1 (+http://www.google.com/bot.html)"), (None, None, Some("Bot")));
        assert_eq!(read("curl/8.5.0"), (None, None, Some("Bot")));
        assert_eq!(read("something"), (None, None, None));
    }

    #[test]
    fn reads_a_search() {
        let search = IssueSearch::parse(
            r#"Timeout  user.email:ada@example.com url:"https://shop/a b" !browser:Safari transaction:/orders* https://x.example SQLSTATE[23000]: :x key:"#,
        );
        assert_eq!(search.words, ["Timeout", "https://x.example", "SQLSTATE[23000]:", ":x", "key:"]);
        assert_eq!(
            search.tags,
            [
                TagFilter {
                    key: "user.email".into(),
                    value: "ada@example.com".into(),
                    prefix: false,
                    negated: false
                },
                TagFilter {
                    key: "url".into(),
                    value: "https://shop/a b".into(),
                    prefix: false,
                    negated: false
                },
                TagFilter { key: "browser".into(), value: "Safari".into(), prefix: false, negated: true },
                TagFilter {
                    key: "transaction".into(),
                    value: "/orders".into(),
                    prefix: true,
                    negated: false
                },
            ]
        );
        assert_eq!(IssueSearch::parse("  "), IssueSearch::default());
        assert_eq!(IssueSearch::parse(r#"a:"""#).words, ["a:"]);
    }
}
