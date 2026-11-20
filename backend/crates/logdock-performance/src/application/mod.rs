//! Use cases of the Performance context and the ports they need.

pub mod ports;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use logdock_kernel::redaction::{RedactionRules, Redactor};
use logdock_kernel::{Clock, PortError, Scope, Timestamp, Value};

use crate::domain::{Rollup, Transaction};
use ports::{RollupQuery, RollupStore, SampleQuery, StoredTransaction, TransactionStore, TransactionSummary};

#[derive(Debug, thiserror::Error)]
pub enum PerformanceError {
    #[error("the transaction has no start or end")]
    Unreadable,
    #[error("{0}")]
    InvalidQuery(&'static str),
    #[error(transparent)]
    Port(#[from] PortError),
}

/// Adapters for every port of this context.
#[derive(Clone)]
pub struct PerformancePorts {
    pub transactions: Arc<dyn TransactionStore>,
    pub rollups: Arc<dyn RollupStore>,
    pub clock: Arc<dyn Clock>,
}

/// All use cases of this context.
#[derive(Clone)]
pub struct PerformanceUseCases {
    pub ingest: IngestTransaction,
    pub overview: Overview,
    pub extents: Extents,
    pub series: Series,
    pub samples: Samples,
    pub trace: TransactionsOfTrace,
}

impl PerformanceUseCases {
    pub fn new(ports: PerformancePorts) -> Self {
        let ports = Arc::new(ports);
        Self {
            ingest: IngestTransaction { ports: Arc::clone(&ports) },
            overview: Overview { ports: Arc::clone(&ports) },
            extents: Extents { ports: Arc::clone(&ports) },
            series: Series { ports: Arc::clone(&ports) },
            samples: Samples { ports: Arc::clone(&ports) },
            trace: TransactionsOfTrace { ports },
        }
    }
}

/// Transactions from the future get the time of receipt (an hour of skew).
const MAX_SKEW_MICROS: i64 = 3_600_000_000;

/// Command: keeps a transaction under its key's scope and counts it and its
/// spans in the rollups of its minute.
#[derive(Clone)]
pub struct IngestTransaction {
    ports: Arc<PerformancePorts>,
}

impl IngestTransaction {
    /// `sample_rate` is the share of transactions the SDK sends (the
    /// envelope's `trace.sample_rate`); each one stands for 1/rate.
    pub async fn execute(
        &self,
        scope: Scope,
        rules: &RedactionRules,
        payload: Value,
        sample_rate: Option<f64>,
    ) -> Result<Transaction, PerformanceError> {
        // Span descriptions and data hold URLs and statements.
        let payload = Redactor::new(rules).value(payload);
        let mut transaction = Transaction::from_payload(&payload).ok_or(PerformanceError::Unreadable)?;
        let received = self.ports.clock.now();
        if transaction.start.unix_micros() > received.unix_micros() + MAX_SKEW_MICROS {
            let shift = transaction.start.unix_micros() - received.unix_micros();
            transaction.start = received;
            for span in &mut transaction.spans {
                span.start = Timestamp::from_unix_micros(span.start.unix_micros() - shift);
            }
        }
        let weight = match sample_rate {
            Some(rate) if rate > 0.0 && rate < 1.0 => (1.0 / rate).round().clamp(1.0, 1_000_000.0) as u64,
            _ => 1,
        };
        self.ports.transactions.insert(scope, &transaction, received).await?;
        self.ports
            .rollups
            .add(scope, transaction.start.minute(), &transaction.measurements(), weight)
            .await?;
        Ok(transaction)
    }
}

/// A name with what its range adds up to.
#[derive(Debug, Clone, PartialEq)]
pub struct NameRow {
    pub name: String,
    pub rollup: Rollup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// Where the time goes: count × average.
    #[default]
    TotalTime,
    Count,
    Slowest,
    Failures,
}

/// Query: each name of one kind with the first and last minute it was
/// measured in the range.
#[derive(Clone)]
pub struct Extents {
    ports: Arc<PerformancePorts>,
}

impl Extents {
    pub async fn execute(
        &self,
        query: RollupQuery,
    ) -> Result<Vec<(String, Timestamp, Timestamp)>, PerformanceError> {
        check(&query)?;
        let rows = self.ports.rollups.extents(&query).await?;
        Ok(rows
            .into_iter()
            .map(|(name, first, last)| {
                (name, Timestamp::from_unix_seconds(first), Timestamp::from_unix_seconds(last + 59))
            })
            .collect())
    }
}

/// Query: the names of one kind, added up over a range.
#[derive(Clone)]
pub struct Overview {
    ports: Arc<PerformancePorts>,
}

impl Overview {
    pub async fn execute(
        &self,
        query: RollupQuery,
        order: Order,
        limit: usize,
    ) -> Result<Vec<NameRow>, PerformanceError> {
        check(&query)?;
        let mut rows: Vec<NameRow> = self
            .ports
            .rollups
            .names(&query)
            .await?
            .into_iter()
            .map(|(name, rollup)| NameRow { name, rollup })
            .collect();
        rows.sort_by(|a, b| {
            let key = |r: &Rollup| match order {
                Order::TotalTime => r.total_us,
                Order::Count => r.count,
                Order::Slowest => r.percentile_us(0.95),
                Order::Failures => r.flagged,
            };
            key(&b.rollup).cmp(&key(&a.rollup)).then_with(|| a.name.cmp(&b.name))
        });
        rows.truncate(limit.clamp(1, 1000));
        Ok(rows)
    }
}

fn check(query: &RollupQuery) -> Result<(), PerformanceError> {
    if query.to <= query.from {
        return Err(PerformanceError::InvalidQuery("the range ends before it starts"));
    }
    if query.to.duration_since(query.from).as_secs() > 92 * 86_400 {
        return Err(PerformanceError::InvalidQuery("the range is longer than 92 days"));
    }
    Ok(())
}

/// One name's buckets over a range.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesPage {
    pub bucket_seconds: i64,
    /// Every bucket of the range, also the empty ones, oldest first.
    pub buckets: Vec<(i64, Rollup)>,
    pub total: Rollup,
}

/// Query: one name over time, in about 60 buckets.
#[derive(Clone)]
pub struct Series {
    ports: Arc<PerformancePorts>,
}

impl Series {
    const BUCKET_STEPS: [i64; 10] = [60, 300, 600, 900, 1800, 3600, 7200, 21_600, 43_200, 86_400];

    /// `name` `None`: all names of the kind together.
    pub async fn execute(
        &self,
        query: RollupQuery,
        name: Option<&str>,
    ) -> Result<SeriesPage, PerformanceError> {
        check(&query)?;
        let span = query.to.unix_seconds() - query.from.unix_seconds();
        let bucket_seconds = Self::BUCKET_STEPS
            .into_iter()
            .find(|step| span / step <= 60)
            .unwrap_or(*Self::BUCKET_STEPS.last().unwrap_or(&86_400));
        let found = self.ports.rollups.series(&query, name, bucket_seconds).await?;
        let first = query.from.unix_seconds().div_euclid(bucket_seconds) * bucket_seconds;
        let mut buckets = Vec::new();
        let mut total = Rollup::default();
        let mut start = first;
        while start < query.to.unix_seconds() {
            let rollup = found.iter().find(|(s, _)| *s == start).map(|(_, r)| *r).unwrap_or_default();
            total.merge(&rollup);
            buckets.push((start, rollup));
            start += bucket_seconds;
        }
        Ok(SeriesPage { bucket_seconds, buckets, total })
    }
}

/// Query: transactions of one name, the newest or the slowest.
#[derive(Clone)]
pub struct Samples {
    ports: Arc<PerformancePorts>,
}

impl Samples {
    pub async fn execute(&self, mut query: SampleQuery) -> Result<Vec<TransactionSummary>, PerformanceError> {
        if !query.kind.is_transaction() {
            return Err(PerformanceError::InvalidQuery("only requests, jobs and commands have samples"));
        }
        query.limit = query.limit.clamp(1, 100);
        Ok(self.ports.transactions.samples(&query).await?)
    }
}

/// Query: the transactions of one trace with their spans.
#[derive(Clone)]
pub struct TransactionsOfTrace {
    ports: Arc<PerformancePorts>,
}

impl TransactionsOfTrace {
    pub async fn execute(&self, trace_id: &str) -> Result<Vec<StoredTransaction>, PerformanceError> {
        Ok(self.ports.transactions.by_trace(trace_id).await?)
    }
}
