//! Use cases of the Retention context and the ports they need.

pub mod ports;
mod readiness;

use std::sync::Arc;

pub use readiness::CheckReadiness;

use crate::domain::RetentionPolicy;
use ports::StorageProbe;

/// Adapters for every port of this context, plugged in by the composition
/// root.
#[derive(Clone)]
pub struct RetentionPorts {
    pub probe: Arc<dyn StorageProbe>,
}

/// All use cases of this context.
#[derive(Clone)]
pub struct RetentionUseCases {
    pub check_readiness: CheckReadiness,
}

impl RetentionUseCases {
    pub fn new(ports: RetentionPorts, policy: RetentionPolicy) -> Self {
        Self { check_readiness: CheckReadiness::new(Arc::clone(&ports.probe), policy) }
    }
}
