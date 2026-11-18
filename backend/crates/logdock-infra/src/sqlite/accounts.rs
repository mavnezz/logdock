use async_trait::async_trait;
use logdock_identity::application::ports::AccountRepository;
use logdock_identity::domain::{
    Account, AccountId, DisplayName, Email, Locale, NewAccount, PasswordHash, Role,
};
use logdock_kernel::{PortError, Timestamp};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::Database;

/// [`AccountRepository`] on the `accounts` table.
#[derive(Debug, Clone)]
pub struct SqliteAccounts {
    db: Database,
}

impl SqliteAccounts {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("accounts", error)
}

macro_rules! select {
    ($rest:literal) => {
        concat!("SELECT id, email, name, role, locale, password_hash, disabled, created_at, updated_at FROM accounts ", $rest)
    };
}

fn account(row: &SqliteRow) -> Result<Account, PortError> {
    let role: String = row.get("role");
    let locale: String = row.get("locale");
    Ok(Account {
        id: AccountId::new(row.get("id")),
        email: Email::parse(row.get("email")).map_err(port)?,
        name: DisplayName::parse(row.get("name")).map_err(port)?,
        role: Role::parse(&role).ok_or_else(|| port(format!("unknown role {role:?}")))?,
        locale: Locale::parse(&locale).unwrap_or_default(),
        password_hash: PasswordHash::new(row.get("password_hash")),
        disabled: row.get("disabled"),
        created_at: Timestamp::from_unix_micros(row.get("created_at")),
        updated_at: Timestamp::from_unix_micros(row.get("updated_at")),
    })
}

#[async_trait]
impl AccountRepository for SqliteAccounts {
    async fn count(&self) -> Result<u64, PortError> {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
            .fetch_one(self.db.pool())
            .await
            .map_err(port)?;
        Ok(count as u64)
    }

    async fn find_by_id(&self, id: AccountId) -> Result<Option<Account>, PortError> {
        let row = sqlx::query(select!("WHERE id = ?"))
            .bind(id.get())
            .fetch_optional(self.db.pool())
            .await
            .map_err(port)?;
        row.as_ref().map(account).transpose()
    }

    async fn find_by_email(&self, email: &Email) -> Result<Option<Account>, PortError> {
        let row = sqlx::query(select!("WHERE email = ?"))
            .bind(email.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(port)?;
        row.as_ref().map(account).transpose()
    }

    async fn list(&self) -> Result<Vec<Account>, PortError> {
        let rows = sqlx::query(select!("ORDER BY email")).fetch_all(self.db.pool()).await.map_err(port)?;
        rows.iter().map(account).collect()
    }

    async fn create(&self, new: NewAccount) -> Result<Option<Account>, PortError> {
        let id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO accounts (email, name, role, locale, password_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (email) DO NOTHING RETURNING id",
        )
        .bind(new.email.as_str())
        .bind(new.name.as_str())
        .bind(new.role.as_str())
        .bind(new.locale.as_str())
        .bind(new.password_hash.as_str())
        .bind(new.created_at.unix_micros())
        .bind(new.created_at.unix_micros())
        .fetch_optional(self.db.pool())
        .await
        .map_err(port)?;
        match id {
            Some(id) => self.find_by_id(AccountId::new(id)).await,
            None => Ok(None),
        }
    }

    async fn create_first(&self, new: NewAccount) -> Result<Option<Account>, PortError> {
        // One statement: two setups at once cannot both find no account.
        let id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO accounts (email, name, role, locale, password_hash, created_at, updated_at) \
             SELECT ?, ?, ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM accounts) RETURNING id",
        )
        .bind(new.email.as_str())
        .bind(new.name.as_str())
        .bind(new.role.as_str())
        .bind(new.locale.as_str())
        .bind(new.password_hash.as_str())
        .bind(new.created_at.unix_micros())
        .bind(new.created_at.unix_micros())
        .fetch_optional(self.db.pool())
        .await
        .map_err(port)?;
        match id {
            Some(id) => self.find_by_id(AccountId::new(id)).await,
            None => Ok(None),
        }
    }

    async fn update(&self, account: &Account) -> Result<(), PortError> {
        sqlx::query(
            "UPDATE accounts SET name = ?, role = ?, locale = ?, disabled = ?, updated_at = ? WHERE id = ?",
        )
        .bind(account.name.as_str())
        .bind(account.role.as_str())
        .bind(account.locale.as_str())
        .bind(account.disabled)
        .bind(account.updated_at.unix_micros())
        .bind(account.id.get())
        .execute(self.db.pool())
        .await
        .map_err(port)?;
        Ok(())
    }

    async fn update_password(
        &self,
        id: AccountId,
        hash: &PasswordHash,
        at: Timestamp,
    ) -> Result<(), PortError> {
        sqlx::query("UPDATE accounts SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(hash.as_str())
            .bind(at.unix_micros())
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }

    async fn delete(&self, id: AccountId) -> Result<(), PortError> {
        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(id.get())
            .execute(self.db.pool())
            .await
            .map_err(port)?;
        Ok(())
    }
}
