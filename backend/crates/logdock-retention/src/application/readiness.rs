use std::sync::Arc;

use crate::application::ports::StorageProbe;
use crate::domain::{Problem, Readiness, RetentionPolicy};

/// Query: can Logdock take data now? Behind `/health/ready`, and asked by
/// the ingest endpoints before they write.
#[derive(Clone)]
pub struct CheckReadiness {
    probe: Arc<dyn StorageProbe>,
    policy: RetentionPolicy,
}

impl CheckReadiness {
    pub(crate) fn new(probe: Arc<dyn StorageProbe>, policy: RetentionPolicy) -> Self {
        Self { probe, policy }
    }

    pub async fn execute(&self) -> Readiness {
        let mut problems = Vec::new();
        if !self.probe.database_ok().await {
            problems.push(Problem::DatabaseUnavailable);
        }
        match self.probe.free_bytes().await {
            Ok(free_bytes) if free_bytes < self.policy.min_free_bytes => {
                problems.push(Problem::LowDisk { free_bytes, min_free_bytes: self.policy.min_free_bytes });
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(error = %error, "cannot read the free space of the data directory");
            }
        }
        problems.extend(
            self.probe.stalled_writers().await.into_iter().map(|writer| Problem::WriterStalled { writer }),
        );
        Readiness { problems }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use logdock_kernel::PortError;

    use super::*;

    struct Probe {
        database: bool,
        free: Result<u64, &'static str>,
        stalled: Vec<&'static str>,
    }

    #[async_trait]
    impl StorageProbe for Probe {
        async fn database_ok(&self) -> bool {
            self.database
        }

        async fn free_bytes(&self) -> Result<u64, PortError> {
            self.free.map_err(|e| PortError::new("disk", e))
        }

        async fn stalled_writers(&self) -> Vec<&'static str> {
            self.stalled.clone()
        }

        async fn database_bytes(&self) -> Result<u64, PortError> {
            Ok(0)
        }
    }

    fn check(probe: Probe) -> CheckReadiness {
        let policy = RetentionPolicy { min_free_bytes: 100, ..RetentionPolicy::default() };
        CheckReadiness::new(Arc::new(probe), policy)
    }

    #[tokio::test]
    async fn is_ready_when_all_is_well() {
        let ready = check(Probe { database: true, free: Ok(100), stalled: vec![] }).execute().await;
        assert!(ready.is_ready());
        // An unreadable free space is logged, not taken for a full disk.
        let unknown = check(Probe { database: true, free: Err("statvfs"), stalled: vec![] }).execute().await;
        assert!(unknown.is_ready());
    }

    #[tokio::test]
    async fn names_every_problem() {
        let probe = Probe { database: false, free: Ok(99), stalled: vec!["logs"] };
        let readiness = check(probe).execute().await;
        assert_eq!(
            readiness.problems,
            [
                Problem::DatabaseUnavailable,
                Problem::LowDisk { free_bytes: 99, min_free_bytes: 100 },
                Problem::WriterStalled { writer: "logs" },
            ]
        );
    }
}
