//! Bounded context *Monitors*: scheduled tasks as the SDK reports them
//! (check-ins), with their schedules, and the runs that fail, take too long
//! or do not happen when the schedule says (docs/architecture.md, 4.3).

pub mod application;
pub mod domain;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
