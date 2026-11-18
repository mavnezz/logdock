use tracing_subscriber::EnvFilter;

use crate::config::LogFormat;

/// Logs go to stdout, where Docker collects them: JSON lines by default,
/// readable text for development.
pub fn init(filter: &str, format: LogFormat) -> anyhow::Result<()> {
    let filter = EnvFilter::try_new(filter)
        .map_err(|error| anyhow::anyhow!("LOGDOCK_LOG_LEVEL: {error}"))?
        // sqlx logs every statement at info; only slow ones are worth it.
        .add_directive("sqlx=warn".parse()?);
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stdout);
    let result = match format {
        LogFormat::Json => builder.json().flatten_event(true).with_current_span(false).try_init(),
        LogFormat::Text => builder.try_init(),
    };
    result.map_err(|error| anyhow::anyhow!("cannot set up logging: {error}"))
}
