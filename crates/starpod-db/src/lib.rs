mod migrate;

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tracing::{debug, info};

use starpod_core::{StarpodError, Result};

/// Unified database for sessions, cron, and auth.
///
/// Owns a single `SqlitePool` backed by `core.db`. Individual stores
/// (`SessionManager`, `CronStore`, `AuthStore`) receive a clone of the
/// pool via `from_pool()` instead of opening their own connections.
pub struct CoreDb {
    pool: SqlitePool,
}

impl CoreDb {
    /// Open (or create) `core.db` inside `db_dir`.
    ///
    /// Runs all migrations, then checks for legacy database files
    /// (`session.db`, `cron.db`, `users.db`) and migrates their data
    /// into the unified database if found.
    pub async fn new(db_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(db_dir)?;

        let db_path = db_dir.join("core.db");
        let opts = SqliteConnectOptions::from_str(
            &format!("sqlite://{}?mode=rwc", db_path.display()),
        )
        .map_err(|e| StarpodError::Database(format!("Invalid DB path: {}", e)))?
        .pragma("journal_mode", "WAL")
        .pragma("foreign_keys", "ON");

        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(opts)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to open core db: {}", e)))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Core migration failed: {}", e)))?;

        debug!("core.db ready at {}", db_path.display());

        // Migrate legacy databases if present
        if migrate::has_legacy_dbs(db_dir) {
            info!("Legacy database files detected — migrating to core.db");
            migrate::migrate_legacy_dbs(&pool, db_dir).await?;
        }

        Ok(Self { pool })
    }

    /// Create an in-memory `CoreDb` for testing.
    ///
    /// Runs all migrations on a shared in-memory database. Each call
    /// returns a fresh, empty database.
    pub async fn in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(|e| StarpodError::Database(format!("Invalid memory DB: {}", e)))?
            .pragma("foreign_keys", "ON");

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to open in-memory db: {}", e)))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Core migration failed: {}", e)))?;

        Ok(Self { pool })
    }

    /// Get a reference to the shared connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_creates_all_tables() {
        let db = CoreDb::in_memory().await.unwrap();
        let pool = db.pool();

        // Verify auth tables
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(pool).await.unwrap();
        assert_eq!(row.0, 0);

        // Verify session tables
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM session_metadata")
            .fetch_one(pool).await.unwrap();
        assert_eq!(row.0, 0);

        // Verify cron tables
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cron_jobs")
            .fetch_one(pool).await.unwrap();
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn foreign_keys_enforced() {
        let db = CoreDb::in_memory().await.unwrap();
        let pool = db.pool();

        // Inserting an api_key with a non-existent user_id should fail
        let result = sqlx::query(
            "INSERT INTO api_keys (id, user_id, prefix, key_hash, created_at) \
             VALUES ('k1', 'nonexistent', 'sp_', 'hash', '2024-01-01')"
        )
        .execute(pool)
        .await;

        assert!(result.is_err(), "FK constraint should reject invalid user_id");
    }

    #[tokio::test]
    async fn on_disk_creates_core_db() {
        let tmp = tempfile::tempdir().unwrap();
        let db = CoreDb::new(tmp.path()).await.unwrap();

        // Verify the file was created
        assert!(tmp.path().join("core.db").exists());

        // Verify tables work
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(db.pool()).await.unwrap();
        assert_eq!(row.0, 0);
    }
}
