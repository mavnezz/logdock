//! Bounded context *Issues*: error events as SDKs send them, grouped into
//! issues by documented rules (docs/architecture.md, 4.2), the issues'
//! states across releases, and the e-mail about new and returning ones.

pub mod application;
pub mod domain;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
