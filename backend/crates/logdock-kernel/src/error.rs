use std::error::Error;

/// A secondary adapter behind a port failed (database, file system, mail).
///
/// Contexts treat it as opaque: they cannot recover from it, only pass it
/// on. The source chain is meant for logs, never for API clients.
#[derive(Debug, thiserror::Error)]
#[error("{what} is unavailable")]
pub struct PortError {
    what: &'static str,
    #[source]
    source: Box<dyn Error + Send + Sync>,
}

impl PortError {
    /// `what` names the resource in a few words, e.g. `"log storage"`.
    pub fn new(what: &'static str, source: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self { what, source: source.into() }
    }

    pub fn what(&self) -> &'static str {
        self.what
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_cause_as_source() {
        let error = PortError::new("log storage", "disk full");
        assert_eq!(error.to_string(), "log storage is unavailable");
        assert_eq!(error.source().unwrap().to_string(), "disk full");
        assert_eq!(error.what(), "log storage");
    }
}
