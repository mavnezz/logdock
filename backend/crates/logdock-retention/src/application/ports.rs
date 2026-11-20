//! What this context needs from the outside world.

use async_trait::async_trait;
use logdock_kernel::{Day, PortError, Timestamp};

use crate::domain::{ScopeUsage, Segment};

/// The state of the data directory and the writers.
#[async_trait]
pub trait StorageProbe: Send + Sync {
    /// Whether the main database answers.
    async fn database_ok(&self) -> bool;

    /// Free space in the data directory.
    async fn free_bytes(&self) -> Result<u64, PortError>;

    /// Writers that have had work waiting without committing any for too
    /// long.
    async fn stalled_writers(&self) -> Vec<&'static str>;

    /// The size of the main database's files.
    async fn database_bytes(&self) -> Result<u64, PortError>;
}

/// The day segments.
#[async_trait]
pub trait SegmentCatalog: Send + Sync {
    /// Every segment with its size.
    async fn segments(&self) -> Result<Vec<Segment>, PortError>;

    async fn delete(&self, segment: Segment) -> Result<(), PortError>;

    /// Closes the segments of days before `today` that are not closed yet;
    /// returns how many.
    async fn close_before(&self, today: Day) -> Result<u32, PortError>;

    /// Measures what each project's stage takes of the segments that
    /// changed since they were measured last: today's, and those closed
    /// since. Returns how many it measured.
    async fn measure(&self) -> Result<u32, PortError>;

    /// What each project's stage takes, over all segments, as measured.
    async fn usage(&self) -> Result<Vec<ScopeUsage>, PortError>;
}

/// What the main database keeps beside the segments.
#[async_trait]
pub trait Housekeeping: Send + Sync {
    /// Deletes counts per minute from before `before`; returns how many rows.
    async fn prune_rollups(&self, before: Timestamp) -> Result<u64, PortError>;
}
