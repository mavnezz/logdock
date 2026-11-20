use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::{Clock, Day, PortError, Timestamp};

use super::*;
use crate::domain::{DeletionReason, Kind, Segment};

struct Fake {
    segments: Mutex<Vec<Segment>>,
    deleted: Mutex<Vec<Segment>>,
    pruned_before: Mutex<Option<Timestamp>>,
}

#[async_trait]
impl SegmentCatalog for Fake {
    async fn segments(&self) -> Result<Vec<Segment>, PortError> {
        Ok(self.segments.lock().unwrap().clone())
    }

    async fn delete(&self, segment: Segment) -> Result<(), PortError> {
        self.segments.lock().unwrap().retain(|s| *s != segment);
        self.deleted.lock().unwrap().push(segment);
        Ok(())
    }

    async fn close_before(&self, _today: Day) -> Result<u32, PortError> {
        Ok(2)
    }

    async fn measure(&self) -> Result<u32, PortError> {
        Ok(3)
    }

    async fn usage(&self) -> Result<Vec<crate::domain::ScopeUsage>, PortError> {
        Ok(vec![crate::domain::ScopeUsage {
            project: logdock_kernel::ProjectId::new(1),
            environment: logdock_kernel::EnvironmentId::new(2),
            kind: Kind::Logs,
            rows: 10,
            bytes: 5,
        }])
    }
}

#[async_trait]
impl Housekeeping for Fake {
    async fn prune_rollups(&self, before: Timestamp) -> Result<u64, PortError> {
        *self.pruned_before.lock().unwrap() = Some(before);
        Ok(7)
    }
}

#[async_trait]
impl ports::StorageProbe for Fake {
    async fn database_ok(&self) -> bool {
        true
    }

    async fn free_bytes(&self) -> Result<u64, PortError> {
        Err(PortError::new("disk", "no statvfs"))
    }

    async fn stalled_writers(&self) -> Vec<&'static str> {
        Vec::new()
    }

    async fn database_bytes(&self) -> Result<u64, PortError> {
        Ok(42)
    }
}

struct Today;

impl Clock for Today {
    fn now(&self) -> Timestamp {
        Day::from_days(20_010).start()
    }
}

#[tokio::test]
async fn deletes_what_ran_out_closes_finished_days_and_reports_storage() {
    let segment = |kind, day: i64, bytes| Segment { kind, day: Day::from_days(day), keep: 0, bytes };
    let fake = Arc::new(Fake {
        segments: Mutex::new(vec![
            segment(Kind::Logs, 20_010, 5),
            segment(Kind::Logs, 19_900, 5),
            segment(Kind::Traces, 20_000, 9),
        ]),
        deleted: Mutex::new(Vec::new()),
        pruned_before: Mutex::new(None),
    });
    let policy = RetentionPolicy { traces_days: 7, rollups_days: 10, ..RetentionPolicy::default() };
    let ports = RetentionPorts {
        probe: fake.clone(),
        catalog: fake.clone(),
        housekeeping: fake.clone(),
        clock: Arc::new(Today),
    };
    let retention = RetentionUseCases::new(ports, policy);

    let report = retention.apply.execute().await.unwrap();
    assert_eq!((report.closed, report.measured), (2, 3));
    assert_eq!(report.pruned_rollups, 7);
    let deleted: Vec<_> =
        report.deleted.iter().map(|d| (d.segment.kind, d.segment.day.days(), d.reason)).collect();
    assert_eq!(
        deleted,
        [(Kind::Logs, 19_900, DeletionReason::Expired), (Kind::Traces, 20_000, DeletionReason::Expired)]
    );
    assert_eq!(*fake.pruned_before.lock().unwrap(), Some(Day::from_days(20_000).start()));

    assert_eq!(fake.deleted.lock().unwrap().len(), 2);
    let status = retention.status.execute().await.unwrap();
    assert_eq!(status.segment_bytes(), 5);
    assert_eq!(status.forecast, None, "no full day left to tell the rate");
    assert_eq!(status.scopes[0].bytes, 5);
    assert_eq!(status.database_bytes, 42);
    assert_eq!(status.free_bytes, None);
}
