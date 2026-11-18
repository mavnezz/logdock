use async_trait::async_trait;
use logdock_identity::application::ports::AuditReader;
use logdock_kernel::{AuditEntry, AuditLog, Outcome, PortError, Timestamp};
use sqlx::Row;

use super::Database;

/// [`AuditLog`] and [`AuditReader`] on the `audit_log` table.
#[derive(Debug, Clone)]
pub struct SqliteAudit {
    db: Database,
}

impl SqliteAudit {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn port(error: sqlx::Error) -> PortError {
    PortError::new("audit log", error)
}

#[async_trait]
impl AuditLog for SqliteAudit {
    async fn record(&self, entry: AuditEntry) -> Result<(), PortError> {
        sqlx::query(
            "INSERT INTO audit_log (at, action, outcome, actor_id, actor_name, target, reason, client_ip, user_agent) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(entry.at.unix_micros())
        .bind(&entry.action)
        .bind(entry.outcome.as_str())
        .bind(entry.actor_id)
        .bind(&entry.actor_name)
        .bind(&entry.target)
        .bind(&entry.reason)
        .bind(entry.client_ip.map(|ip| ip.to_string()))
        .bind(&entry.user_agent)
        .execute(self.db.pool())
        .await
        .map_err(port)?;
        Ok(())
    }
}

#[async_trait]
impl AuditReader for SqliteAudit {
    async fn list(&self, before_id: Option<i64>, limit: u32) -> Result<Vec<(i64, AuditEntry)>, PortError> {
        let rows = sqlx::query("SELECT * FROM audit_log WHERE id < ? ORDER BY id DESC LIMIT ?")
            .bind(before_id.unwrap_or(i64::MAX))
            .bind(i64::from(limit))
            .fetch_all(self.db.pool())
            .await
            .map_err(port)?;
        Ok(rows
            .iter()
            .map(|row| {
                let ip: Option<String> = row.get("client_ip");
                let outcome: String = row.get("outcome");
                let entry = AuditEntry {
                    at: Timestamp::from_unix_micros(row.get("at")),
                    action: row.get("action"),
                    outcome: Outcome::parse(&outcome).unwrap_or(Outcome::Failure),
                    actor_id: row.get("actor_id"),
                    actor_name: row.get("actor_name"),
                    target: row.get("target"),
                    reason: row.get("reason"),
                    client_ip: ip.and_then(|ip| ip.parse().ok()),
                    user_agent: row.get("user_agent"),
                };
                (row.get("id"), entry)
            })
            .collect())
    }
}
