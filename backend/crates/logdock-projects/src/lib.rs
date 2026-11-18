//! Bounded context *Projects*: the applications that send to Logdock, their
//! stages (environments), the ingest keys that tie data to both, each
//! project's settings, and what ingest accepted and rejected.

pub mod application;
pub mod domain;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
