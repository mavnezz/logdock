use std::sync::Arc;
use std::time::Duration;

use logdock_kernel::{Clock, PortError};

use crate::application::ports::{Housekeeping, SegmentCatalog, StorageProbe};
use crate::domain::{Deletion, RetentionPolicy, StorageStatus, plan_deletions};

/// What one run of [`ApplyRetention`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionReport {
    pub deleted: Vec<Deletion>,
    pub closed: u32,
    pub pruned_rollups: u64,
}

/// Command: closes finished days, deletes what is older than its retention
/// or above the storage limit, and prunes old counts. Runs every hour.
#[derive(Clone)]
pub struct ApplyRetention {
    pub(crate) catalog: Arc<dyn SegmentCatalog>,
    pub(crate) housekeeping: Arc<dyn Housekeeping>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) policy: RetentionPolicy,
}

impl ApplyRetention {
    pub async fn execute(&self) -> Result<RetentionReport, PortError> {
        let now = self.clock.now();
        let today = now.day();
        let closed = self.catalog.close_before(today).await?;
        let segments = self.catalog.segments().await?;
        let deletions = plan_deletions(&self.policy, &segments, today);
        for deletion in &deletions {
            let segment = deletion.segment;
            self.catalog.delete(segment.kind, segment.day).await?;
            tracing::info!(
                kind = segment.kind.as_str(),
                day = %segment.day,
                bytes = segment.bytes,
                reason = deletion.reason.as_str(),
                "segment deleted"
            );
        }
        let keep = Duration::from_secs(u64::from(self.policy.rollups_days.max(1)) * 86_400);
        let pruned_rollups = self.housekeeping.prune_rollups(now.saturating_sub(keep)).await?;
        Ok(RetentionReport { deleted: deletions, closed, pruned_rollups })
    }
}

/// Query: how much is kept and may be kept.
#[derive(Clone)]
pub struct GetStorageStatus {
    pub(crate) catalog: Arc<dyn SegmentCatalog>,
    pub(crate) probe: Arc<dyn StorageProbe>,
    pub(crate) policy: RetentionPolicy,
}

impl GetStorageStatus {
    pub async fn execute(&self) -> Result<StorageStatus, PortError> {
        let segments = self.catalog.segments().await?;
        let database = self.probe.database_bytes().await?;
        let free = self.probe.free_bytes().await.ok();
        Ok(StorageStatus::new(self.policy, &segments, database, free))
    }
}
