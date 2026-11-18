//! Secondary (driven) adapters: the implementations behind the ports that
//! the bounded contexts declare.
//!
//! - [`sqlite`]: the main database and its migrations.
//! - [`health`]: what the writers report, and the readiness probe.

mod clock;
pub mod health;
pub mod sqlite;

pub use clock::SystemClock;
