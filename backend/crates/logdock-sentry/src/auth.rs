//! How SDKs say which key they send with
//! (<https://develop.sentry.dev/sdk/foundations/transport/authentication/>).

/// The `sentry_key` of an `X-Sentry-Auth` header:
/// `Sentry sentry_version=7, sentry_client=…, sentry_key=<key>`.
pub fn key_from_auth_header(header: &str) -> Option<String> {
    let rest = header.trim();
    let rest = rest.strip_prefix("Sentry").or_else(|| rest.strip_prefix("sentry")).unwrap_or(rest);
    rest.split(',')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| name.trim() == "sentry_key")
        .map(|(_, value)| value.trim().to_owned())
        .filter(|key| !key.is_empty())
}

/// The `sentry_key` of a query string: `sentry_version=7&sentry_key=<key>`.
pub fn key_from_query(query: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == "sentry_key")
        .map(|(_, value)| value.to_owned())
        .filter(|key| !key.is_empty())
}

/// The public key of a DSN, `https://<key>@host/<project>`, as the envelope
/// header's `dsn` carries it.
pub fn key_from_dsn(dsn: &str) -> Option<String> {
    let (_, rest) = dsn.split_once("://")?;
    let (user, _) = rest.split_once('@')?;
    let key = user.split(':').next()?;
    (!key.is_empty()).then(|| key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_key_from_every_place_sdks_put_it() {
        let header = "Sentry sentry_version=7, sentry_client=sentry.php/4.37.1, sentry_key=abc123";
        assert_eq!(key_from_auth_header(header).as_deref(), Some("abc123"));
        assert_eq!(key_from_auth_header("Sentry sentry_version=7").as_deref(), None);
        assert_eq!(key_from_auth_header("sentry_key=,x=y").as_deref(), None);
        assert_eq!(key_from_query("sentry_version=7&sentry_key=abc").as_deref(), Some("abc"));
        assert_eq!(key_from_query("x=1").as_deref(), None);
        assert_eq!(key_from_dsn("https://abc@logs.example.com/3").as_deref(), Some("abc"));
        assert_eq!(key_from_dsn("https://abc:secret@logs.example.com/3").as_deref(), Some("abc"));
        assert_eq!(key_from_dsn("https://logs.example.com/3"), None);
        assert_eq!(key_from_dsn("nonsense"), None);
    }
}
