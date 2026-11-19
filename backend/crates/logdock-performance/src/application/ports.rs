//! What this context needs from the outside world.

use async_trait::async_trait;
use logdock_kernel::{EnvironmentId, PortError, ProjectId, Scope, Timestamp};

use crate::domain::{Kind, Measurement, Rollup, Transaction};

/// Which rollups a query adds up. Empty lists mean "all".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollupQuery {
    pub projects: Vec<ProjectId>,
    pub environments: Vec<EnvironmentId>,
    pub kind: Kind,
    /// Minutes in `[from, to)`.
    pub from: Timestamp,
    pub to: Timestamp,
}

#[async_trait]
pub trait RollupStore: Send + Sync {
    /// Counts the measurements of one transaction in their minute, each
    /// `weight` times.
    async fn add(
        &self,
        scope: Scope,
        minute: i64,
        measurements: &[Measurement],
        weight: u64,
    ) -> Result<(), PortError>;

    /// Every name of the query's kind, added up over its range.
    async fn names(&self, query: &RollupQuery) -> Result<Vec<(String, Rollup)>, PortError>;

    /// One name over time: (Unix seconds of the bucket's start, rollup) for
    /// every bucket of `bucket_seconds` that has any.
    async fn series(
        &self,
        query: &RollupQuery,
        name: &str,
        bucket_seconds: i64,
    ) -> Result<Vec<(i64, Rollup)>, PortError>;
}

/// A transaction as listed, without its spans.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionSummary {
    pub scope: Scope,
    pub trace_id: String,
    pub span_id: String,
    pub kind: Kind,
    pub name: String,
    pub start: Timestamp,
    pub duration_us: u64,
    pub status: Option<String>,
    pub http_status: Option<i64>,
    pub failed: bool,
    pub release: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
}

/// Which transactions to list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleQuery {
    pub projects: Vec<ProjectId>,
    pub environments: Vec<EnvironmentId>,
    pub kind: Kind,
    /// As counted: `GET /orders/{order}`.
    pub name: String,
    pub from: Timestamp,
    pub to: Timestamp,
    /// The slowest first, otherwise the newest.
    pub slowest: bool,
    pub limit: u32,
}

/// A transaction as kept, with its spans.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredTransaction {
    pub scope: Scope,
    pub transaction: Transaction,
}

#[async_trait]
pub trait TransactionStore: Send + Sync {
    async fn insert(
        &self,
        scope: Scope,
        transaction: &Transaction,
        received: Timestamp,
    ) -> Result<(), PortError>;

    async fn samples(&self, query: &SampleQuery) -> Result<Vec<TransactionSummary>, PortError>;

    /// The transactions of one trace, with their spans, oldest first.
    async fn by_trace(&self, trace_id: &str) -> Result<Vec<StoredTransaction>, PortError>;
}
