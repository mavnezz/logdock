use async_trait::async_trait;
use logdock_identity::application::ports::ApiTokenRepository;
use logdock_identity::domain::{
    AccountId, ApiToken, ApiTokenId, NewApiToken, TokenHash, TokenName, TokenScope,
};
use logdock_kernel::{PortError, Timestamp};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::Database;

/// [`ApiTokenRepository`] on the `api_tokens` table.
#[derive(Debug, Clone)]
pub struct SqliteApiTokens {
    db: Database,
}

impl SqliteApiTokens {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("api tokens", error)
}

fn token(row: &SqliteRow) -> Result<ApiToken, PortError> {
    let name: String = row.get("name");
    let scope: String = row.get("scope");
    Ok(ApiToken {
        id: ApiTokenId::new(row.get("id")),
        account_id: AccountId::new(row.get("account_id")),
        name: TokenName::parse(&name).map_err(port)?,
        scope: TokenScope::parse(&scope).ok_or_else(|| port(format!("unknown token scope {scope:?}")))?,
        prefix: row.get("prefix"),
        created_at: Timestamp::from_unix_micros(row.get("created_at")),
        last_used_at: row.get::<Option<i64>, _>("last_used_at").map(Timestamp::from_unix_micros),
        revoked_at: row.get::<Option<i64>, _>("revoked_at").map(Timestamp::from_unix_micros),
    })
}

#[async_trait]
impl ApiTokenRepository for SqliteApiTokens {
    async fn create(&self, new: NewApiToken) -> Result<ApiToken, PortError> {
        let row = sqlx::query(
            "INSERT INTO api_tokens (account_id, name, scope, prefix, hash, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(new.account_id.get())
        .bind(new.name.as_str())
        .bind(new.scope.as_str())
        .bind(&new.prefix)
        .bind(new.hash.as_str())
        .bind(new.created_at.unix_micros())
        .fetch_one(self.db.pool())
        .await
        .map_err(port)?;
        token(&row)
    }

    async fn list(&self, account: AccountId) -> Result<Vec<ApiToken>, PortError> {
        let rows = sqlx::query("SELECT * FROM api_tokens WHERE account_id = ? ORDER BY id DESC")
            .bind(account.get())
            .fetch_all(self.db.pool())
            .await
            .map_err(port)?;
        rows.iter().map(token).collect()
    }

    async fn find_by_hash(&self, hash: &TokenHash) -> Result<Option<ApiToken>, PortError> {
        let row = sqlx::query("SELECT * FROM api_tokens WHERE hash = ?")
            .bind(hash.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(port)?;
        row.as_ref().map(token).transpose()
    }

    async fn revoke(&self, id: ApiTokenId, at: Timestamp) -> Result<(), PortError> {
        sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(at.unix_micros())
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn touch(&self, id: ApiTokenId, at: Timestamp) -> Result<(), PortError> {
        sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
            .bind(at.unix_micros())
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }
}
