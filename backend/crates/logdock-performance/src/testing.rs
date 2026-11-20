//! In-memory implementations of every port, for tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use logdock_kernel::{Clock, PortError, Scope, Timestamp};

use crate::application::ports::{
    RollupQuery, RollupStore, SampleQuery, StoredTransaction, TransactionStore, TransactionSummary,
};
use crate::application::{PerformancePorts, PerformanceUseCases};
use crate::domain::{Kind, Measurement, Rollup, Transaction};

type Key = (i64, i64, i64, Kind, String);

#[derive(Default)]
pub struct MemoryRollups(pub Mutex<BTreeMap<Key, Rollup>>);

fn scope_of(project: i64, environment: i64) -> Scope {
    Scope {
        project: logdock_kernel::ProjectId::new(project),
        environment: logdock_kernel::EnvironmentId::new(environment),
    }
}

fn wanted(query: &RollupQuery, scope: Scope, minute: i64, kind: Kind) -> bool {
    kind == query.kind
        && minute >= query.from.unix_seconds().div_euclid(60) * 60
        && minute < query.to.unix_seconds()
        && (query.projects.is_empty() || query.projects.contains(&scope.project))
        && (query.environments.is_empty() || query.environments.contains(&scope.environment))
}

#[async_trait]
impl RollupStore for MemoryRollups {
    async fn add(
        &self,
        scope: Scope,
        minute: i64,
        measurements: &[Measurement],
        weight: u64,
    ) -> Result<(), PortError> {
        let mut rollups = self.0.lock().unwrap();
        for m in measurements {
            let key = (scope.project.get(), scope.environment.get(), minute, m.kind, m.name.clone());
            rollups.entry(key).or_default().add(m.duration_us, m.flagged, weight);
        }
        Ok(())
    }

    async fn names(&self, query: &RollupQuery) -> Result<Vec<(String, Rollup)>, PortError> {
        let mut names: BTreeMap<String, Rollup> = BTreeMap::new();
        for ((project, environment, minute, kind, name), rollup) in self.0.lock().unwrap().iter() {
            if wanted(query, scope_of(*project, *environment), *minute, *kind) {
                names.entry(name.clone()).or_default().merge(rollup);
            }
        }
        Ok(names.into_iter().collect())
    }

    async fn extents(&self, query: &RollupQuery) -> Result<Vec<(String, i64, i64)>, PortError> {
        let mut names: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        for (project, environment, minute, kind, name) in self.0.lock().unwrap().keys() {
            if wanted(query, scope_of(*project, *environment), *minute, *kind) {
                let range = names.entry(name.clone()).or_insert((*minute, *minute));
                *range = (range.0.min(*minute), range.1.max(*minute));
            }
        }
        Ok(names.into_iter().map(|(name, (first, last))| (name, first, last)).collect())
    }

    async fn series(
        &self,
        query: &RollupQuery,
        name: &str,
        bucket_seconds: i64,
    ) -> Result<Vec<(i64, Rollup)>, PortError> {
        let mut buckets: BTreeMap<i64, Rollup> = BTreeMap::new();
        for ((project, environment, minute, kind, n), rollup) in self.0.lock().unwrap().iter() {
            if n == name && wanted(query, scope_of(*project, *environment), *minute, *kind) {
                buckets.entry(minute.div_euclid(bucket_seconds) * bucket_seconds).or_default().merge(rollup);
            }
        }
        Ok(buckets.into_iter().collect())
    }
}

#[derive(Default)]
pub struct MemoryTransactions(pub Mutex<Vec<StoredTransaction>>);

#[async_trait]
impl TransactionStore for MemoryTransactions {
    async fn insert(
        &self,
        scope: Scope,
        transaction: &Transaction,
        _received: Timestamp,
    ) -> Result<(), PortError> {
        self.0.lock().unwrap().push(StoredTransaction { scope, transaction: transaction.clone() });
        Ok(())
    }

    async fn samples(&self, query: &SampleQuery) -> Result<Vec<TransactionSummary>, PortError> {
        let mut found: Vec<TransactionSummary> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.transaction.kind == query.kind && s.transaction.display_name() == query.name)
            .filter(|s| s.transaction.start >= query.from && s.transaction.start < query.to)
            .map(|s| summary(s.scope, &s.transaction))
            .collect();
        if query.slowest {
            found.sort_by_key(|s| std::cmp::Reverse(s.duration_us));
        } else {
            found.sort_by_key(|s| std::cmp::Reverse(s.start));
        }
        found.truncate(query.limit as usize);
        Ok(found)
    }

    async fn by_trace(&self, trace_id: &str) -> Result<Vec<StoredTransaction>, PortError> {
        Ok(self.0.lock().unwrap().iter().filter(|s| s.transaction.trace_id == trace_id).cloned().collect())
    }
}

pub fn summary(scope: Scope, t: &Transaction) -> TransactionSummary {
    TransactionSummary {
        scope,
        trace_id: t.trace_id.clone(),
        span_id: t.span_id.clone(),
        kind: t.kind,
        name: t.display_name(),
        start: t.start,
        duration_us: t.duration_us,
        status: t.status.clone(),
        http_status: t.http_status,
        failed: t.failed(),
        release: t.release.clone(),
        service: t.service.clone(),
        host: t.host.clone(),
    }
}

pub struct FixedClock(pub Mutex<Timestamp>);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

pub struct TestPerformance {
    pub use_cases: PerformanceUseCases,
    pub rollups: Arc<MemoryRollups>,
    pub transactions: Arc<MemoryTransactions>,
    pub clock: Arc<FixedClock>,
}

impl TestPerformance {
    pub fn new() -> Self {
        let rollups = Arc::new(MemoryRollups::default());
        let transactions = Arc::new(MemoryTransactions::default());
        let clock = Arc::new(FixedClock(Mutex::new(Timestamp::from_unix_seconds(1_795_050_600))));
        let use_cases = PerformanceUseCases::new(PerformancePorts {
            transactions: transactions.clone(),
            rollups: rollups.clone(),
            clock: clock.clone(),
        });
        Self { use_cases, rollups, transactions, clock }
    }
}

impl Default for TestPerformance {
    fn default() -> Self {
        Self::new()
    }
}
