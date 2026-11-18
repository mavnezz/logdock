//! Redaction before anything is stored (docs/architecture.md, 6): values
//! under sensitive keys, and patterns in free text. Shared by every context
//! that stores what clients send.

use crate::Value;

/// Fragments of keys whose values are never stored, compared in lower case
/// without `_`, `-` and `.`: `api_key`, `X-Api-Key` and `apiKey` all match
/// `apikey`.
pub const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "accesskey",
    "authorization",
    "cookie",
    "session",
    "csrf",
    "xsrf",
    "creditcard",
    "cardnumber",
    "cvv",
    "cvc",
    "iban",
    "privatekey",
    "signature",
    "credentials",
    "bearer",
];

const MARK: &str = "[redacted]";

/// A project's rules on top of the built-in ones.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactionRules {
    /// More key fragments, in the form of [`SENSITIVE_KEYS`].
    pub extra_keys: Vec<String>,
    /// When set, only these attribute keys are kept: exact names, or
    /// prefixes ending in `*` (`user.*`). Everything else is dropped.
    pub allowed_keys: Option<Vec<String>>,
    /// Replace e-mail addresses in free text.
    pub scrub_emails: bool,
    /// Replace IPv4 addresses in free text.
    pub scrub_ip_addresses: bool,
}

/// Applies [`RedactionRules`] and counts what it replaced.
#[derive(Debug)]
pub struct Redactor<'a> {
    rules: &'a RedactionRules,
    extra: Vec<String>,
    count: u32,
}

impl<'a> Redactor<'a> {
    pub fn new(rules: &'a RedactionRules) -> Self {
        let extra = rules.extra_keys.iter().map(|key| normalize(key)).filter(|key| !key.is_empty()).collect();
        Self { rules, extra, count: 0 }
    }

    /// How many values were replaced or dropped so far.
    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn is_sensitive(&self, key: &str) -> bool {
        let key = normalize(key);
        !key.is_empty()
            && (SENSITIVE_KEYS.iter().any(|fragment| key.contains(fragment))
                || self.extra.iter().any(|fragment| key.contains(fragment.as_str())))
    }

    fn is_allowed(&self, key: &str) -> bool {
        let Some(allowed) = &self.rules.allowed_keys else { return true };
        allowed.iter().any(|rule| match rule.strip_suffix('*') {
            Some(prefix) => key.starts_with(prefix),
            None => key == rule,
        })
    }

    /// Attributes as stored: flattened keys, allow list applied, sensitive
    /// values replaced, free text in the others scrubbed.
    pub fn attributes(&mut self, fields: Vec<(String, Value)>) -> Vec<(String, Value)> {
        let mut kept = Vec::with_capacity(fields.len());
        for (key, value) in fields {
            if !self.is_allowed(&key) {
                self.count += 1;
                continue;
            }
            let value = self.keyed(&key, value);
            kept.push((key, value));
        }
        kept
    }

    /// A value under `key`: replaced whole when the key is sensitive,
    /// otherwise walked.
    pub fn keyed(&mut self, key: &str, value: Value) -> Value {
        if self.is_sensitive(key) && !matches!(value, Value::Null | Value::Bool(_)) && !is_empty(&value) {
            self.count += 1;
            return Value::String(MARK.to_owned());
        }
        self.value(value)
    }

    /// Walks a value: objects' keys are checked, strings scrubbed.
    pub fn value(&mut self, value: Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.text(&text)),
            Value::Array(items) => Value::Array(items.into_iter().map(|item| self.value(item)).collect()),
            Value::Object(fields) => Value::Object(
                fields.into_iter().map(|(key, value)| (key.clone(), self.keyed(&key, value))).collect(),
            ),
            other => other,
        }
    }

    /// Free text with what looks like a secret replaced.
    pub fn text(&mut self, text: &str) -> String {
        let mut out = text.to_owned();
        let passes: [fn(&mut Self, &str) -> Option<String>; 7] = [
            Self::scrub_url_credentials,
            Self::scrub_bearer,
            Self::scrub_jwt,
            Self::scrub_assignments,
            Self::scrub_cards,
            Self::scrub_emails,
            Self::scrub_ips,
        ];
        for pass in passes {
            if let Some(scrubbed) = pass(self, &out) {
                out = scrubbed;
            }
        }
        out
    }

    /// `https://user:secret@host` → `https://[redacted]@host`.
    fn scrub_url_credentials(&mut self, text: &str) -> Option<String> {
        if !text.contains("://") {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        let mut changed = false;
        while let Some(at) = rest.find("://") {
            let (head, tail) = rest.split_at(at + 3);
            out.push_str(head);
            let authority_end = tail
                .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
                .unwrap_or(tail.len());
            let authority = &tail[..authority_end];
            match authority.rfind('@') {
                Some(at) if authority[..at].contains(':') => {
                    out.push_str(MARK);
                    out.push_str(&authority[at..]);
                    self.count += 1;
                    changed = true;
                }
                _ => out.push_str(authority),
            }
            rest = &tail[authority_end..];
        }
        out.push_str(rest);
        changed.then_some(out)
    }

    /// `Bearer abc…` and `Basic abc…` → `Bearer [redacted]`.
    fn scrub_bearer(&mut self, text: &str) -> Option<String> {
        let lower = text.to_ascii_lowercase();
        if !lower.contains("bearer ") && !lower.contains("basic ") {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut index = 0;
        let mut changed = false;
        while index < text.len() {
            let rest = &lower[index..];
            let scheme = ["bearer ", "basic "].into_iter().find(|scheme| rest.starts_with(scheme));
            if let Some(scheme) = scheme
                && is_boundary(text, index)
            {
                let start = index + scheme.len();
                let token_len = text[start..].find(|c: char| !is_token_char(c)).unwrap_or(text.len() - start);
                if token_len >= 8 {
                    out.push_str(&text[index..start]);
                    out.push_str(MARK);
                    self.count += 1;
                    changed = true;
                    index = start + token_len;
                    continue;
                }
            }
            let c = text[index..].chars().next().expect("index is on a character");
            out.push(c);
            index += c.len_utf8();
        }
        changed.then_some(out)
    }

    /// JSON web tokens: three base64url parts, the first starting `eyJ`.
    fn scrub_jwt(&mut self, text: &str) -> Option<String> {
        if !text.contains("eyJ") {
            return None;
        }
        let mut changed = false;
        let out = replace_words(text, is_token_char, |word| {
            let parts: Vec<&str> = word.split('.').collect();
            let jwt = parts.len() == 3
                && word.starts_with("eyJ")
                && parts.iter().all(|p| {
                    p.len() >= 8 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                });
            jwt.then(|| {
                changed = true;
                "[redacted:jwt]".to_owned()
            })
        });
        if changed {
            self.count += 1;
        }
        changed.then_some(out)
    }

    /// `password=…`, `token: …`, `"secret":"…"` → the value replaced.
    fn scrub_assignments(&mut self, text: &str) -> Option<String> {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut copied = 0;
        let mut changed = false;
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'=' && bytes[i] != b':' {
                i += 1;
                continue;
            }
            // The key before the sign, skipping a closing quote.
            let mut key_end = i;
            while key_end > 0 && matches!(bytes[key_end - 1], b'"' | b'\'') {
                key_end -= 1;
            }
            let mut key_start = key_end;
            while key_start > 0 && is_key_byte(bytes[key_start - 1]) {
                key_start -= 1;
            }
            let key = &text[key_start..key_end];
            if key.is_empty() || !self.is_sensitive(key) {
                i += 1;
                continue;
            }
            // The value after it, skipping spaces and an opening quote.
            let mut value_start = i + 1;
            while value_start < bytes.len() && bytes[value_start] == b' ' {
                value_start += 1;
            }
            let quote = bytes.get(value_start).copied().filter(|b| matches!(b, b'"' | b'\''));
            if quote.is_some() {
                value_start += 1;
            }
            // `Authorization: Bearer …`: the scheme stays, the token goes.
            let scheme = ["bearer ", "basic "].into_iter().find(|scheme| {
                text[value_start..].get(..scheme.len()).is_some_and(|s| s.eq_ignore_ascii_case(scheme))
            });
            if let Some(scheme) = scheme {
                value_start += scheme.len();
            }
            if text[value_start..].starts_with(MARK) {
                i = value_start + MARK.len();
                continue;
            }
            let value_end = match quote {
                Some(q) => text[value_start..].find(q as char).map_or(bytes.len(), |end| value_start + end),
                None => text[value_start..]
                    .find(|c: char| {
                        c.is_whitespace() || matches!(c, '&' | ',' | ';' | ')' | ']' | '}' | '"' | '\'')
                    })
                    .map_or(bytes.len(), |end| value_start + end),
            };
            if value_end > value_start && &text[value_start..value_end] != MARK {
                out.push_str(&text[copied..value_start]);
                out.push_str(MARK);
                copied = value_end;
                self.count += 1;
                changed = true;
            }
            i = value_end.max(i + 1);
        }
        out.push_str(&text[copied..]);
        changed.then_some(out)
    }

    /// Card numbers: 13 to 19 digits, spaces or dashes between, passing the
    /// Luhn check.
    fn scrub_cards(&mut self, text: &str) -> Option<String> {
        if text.bytes().filter(u8::is_ascii_digit).count() < 13 {
            return None;
        }
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut copied = 0;
        let mut changed = false;
        let mut i = 0;
        while i < bytes.len() {
            if !bytes[i].is_ascii_digit() || (i > 0 && bytes[i - 1].is_ascii_alphanumeric()) {
                i += 1;
                continue;
            }
            let mut end = i;
            let mut digits = Vec::new();
            while end < bytes.len() {
                match bytes[end] {
                    b if b.is_ascii_digit() => digits.push(b - b'0'),
                    b' ' | b'-' if end + 1 < bytes.len() && bytes[end + 1].is_ascii_digit() => {}
                    _ => break,
                }
                end += 1;
            }
            let bounded = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if bounded && (13..=19).contains(&digits.len()) && luhn(&digits) {
                out.push_str(&text[copied..i]);
                out.push_str("[redacted:card]");
                copied = end;
                self.count += 1;
                changed = true;
            }
            i = end.max(i + 1);
        }
        out.push_str(&text[copied..]);
        changed.then_some(out)
    }

    fn scrub_emails(&mut self, text: &str) -> Option<String> {
        if !self.rules.scrub_emails || !text.contains('@') {
            return None;
        }
        let mut count = 0;
        let out = replace_words(
            text,
            |c| c.is_alphanumeric() || "._%+-@".contains(c),
            |word| {
                let word = word.trim_end_matches('.');
                let (local, domain) = word.split_once('@')?;
                let valid = !local.is_empty()
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.contains('@');
                valid.then(|| {
                    count += 1;
                    "[redacted:email]".to_owned()
                })
            },
        );
        self.count += count;
        (count > 0).then_some(out)
    }

    fn scrub_ips(&mut self, text: &str) -> Option<String> {
        if !self.rules.scrub_ip_addresses {
            return None;
        }
        let mut count = 0;
        let out = replace_words(
            text,
            |c| c.is_ascii_digit() || c == '.',
            |word| {
                let parts: Vec<&str> = word.split('.').collect();
                let ip = parts.len() == 4
                    && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.parse::<u8>().is_ok());
                ip.then(|| {
                    count += 1;
                    "[redacted:ip]".to_owned()
                })
            },
        );
        self.count += count;
        (count > 0).then_some(out)
    }
}

fn normalize(key: &str) -> String {
    key.chars().filter(|c| !matches!(c, '_' | '-' | '.' | ' ')).flat_map(char::to_lowercase).collect()
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        _ => false,
    }
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~' | '+' | '/' | '=')
}

fn is_key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

fn is_boundary(text: &str, index: usize) -> bool {
    index == 0 || !text[..index].chars().next_back().is_some_and(char::is_alphanumeric)
}

/// Replaces maximal runs of `in_word` characters for which `replace` says
/// so.
fn replace_words(
    text: &str,
    in_word: impl Fn(char) -> bool,
    mut replace: impl FnMut(&str) -> Option<String>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars() {
        if in_word(c) {
            word.push(c);
            continue;
        }
        flush(&mut out, &mut word, &mut replace);
        out.push(c);
    }
    flush(&mut out, &mut word, &mut replace);
    out
}

fn flush(out: &mut String, word: &mut String, replace: &mut impl FnMut(&str) -> Option<String>) {
    if word.is_empty() {
        return;
    }
    match replace(word) {
        Some(replacement) => {
            // A sentence's full stop after an address stays.
            let trailing: String = word.chars().rev().take_while(|c| *c == '.').collect();
            out.push_str(&replacement);
            out.push_str(&trailing);
        }
        None => out.push_str(word),
    }
    word.clear();
}

fn luhn(digits: &[u8]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            let d = u32::from(*d);
            if i % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rules: &RedactionRules, input: &str) -> (String, u32) {
        let mut redactor = Redactor::new(rules);
        let out = redactor.text(input);
        (out, redactor.count())
    }

    fn plain(input: &str) -> String {
        text(&RedactionRules::default(), input).0
    }

    #[test]
    fn replaces_values_under_sensitive_keys() {
        let rules = RedactionRules { extra_keys: vec!["Customer-PIN".into()], ..Default::default() };
        let mut redactor = Redactor::new(&rules);
        let fields = vec![
            ("password".to_owned(), Value::String("hunter2".into())),
            ("X-Api-Key".to_owned(), Value::Int(1234)),
            ("user.session_id".to_owned(), Value::String("abc".into())),
            ("customer_pin".to_owned(), Value::String("0000".into())),
            ("order_id".to_owned(), Value::Int(42)),
            ("author".to_owned(), Value::String("Ada".into())),
            ("token".to_owned(), Value::Null),
            ("headers".to_owned(), Value::Object(vec![("Cookie".to_owned(), Value::String("a=b".into()))])),
        ];
        let out = redactor.attributes(fields);
        let shown: Vec<String> =
            out.iter().map(|(k, v)| format!("{k}={}", v.to_text().unwrap_or("…".into()))).collect();
        assert_eq!(
            shown,
            [
                "password=[redacted]",
                "X-Api-Key=[redacted]",
                "user.session_id=[redacted]",
                "customer_pin=[redacted]",
                "order_id=42",
                "author=Ada",
                "token=…",
                "headers=…",
            ]
        );
        assert_eq!(out[7].1.get("Cookie").and_then(Value::as_str), Some("[redacted]"));
        assert_eq!(redactor.count(), 5);
    }

    #[test]
    fn keeps_only_allowed_keys_when_told_to() {
        let rules = RedactionRules {
            allowed_keys: Some(vec!["order_id".into(), "user.*".into()]),
            ..Default::default()
        };
        let mut redactor = Redactor::new(&rules);
        let fields = vec![
            ("order_id".to_owned(), Value::Int(1)),
            ("user.id".to_owned(), Value::Int(2)),
            ("payload".to_owned(), Value::String("everything".into())),
        ];
        let keys: Vec<String> = redactor.attributes(fields).into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["order_id", "user.id"]);
        assert_eq!(redactor.count(), 1);
    }

    #[test]
    fn scrubs_credentials_in_urls_and_authorization_headers() {
        assert_eq!(
            plain("connect to postgres://app:s3cret@db:5432/app failed"),
            "connect to postgres://[redacted]@db:5432/app failed"
        );
        assert_eq!(plain("see https://example.com/a@b"), "see https://example.com/a@b");
        assert_eq!(plain("Authorization: Bearer abcdefgh12345678"), "Authorization: Bearer [redacted]");
        assert_eq!(plain("basic dXNlcjpwYXNz is not short"), "basic [redacted] is not short");
        assert_eq!(plain("a bearer of bad news"), "a bearer of bad news");
    }

    #[test]
    fn scrubs_tokens_and_assignments() {
        let jwt =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert_eq!(plain(&format!("token {jwt} expired")), "token [redacted:jwt] expired");
        assert_eq!(plain("login with password=hunter2&next=/"), "login with password=[redacted]&next=/");
        assert_eq!(
            plain(r#"{"user":"ada","api_token":"abc123"}"#),
            r#"{"user":"ada","api_token":"[redacted]"}"#
        );
        assert_eq!(plain("secret: 'xyz', ok"), "secret: '[redacted]', ok");
        assert_eq!(plain("time: 12:30 and ratio=3"), "time: 12:30 and ratio=3");
    }

    #[test]
    fn scrubs_card_numbers_that_pass_luhn_only() {
        assert_eq!(plain("paid with 4111 1111 1111 1111."), "paid with [redacted:card].");
        assert_eq!(plain("card 4111-1111-1111-1111 ok"), "card [redacted:card] ok");
        assert_eq!(plain("order 4111111111111112 is no card"), "order 4111111111111112 is no card");
        assert_eq!(plain("id abc4111111111111111"), "id abc4111111111111111");
    }

    #[test]
    fn scrubs_addresses_only_when_asked() {
        assert_eq!(plain("mail ada@example.com from 10.0.0.1"), "mail ada@example.com from 10.0.0.1");
        let rules = RedactionRules { scrub_emails: true, scrub_ip_addresses: true, ..Default::default() };
        let (out, count) = text(&rules, "mail ada@example.com. from 10.0.0.1, not 1.2.3");
        assert_eq!(out, "mail [redacted:email]. from [redacted:ip], not 1.2.3");
        assert_eq!(count, 2);
    }

    #[test]
    fn walks_nested_values() {
        let rules = RedactionRules::default();
        let mut redactor = Redactor::new(&rules);
        let value = Value::Array(vec![
            Value::String("password=x".into()),
            Value::Object(vec![("secret".to_owned(), Value::Array(vec![Value::Int(1)]))]),
            Value::Object(vec![("secret".to_owned(), Value::String(String::new()))]),
        ]);
        let out = redactor.value(value);
        assert_eq!(out.as_array().unwrap()[0].as_str(), Some("password=[redacted]"));
        assert_eq!(out.as_array().unwrap()[1].get("secret").and_then(Value::as_str), Some("[redacted]"));
        assert_eq!(out.as_array().unwrap()[2].get("secret").and_then(Value::as_str), Some(""));
        assert_eq!(redactor.count(), 2);
        assert!(!redactor.is_sensitive(""));
    }
}
