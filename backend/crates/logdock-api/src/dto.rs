//! Shapes shared by several routes.

use logdock_kernel::Timestamp;

/// A point in time as the API writes it: RFC 3339 in UTC with
/// microseconds, e.g. `2026-11-18T09:15:02.123456Z`.
pub(crate) fn time(at: Timestamp) -> String {
    at.utc().to_string()
}

pub(crate) fn optional_time(at: Option<Timestamp>) -> Option<String> {
    at.map(time)
}
