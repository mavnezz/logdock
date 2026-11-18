use logdock_kernel::{Secret, constant_time_eq};

/// The one-time code that lets the first administrator be created. Written
/// to Logdock's log at a start without accounts; whoever can read that log
/// runs the setup.
#[derive(Debug, Clone)]
pub struct SetupCode(Secret);

impl SetupCode {
    pub fn new(code: Secret) -> Self {
        Self(code)
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }

    /// Compares what was typed, ignoring case, spaces and dashes.
    pub fn matches(&self, typed: &str) -> bool {
        let normal = |text: &str| -> String {
            text.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
        };
        constant_time_eq(normal(self.0.expose()).as_bytes(), normal(typed).as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_however_it_is_typed() {
        let code = SetupCode::new(Secret::new("K7QX-2M9P"));
        assert!(code.matches("K7QX-2M9P"));
        assert!(code.matches(" k7qx 2m9p "));
        assert!(!code.matches("K7QX-2M9Q"));
        assert!(!code.matches(""));
        assert_eq!(code.expose(), "K7QX-2M9P");
    }
}
