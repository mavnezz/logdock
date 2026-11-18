//! Bounded context *Logs*: the log entries apps send, as they are kept
//! (limited and redacted), found again (search, trace, live view) and
//! counted.

pub mod application;
pub mod domain;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
