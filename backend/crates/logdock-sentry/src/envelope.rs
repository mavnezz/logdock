//! Envelopes: a header line, then items, each a header line and a payload
//! of the header's `length`, or up to the next newline without one.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("the envelope is empty")]
    Empty,
    #[error("the envelope's header is no JSON object")]
    Header,
    #[error("item {index}'s header is no JSON object with a type")]
    ItemHeader { index: usize },
    #[error("item {index} says it is {length} bytes long, but the envelope ends before")]
    Truncated { index: usize, length: usize },
    #[error("the envelope has more than {max} items")]
    TooManyItems { max: usize },
}

/// How much one envelope may hold.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_items: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_items: 1000 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// The item's type: `event`, `log`, `transaction`, `check_in`, …
    pub kind: String,
    pub header: Value,
    pub payload: Vec<u8>,
}

impl Item {
    /// The payload as JSON, for the types that carry JSON.
    pub fn json(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_slice(&self.payload)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub header: Value,
    pub items: Vec<Item>,
}

impl Envelope {
    pub fn parse(bytes: &[u8], limits: Limits) -> Result<Self, EnvelopeError> {
        let mut rest = bytes;
        let header_line = next_line(&mut rest).filter(|line| !line.iter().all(u8::is_ascii_whitespace));
        let header_line = header_line.ok_or(EnvelopeError::Empty)?;
        let header: Value = serde_json::from_slice(header_line).map_err(|_| EnvelopeError::Header)?;
        if !header.is_object() {
            return Err(EnvelopeError::Header);
        }
        let mut items = Vec::new();
        while !rest.is_empty() {
            let index = items.len();
            let Some(line) = next_line(&mut rest) else { break };
            if line.iter().all(u8::is_ascii_whitespace) {
                // Blank lines between items are tolerated.
                continue;
            }
            if items.len() >= limits.max_items {
                return Err(EnvelopeError::TooManyItems { max: limits.max_items });
            }
            let item_header: Value =
                serde_json::from_slice(line).map_err(|_| EnvelopeError::ItemHeader { index })?;
            let kind = item_header
                .get("type")
                .and_then(Value::as_str)
                .ok_or(EnvelopeError::ItemHeader { index })?
                .to_owned();
            let payload = match item_header.get("length").and_then(Value::as_u64) {
                Some(length) => {
                    let length = usize::try_from(length).unwrap_or(usize::MAX);
                    if rest.len() < length {
                        return Err(EnvelopeError::Truncated { index, length });
                    }
                    let (payload, after) = rest.split_at(length);
                    rest = after;
                    // A newline ends the payload unless the envelope does.
                    if rest.first() == Some(&b'\n') {
                        rest = &rest[1..];
                    }
                    payload.to_vec()
                }
                None => next_line(&mut rest).unwrap_or_default().to_vec(),
            };
            items.push(Item { kind, header: item_header, payload });
        }
        Ok(Self { header, items })
    }

    /// The envelope header's `event_id`, if it has one.
    pub fn event_id(&self) -> Option<&str> {
        self.header.get("event_id").and_then(Value::as_str)
    }

    /// The envelope header's `dsn`, if it has one.
    pub fn dsn(&self) -> Option<&str> {
        self.header.get("dsn").and_then(Value::as_str)
    }

    /// The dynamic sampling context's sample rate, if the SDK sent one.
    pub fn sample_rate(&self) -> Option<f64> {
        let rate = self.header.get("trace")?.get("sample_rate")?;
        rate.as_f64().or_else(|| rate.as_str().and_then(|s| s.parse().ok()))
    }
}

/// The next line without its newline (and a `\r` before it); `None` at the
/// end.
fn next_line<'a>(rest: &mut &'a [u8]) -> Option<&'a [u8]> {
    if rest.is_empty() {
        return None;
    }
    let (line, after) = match rest.iter().position(|b| *b == b'\n') {
        Some(at) => (&rest[..at], &rest[at + 1..]),
        None => (*rest, &rest[rest.len()..]),
    };
    *rest = after;
    Some(line.strip_suffix(b"\r").unwrap_or(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Envelope, EnvelopeError> {
        Envelope::parse(text.as_bytes(), Limits::default())
    }

    #[test]
    fn reads_items_with_and_without_length() {
        let text = concat!(
            r#"{"event_id":"9ec79c33ec9942ab8353589fcb2e04dc","dsn":"https://k@h/1","trace":{"sample_rate":"0.25"}}"#,
            "\n",
            r#"{"type":"attachment","length":10}"#,
            "\n",
            "0123\n56789",
            "\n",
            r#"{"type":"event"}"#,
            "\n",
            r#"{"message":"hi"}"#,
            "\n",
            r#"{"type":"log","item_count":1}"#,
            "\n",
            r#"{"items":[]}"#,
        );
        let envelope = parse(text).unwrap();
        assert_eq!(envelope.event_id(), Some("9ec79c33ec9942ab8353589fcb2e04dc"));
        assert_eq!(envelope.dsn(), Some("https://k@h/1"));
        assert_eq!(envelope.sample_rate(), Some(0.25));
        let kinds: Vec<&str> = envelope.items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["attachment", "event", "log"]);
        assert_eq!(envelope.items[0].payload, b"0123\n56789");
        assert_eq!(envelope.items[1].json().unwrap()["message"], "hi");
    }

    #[test]
    fn tolerates_crlf_trailing_newlines_and_empty_items() {
        let envelope =
            parse("{}\r\n{\"type\":\"session\",\"length\":0}\n\n{\"type\":\"log\"}\r\n{}\r\n\n").unwrap();
        assert_eq!(envelope.items.len(), 2);
        assert!(envelope.items[0].payload.is_empty());
        assert_eq!(envelope.items[1].payload, b"{}");
        assert_eq!(parse("{}").unwrap().items.len(), 0);
        assert_eq!(parse("{}\n{\"type\":\"log\"}").unwrap().items[0].payload, b"");
    }

    #[test]
    fn refuses_what_is_no_envelope() {
        assert_eq!(parse(""), Err(EnvelopeError::Empty));
        assert_eq!(parse("  \n"), Err(EnvelopeError::Empty));
        assert_eq!(parse("not json\n"), Err(EnvelopeError::Header));
        assert_eq!(parse("[1]\n"), Err(EnvelopeError::Header));
        assert_eq!(parse("{}\n{\"length\":2}\nab"), Err(EnvelopeError::ItemHeader { index: 0 }));
        assert_eq!(parse("{}\nnope\n"), Err(EnvelopeError::ItemHeader { index: 0 }));
        assert_eq!(
            parse("{}\n{\"type\":\"x\",\"length\":99}\nab"),
            Err(EnvelopeError::Truncated { index: 0, length: 99 })
        );
        let many = format!("{{}}\n{}", "{\"type\":\"x\",\"length\":0}\n".repeat(3));
        let limits = Limits { max_items: 2 };
        assert_eq!(Envelope::parse(many.as_bytes(), limits), Err(EnvelopeError::TooManyItems { max: 2 }));
    }

    #[test]
    fn never_panics_on_broken_input() {
        // Every prefix and a few corruptions of a valid envelope.
        let valid = "{\"event_id\":\"x\"}\n{\"type\":\"log\",\"length\":13}\n{\"items\":[]}\n{\"type\":\"event\"}\n{}\n";
        for end in 0..=valid.len() {
            let _ = parse(&valid[..end]);
        }
        let bytes = valid.as_bytes();
        for i in 0..bytes.len() {
            let mut broken = bytes.to_vec();
            broken[i] = b"\n{}\"9\x00\xff"[i % 7];
            let _ = Envelope::parse(&broken, Limits::default());
        }
    }
}
