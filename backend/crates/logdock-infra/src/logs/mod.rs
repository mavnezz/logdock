//! The Logs context's adapters: the writer, the search over day segments,
//! the counts beside them, and the live feed.

mod store;
#[cfg(test)]
mod tests;
mod writer;

pub use store::{SegmentLogStore, SqliteLogStats};
pub use writer::{LogWriter, WriterConfig, WriterTask};

use logdock_kernel::{Day, EnvironmentId, ProjectId, Scope, Timestamp};
use logdock_logs::domain::{Level, LogEntry, LogId};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::json::fields_from_json;

/// An entry as read back from its segment.
fn entry(day: Day, row: &SqliteRow) -> LogEntry {
    LogEntry {
        id: Some(LogId { day, row: row.get("id") }),
        scope: Scope {
            project: ProjectId::new(row.get("project_id")),
            environment: EnvironmentId::new(row.get("environment_id")),
        },
        time: Timestamp::from_unix_micros(row.get("time")),
        received: Timestamp::from_unix_micros(row.get("received")),
        level: Level::from_number(row.get("level")).unwrap_or(Level::Info),
        message: row.get("message"),
        template: row.get("template"),
        attributes: fields_from_json(row.get("attributes")),
        trace_id: row.get("trace_id"),
        span_id: row.get("span_id"),
        service: row.get("service"),
        host: row.get("host"),
        container: row.get("container"),
        release: row.get("release"),
        user_id: row.get("user_id"),
        redactions: row.get::<i64, _>("redactions") as u32,
        truncated: row.get("truncated"),
    }
}
