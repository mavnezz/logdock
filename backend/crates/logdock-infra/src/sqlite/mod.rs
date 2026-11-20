//! SQLite: the main database (`logdock.db`) behind the repositories of all
//! contexts. Opening it also brings its schema up to date; a database
//! written by a newer release is refused instead of being guessed at.

mod accounts;
mod api_tokens;
mod audit;
mod ingest_counts;
mod keys;
mod projects;
#[cfg(test)]
pub(crate) mod repository_tests;
mod sessions;

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::migrate::{MigrateError, Migrator};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

pub use accounts::SqliteAccounts;
pub use api_tokens::SqliteApiTokens;
pub use audit::SqliteAudit;
pub use ingest_counts::SqliteIngestCounter;
pub use keys::{RandomKeys, SqliteKeys};
pub use projects::SqliteProjects;
pub use sessions::SqliteSessions;

static MIGRATOR: Migrator = sqlx::migrate!();

/// Connections in the pool. SQLite has one writer at a time anyway; reads
/// of the interface run beside it in WAL mode.
const MAX_CONNECTIONS: u32 = 8;

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("cannot create {path}")]
    Create {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot open {path}")]
    Open {
        path: PathBuf,
        #[source]
        source: sqlx::Error,
    },
    #[error(
        "the database schema (version {found}) is newer than this release supports (version {supported}); \
         refusing to start to protect it"
    )]
    TooNew { found: i64, supported: i64 },
    #[error("cannot read the schema version")]
    Inspect(#[source] sqlx::Error),
    #[error("database migration failed")]
    Migrate(#[source] MigrateError),
}

/// What [`Database::open`] did, for the startup log.
#[derive(Debug, PartialEq, Eq)]
pub struct OpenReport {
    pub created: bool,
    pub schema_before: Option<i64>,
    pub schema_after: i64,
}

/// The main database, ready for the repositories. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Database {
    pool: SqlitePool,
    path: PathBuf,
}

impl Database {
    /// Opens the database at `path`, creating it and its directory if
    /// needed, and applies all pending migrations.
    pub async fn open(path: &Path) -> Result<(Self, OpenReport), DatabaseError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|source| DatabaseError::Create { path: dir.to_owned(), source })?;
        }
        let created = !path.exists();
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await
            .map_err(|source| DatabaseError::Open { path: path.to_owned(), source })?;
        let database = Self { pool, path: path.to_owned() };
        match database.migrate().await {
            Ok((schema_before, schema_after)) => {
                Ok((database, OpenReport { created, schema_before, schema_after }))
            }
            Err(error) => {
                database.pool.close().await;
                Err(error)
            }
        }
    }

    async fn migrate(&self) -> Result<(Option<i64>, i64), DatabaseError> {
        let before = self.schema_version().await?;
        let supported = Self::supported_schema();
        if let Some(found) = before
            && found > supported
        {
            return Err(DatabaseError::TooNew { found, supported });
        }
        MIGRATOR.run(&self.pool).await.map_err(DatabaseError::Migrate)?;
        Ok((before, self.schema_version().await?.unwrap_or(0)))
    }

    async fn schema_version(&self) -> Result<Option<i64>, DatabaseError> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(DatabaseError::Inspect)?;
        if !exists {
            return Ok(None);
        }
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&self.pool)
            .await
            .map_err(DatabaseError::Inspect)
    }

    /// The newest migration this release knows.
    pub fn supported_schema() -> i64 {
        MIGRATOR.iter().filter(|m| m.migration_type.is_up_migration()).map(|m| m.version).max().unwrap_or(0)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// A transaction that takes the write lock at its start. A deferred one
    /// that reads before it writes fails at once with `SQLITE_BUSY` when
    /// another connection wrote meanwhile (WAL mode does not wait then);
    /// this one waits for the lock as long as the busy timeout allows.
    pub async fn write(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
        self.pool.begin_with("BEGIN IMMEDIATE").await
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the database answers a query.
    pub async fn ping(&self) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(&self.pool).await.is_ok()
    }

    /// Writes the WAL back into the database file, for a clean shutdown.
    pub async fn close(&self) {
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&self.pool).await;
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_and_reopens_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data/logdock.db");
        let (db, report) = Database::open(&path).await.unwrap();
        assert!(report.created);
        assert_eq!(report.schema_before, None);
        assert_eq!(report.schema_after, Database::supported_schema());
        assert!(db.ping().await);
        assert_eq!(db.path(), path);
        db.close().await;

        let (db, report) = Database::open(&path).await.unwrap();
        assert!(!report.created);
        assert_eq!(report.schema_before, Some(Database::supported_schema()));
        db.close().await;
    }

    #[tokio::test]
    async fn refuses_a_database_of_a_newer_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logdock.db");
        let (db, _) = Database::open(&path).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
             VALUES (99999, 'future', 1, x'00', 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        db.close().await;
        let error = Database::open(&path).await.unwrap_err();
        assert!(matches!(error, DatabaseError::TooNew { found: 99999, .. }), "{error}");
    }
}
