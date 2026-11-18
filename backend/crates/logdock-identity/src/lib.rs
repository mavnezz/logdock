//! Bounded context *Identity*: who may use Logdock's interface and what
//! they may do.
//!
//! Accounts of Logdock's own with two roles, sign-in with sessions, the
//! one-time first setup, and the audit log. Ingest keys are no accounts;
//! they belong to the Projects context.

pub mod application;
pub mod domain;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
