/// How severe an entry is: Sentry's six levels (trace … fatal). Monolog's
/// eight map onto them as sentry-php maps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Trace = 1,
    Debug = 2,
    Info = 3,
    Warn = 4,
    Error = 5,
    Fatal = 6,
}

impl Level {
    pub const ALL: [Level; 6] =
        [Level::Trace, Level::Debug, Level::Info, Level::Warn, Level::Error, Level::Fatal];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }

    pub fn number(self) -> i64 {
        self as i64
    }

    pub fn from_number(number: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.number() == number)
    }

    /// Sentry's and Monolog's names, in any case: `warning` and `notice`,
    /// `critical`, `alert` and `emergency` too.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "trace" => Self::Trace,
            "debug" => Self::Debug,
            "info" | "notice" | "log" => Self::Info,
            "warn" | "warning" => Self::Warn,
            "error" | "err" => Self::Error,
            "fatal" | "critical" | "alert" | "emergency" => Self::Fatal,
            _ => return None,
        })
    }

    /// Monolog's numbers (100 debug … 600 emergency).
    pub fn from_monolog(number: i64) -> Option<Self> {
        if number <= 0 {
            return None;
        }
        Some(match number {
            ..=100 => Self::Debug,
            101..=250 => Self::Info,
            251..=300 => Self::Warn,
            301..=400 => Self::Error,
            _ => Self::Fatal,
        })
    }

    /// OpenTelemetry's severity numbers, which Sentry's logs carry (1–24).
    pub fn from_severity(number: i64) -> Option<Self> {
        Some(match number {
            1..=4 => Self::Trace,
            5..=8 => Self::Debug,
            9..=12 => Self::Info,
            13..=16 => Self::Warn,
            17..=20 => Self::Error,
            21..=24 => Self::Fatal,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_name_and_number() {
        for level in Level::ALL {
            assert_eq!(Level::parse(level.as_str()), Some(level));
            assert_eq!(Level::from_number(level.number()), Some(level));
        }
        assert_eq!(Level::parse("WARNING"), Some(Level::Warn));
        assert_eq!(Level::parse("notice"), Some(Level::Info));
        assert_eq!(Level::parse("emergency"), Some(Level::Fatal));
        assert_eq!(Level::parse("loud"), None);
        assert_eq!(Level::from_number(9), None);
        assert_eq!(Level::from_monolog(100), Some(Level::Debug));
        assert_eq!(Level::from_monolog(250), Some(Level::Info));
        assert_eq!(Level::from_monolog(300), Some(Level::Warn));
        assert_eq!(Level::from_monolog(400), Some(Level::Error));
        assert_eq!(Level::from_monolog(600), Some(Level::Fatal));
        assert_eq!(Level::from_monolog(0), None);
        assert_eq!(Level::from_severity(13), Some(Level::Warn));
        assert_eq!(Level::from_severity(24), Some(Level::Fatal));
        assert_eq!(Level::from_severity(0), None);
        assert!(Level::Error > Level::Warn);
    }
}
