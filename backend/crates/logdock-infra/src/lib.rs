//! Secondary (driven) adapters: the implementations behind the ports that
//! the bounded contexts declare.
//!
//! - [`sqlite`]: the main database, its migrations and repositories.
//! - [`identity`]: password hashing, session tokens, the setup code.
//! - [`segments`]: the day segments and their catalog.
//! - [`logs`]: the log writer, search and counts.
//! - [`health`]: what the writers report, and the readiness probe.

mod clock;
pub mod health;
pub mod identity;
pub mod issues;
mod json;
pub mod logs;
pub mod mail;
pub mod monitors;
pub mod performance;
mod random;
pub mod segments;
pub mod sqlite;

pub use clock::SystemClock;
