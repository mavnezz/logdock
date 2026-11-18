use async_trait::async_trait;
use logdock_identity::application::ports::SessionRepository;
use logdock_identity::domain::{AccountId, NewSession, Session, SessionId, TokenHash};
use logdock_kernel::{PortError, Secret, Timestamp};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::Database;

/// [`SessionRepository`] on the `sessions` table.
#[derive(Debug, Clone)]
pub struct SqliteSessions {
    db: Database,
}

impl SqliteSessions {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn port(error: sqlx::Error) -> PortError {
    PortError::new("sessions", error)
}

fn session(row: &SqliteRow) -> Session {
    let ip: Option<String> = row.get("client_ip");
    Session {
        id: SessionId::new(row.get("id")),
        account_id: AccountId::new(row.get("account_id")),
        token_hash: TokenHash::new(row.get("token_hash")),
        csrf_token: Secret::new(row.get::<String, _>("csrf_token")),
        created_at: Timestamp::from_unix_micros(row.get("created_at")),
        last_seen_at: Timestamp::from_unix_micros(row.get("last_seen_at")),
        client_ip: ip.and_then(|ip| ip.parse().ok()),
        user_agent: row.get("user_agent"),
    }
}

#[async_trait]
impl SessionRepository for SqliteSessions {
    async fn create(&self, new: NewSession) -> Result<Session, PortError> {
        let row = sqlx::query(
            "INSERT INTO sessions (account_id, token_hash, csrf_token, created_at, last_seen_at, client_ip, user_agent) \
             VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(new.account_id.get())
        .bind(new.token_hash.as_str())
        .bind(new.csrf_token.expose())
        .bind(new.created_at.unix_micros())
        .bind(new.created_at.unix_micros())
        .bind(new.client_ip.map(|ip| ip.to_string()))
        .bind(new.user_agent)
        .fetch_one(self.db.pool())
        .await
        .map_err(port)?;
        Ok(session(&row))
    }

    async fn find_by_token(&self, hash: &TokenHash) -> Result<Option<Session>, PortError> {
        let row = sqlx::query("SELECT * FROM sessions WHERE token_hash = ?")
            .bind(hash.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(port)?;
        Ok(row.as_ref().map(session))
    }

    async fn touch(&self, id: SessionId, at: Timestamp) -> Result<(), PortError> {
        sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE id = ?")
            .bind(at.unix_micros())
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn delete(&self, id: SessionId) -> Result<(), PortError> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn delete_for_account(
        &self,
        account: AccountId,
        except: Option<SessionId>,
    ) -> Result<(), PortError> {
        sqlx::query("DELETE FROM sessions WHERE account_id = ? AND id IS NOT ?")
            .bind(account.get())
            .bind(except.map(SessionId::get))
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn delete_expired(
        &self,
        idle_before: Timestamp,
        created_before: Timestamp,
    ) -> Result<u64, PortError> {
        let done = sqlx::query("DELETE FROM sessions WHERE last_seen_at < ? OR created_at < ?")
            .bind(idle_before.unix_micros())
            .bind(created_before.unix_micros())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(done.rows_affected())
    }
}
