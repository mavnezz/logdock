//! Configuration from environment variables only (docs/architecture.md,
//! 2.4), with defaults for everything that can have one.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use logdock_api::Cidr;
use logdock_retention::domain::RetentionPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Json,
    Text,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub web_dir: Option<PathBuf>,
    pub public_url: Option<String>,
    pub secure_cookies: bool,
    pub trusted_proxies: Vec<Cidr>,
    pub retention: RetentionPolicy,
    pub shutdown_timeout: Duration,
    pub log_filter: String,
    pub log_format: LogFormat,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{variable}: {problem}")]
pub struct ConfigError {
    pub variable: &'static str,
    pub problem: String,
}

/// The default listen address inside the container.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:8080";

impl Config {
    /// Reads the process's environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the variables through `lookup`, which returns a variable's value.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let env = Env(&lookup);
        let public_url = env.text("LOGDOCK_PUBLIC_URL").map(|url| url.trim_end_matches('/').to_owned());
        if let Some(url) = &public_url
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return Err(ConfigError {
                variable: "LOGDOCK_PUBLIC_URL",
                problem: "must start with https:// or http://".into(),
            });
        }
        let https = public_url.as_deref().is_none_or(|url| url.starts_with("https://"));
        let defaults = RetentionPolicy::default();
        let retention = RetentionPolicy {
            logs_days: env.parse("LOGDOCK_RETENTION_LOGS_DAYS", defaults.logs_days, parse_days)?,
            events_days: env.parse("LOGDOCK_RETENTION_EVENTS_DAYS", defaults.events_days, parse_days)?,
            traces_days: env.parse("LOGDOCK_RETENTION_TRACES_DAYS", defaults.traces_days, parse_days)?,
            max_storage_bytes: env.parse("LOGDOCK_MAX_STORAGE", defaults.max_storage_bytes, parse_size)?,
            min_free_bytes: env.parse("LOGDOCK_MIN_FREE_DISK", defaults.min_free_bytes, parse_size)?,
        };
        let web_dir = match env.text("LOGDOCK_WEB_DIR") {
            Some(dir) if dir.is_empty() => None,
            Some(dir) => Some(PathBuf::from(dir)),
            None => Some(PathBuf::from("/usr/share/logdock/web")),
        };
        Ok(Self {
            listen: env.parse(
                "LOGDOCK_LISTEN",
                DEFAULT_LISTEN.parse().expect("a valid default"),
                |text| text.parse().map_err(|_| "is no address and port, such as 0.0.0.0:8080".to_owned()),
            )?,
            data_dir: PathBuf::from(env.text("LOGDOCK_DATA_DIR").unwrap_or_else(|| "/data".to_owned())),
            web_dir,
            secure_cookies: env.parse("LOGDOCK_SECURE_COOKIES", https, parse_bool)?,
            public_url,
            trusted_proxies: env.parse("LOGDOCK_TRUSTED_PROXIES", Vec::new(), |text| {
                text.split(',').map(str::trim).filter(|net| !net.is_empty()).map(str::parse).collect()
            })?,
            retention,
            shutdown_timeout: env.parse(
                "LOGDOCK_SHUTDOWN_TIMEOUT",
                Duration::from_secs(20),
                parse_duration,
            )?,
            log_filter: env.text("LOGDOCK_LOG_LEVEL").unwrap_or_else(|| "info".to_owned()),
            log_format: env.parse("LOGDOCK_LOG_FORMAT", LogFormat::Json, |text| match text {
                "json" => Ok(LogFormat::Json),
                "text" => Ok(LogFormat::Text),
                _ => Err("must be json or text".to_owned()),
            })?,
        })
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("logdock.db")
    }
}

struct Env<'a>(&'a dyn Fn(&str) -> Option<String>);

impl Env<'_> {
    fn text(&self, name: &str) -> Option<String> {
        (self.0)(name).map(|value| value.trim().to_owned())
    }

    fn parse<T>(
        &self,
        name: &'static str,
        default: T,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Result<T, ConfigError> {
        match self.text(name) {
            Some(value) if !value.is_empty() => {
                parse(&value).map_err(|problem| ConfigError { variable: name, problem })
            }
            _ => Ok(default),
        }
    }
}

fn parse_days(text: &str) -> Result<u32, String> {
    match text.parse::<u32>() {
        Ok(days) if (1..=3650).contains(&days) => Ok(days),
        _ => Err("must be a number of days from 1 to 3650".to_owned()),
    }
}

fn parse_bool(text: &str) -> Result<bool, String> {
    match text.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err("must be true or false".to_owned()),
    }
}

/// `50GiB`, `500MB`, `1048576`: binary (KiB, MiB, GiB, TiB) and decimal
/// (KB, MB, GB, TB) units, bytes without one.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let split = text.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number.parse().map_err(|_| format!("{text:?} is no size such as 50GiB"))?;
    let factor: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "g" | "gb" => 1e9,
        "t" | "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return Err(format!("{text:?} has an unknown unit")),
    };
    Ok((number * factor).round() as u64)
}

/// `20s`, `5m`, `12h`, `7d`, or milliseconds as `500ms`.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let split = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: u64 = number.parse().map_err(|_| format!("{text:?} is no duration such as 20s"))?;
    let seconds = match unit.trim() {
        "ms" => return Ok(Duration::from_millis(number)),
        "" | "s" => number,
        "m" => number * 60,
        "h" => number * 3600,
        "d" => number * 86_400,
        _ => return Err(format!("{text:?} has an unknown unit")),
    };
    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn config(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars: HashMap<String, String> =
            vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        Config::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn has_defaults_for_everything() {
        let config = config(&[]).unwrap();
        assert_eq!(config.listen.to_string(), DEFAULT_LISTEN);
        assert_eq!(config.database_path(), PathBuf::from("/data/logdock.db"));
        assert_eq!(config.web_dir, Some(PathBuf::from("/usr/share/logdock/web")));
        assert!(config.secure_cookies);
        assert_eq!(config.retention, RetentionPolicy::default());
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(config.shutdown_timeout, Duration::from_secs(20));
    }

    #[test]
    fn reads_what_is_set() {
        let config = config(&[
            ("LOGDOCK_LISTEN", "127.0.0.1:9000"),
            ("LOGDOCK_DATA_DIR", "/tmp/logdock"),
            ("LOGDOCK_WEB_DIR", ""),
            ("LOGDOCK_PUBLIC_URL", "http://logs.local/"),
            ("LOGDOCK_TRUSTED_PROXIES", "10.0.0.0/8, 172.16.0.0/12"),
            ("LOGDOCK_RETENTION_LOGS_DAYS", "14"),
            ("LOGDOCK_MAX_STORAGE", "20GiB"),
            ("LOGDOCK_MIN_FREE_DISK", "500MB"),
            ("LOGDOCK_SHUTDOWN_TIMEOUT", "5s"),
            ("LOGDOCK_LOG_FORMAT", "text"),
        ])
        .unwrap();
        assert_eq!(config.listen.port(), 9000);
        assert_eq!(config.web_dir, None);
        assert_eq!(config.public_url.as_deref(), Some("http://logs.local"));
        assert!(!config.secure_cookies, "plain HTTP has no secure cookies");
        assert_eq!(config.trusted_proxies.len(), 2);
        assert_eq!(config.retention.logs_days, 14);
        assert_eq!(config.retention.max_storage_bytes, 20 * 1024 * 1024 * 1024);
        assert_eq!(config.retention.min_free_bytes, 500_000_000);
        assert_eq!(config.shutdown_timeout, Duration::from_secs(5));
        assert_eq!(config.log_format, LogFormat::Text);
    }

    #[test]
    fn names_the_variable_that_is_wrong() {
        for (variable, value) in [
            ("LOGDOCK_LISTEN", "nowhere"),
            ("LOGDOCK_RETENTION_LOGS_DAYS", "0"),
            ("LOGDOCK_MAX_STORAGE", "lots"),
            ("LOGDOCK_TRUSTED_PROXIES", "10.0.0.0/99"),
            ("LOGDOCK_SECURE_COOKIES", "perhaps"),
            ("LOGDOCK_PUBLIC_URL", "logs.example.com"),
            ("LOGDOCK_LOG_FORMAT", "xml"),
            ("LOGDOCK_SHUTDOWN_TIMEOUT", "5 weeks"),
        ] {
            let error = config(&[(variable, value)]).unwrap_err();
            assert_eq!(error.variable, variable);
        }
    }

    #[test]
    fn reads_sizes_and_durations() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("1KiB").unwrap(), 1024);
        assert_eq!(parse_size("1.5 GB").unwrap(), 1_500_000_000);
        assert_eq!(parse_size("2tib").unwrap(), 2 * 1024u64.pow(4));
        assert!(parse_size("3 parsecs").is_err());
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("12h").unwrap(), Duration::from_secs(43_200));
        assert_eq!(parse_duration("7d").unwrap(), Duration::from_secs(604_800));
        assert_eq!(parse_duration("90").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert!(parse_duration("x").is_err());
        assert_eq!(parse_bool("YES"), Ok(true));
        assert_eq!(parse_bool("off"), Ok(false));
    }
}
