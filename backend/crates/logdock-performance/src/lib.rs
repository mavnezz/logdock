//! Bounded context *Performance*: transactions as SDKs send them (requests,
//! jobs, commands) with their spans, and rollups per minute of them and of
//! what they spent their time on: queries, outgoing requests, cache
//! (docs/architecture.md, 4.3).

pub mod application;
pub mod domain;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
