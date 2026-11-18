//! Shared kernel of Logdock.
//!
//! Types that every bounded context and every adapter agrees on. Nothing in
//! here performs I/O.

mod audit;
mod error;
mod ids;
mod request;
mod secret;
mod time;
mod value;

pub use audit::{Actor, AuditEntry, AuditLog, Outcome};
pub use error::PortError;
pub use ids::{EnvironmentId, ProjectId, Scope};
pub use request::RequestContext;
pub use secret::{Secret, constant_time_eq};
pub use time::{Clock, Day, InvalidDay, Timestamp, UtcTime};
pub use value::Value;

/// The version of Logdock that runs: the image's (`LOGDOCK_VERSION` at build
/// time), or the crates' in a build without one.
pub const VERSION: &str = match option_env!("LOGDOCK_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};
