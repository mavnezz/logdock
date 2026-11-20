//! Use cases of the Retention context and the ports they need.

pub mod ports;
mod readiness;
mod retention;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use logdock_kernel::Clock;

pub use readiness::CheckReadiness;
pub use retention::{ApplyRetention, GetStorageStatus, RetentionReport};

use crate::domain::RetentionPolicy;
use ports::{Housekeeping, SegmentCatalog, StorageProbe};

/// Adapters for every port of this context, plugged in by the composition
/// root.
#[derive(Clone)]
pub struct RetentionPorts {
    pub probe: Arc<dyn StorageProbe>,
    pub catalog: Arc<dyn SegmentCatalog>,
    pub housekeeping: Arc<dyn Housekeeping>,
    pub clock: Arc<dyn Clock>,
}

/// All use cases of this context.
#[derive(Clone)]
pub struct RetentionUseCases {
    pub check_readiness: CheckReadiness,
    pub apply: ApplyRetention,
    pub status: GetStorageStatus,
}

impl RetentionUseCases {
    pub fn new(ports: RetentionPorts, policy: RetentionPolicy) -> Self {
        Self {
            check_readiness: CheckReadiness::new(Arc::clone(&ports.probe), policy),
            apply: ApplyRetention {
                catalog: Arc::clone(&ports.catalog),
                housekeeping: ports.housekeeping,
                clock: Arc::clone(&ports.clock),
                policy,
            },
            status: GetStorageStatus {
                catalog: ports.catalog,
                probe: ports.probe,
                clock: ports.clock,
                policy,
            },
        }
    }
}
