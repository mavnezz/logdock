//! Secondary (driven) adapters: the implementations behind the ports that
//! the bounded contexts declare.
//!
//! - [`sqlite`]: the main database, its migrations and repositories.
//! - [`identity`]: password hashing, session tokens, the setup code.
//! - [`health`]: what the writers report, and the readiness probe.

mod clock;
pub mod health;
pub mod identity;
mod random;
pub mod sqlite;

pub use clock::SystemClock;
