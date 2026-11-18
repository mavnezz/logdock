//! What this context needs from the outside world.

use async_trait::async_trait;
use logdock_kernel::PortError;

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
}
