//! Unified SQLite database for Starpod's transactional data.
//!
//! All transactional data (sessions, cron scheduling, authentication) lives in
//! a single `core.db` file. A shared connection pool (WAL mode, foreign keys
//! enabled) serves all three domains.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────┐
//! │  CoreDb   │  owns SqlitePool (max 2 conns, WAL, FK ON)
//! └────┬─────┘
//!      │ pool.clone()
//!      ├──────────────► SessionManager::from_pool(pool)
//!      ├──────────────► CronStore::from_pool(pool)
//!      └──────────────► AuthStore::from_pool(pool)
//! ```
//!
//! # Databases kept separate
//!
//! - **memory.db** — FTS5 + vector blobs, bulk reindex I/O, different access pattern
//! - **vault.db** — AES-256-GCM encrypted, optional (needs `.vault_key`), isolated security boundary
//!
//! # Usage
//!
//! ```no_run
//! # async fn example() -> starpod_core::Result<()> {
//! use starpod_db::CoreDb;
//!
//! let db = CoreDb::new(std::path::Path::new(".starpod/db")).await?;
//! // Pass db.pool().clone() to SessionManager, CronStore, AuthStore
//! # Ok(())
//! # }
//! ```

pub mod connectors;

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tracing::{debug, info};

use starpod_core::{Result, StarpodError};

/// Unified database for sessions, cron, and auth.
///
/// Owns a single `SqlitePool` backed by `core.db`. Individual stores
/// (`SessionManager`, `CronStore`, `AuthStore`) receive a clone of the
/// pool via `from_pool()` instead of opening their own connections.
///
/// The pool is configured with:
/// - **WAL journal mode** — concurrent readers don't block writers
/// - **Foreign keys ON** — referential integrity across all tables
/// - **2 max connections** — one writer + one reader; SQLite serialises
///   writes anyway, so more connections just waste memory (~2 MB page
///   cache each) and cause lock contention on small VMs
/// - **`busy_timeout = 5000`** — wait up to 5 s for a lock instead of
///   returning SQLITE_BUSY immediately
/// - **`synchronous = NORMAL`** — safe with WAL, avoids fsync per commit
/// - **`cache_size = -2000`** — 2 MB page cache per connection (default)
pub struct CoreDb {
    pool: SqlitePool,
}

impl CoreDb {
    /// Open (or create) `core.db` inside `db_dir`.
    ///
    /// Runs all migrations from `./migrations`. If migrations fail due to a
    /// checksum mismatch or a removed migration (common during development
    /// when migration files are edited in-place), the database is deleted
    /// and recreated from scratch.
    pub async fn new(db_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(db_dir)?;

        let db_path = db_dir.join("core.db");

        // Try to open and migrate; on schema mismatch, recreate from scratch.
        match Self::open_and_migrate(&db_path).await {
            Ok(pool) => {
                debug!("core.db ready at {}", db_path.display());
                Ok(Self { pool })
            }
            Err(e) => {
                let msg = e.to_string();
                let is_schema_mismatch = msg.contains("previously applied but is missing")
                    || msg.contains("checksum mismatch");

                if !is_schema_mismatch {
                    return Err(e);
                }

                info!("Migration schema changed — recreating core.db");
                // Remove db + WAL/SHM files
                let db_str = db_path.display().to_string();
                let _ = std::fs::remove_file(&db_path);
                let _ = std::fs::remove_file(format!("{db_str}-wal"));
                let _ = std::fs::remove_file(format!("{db_str}-shm"));

                let pool = Self::open_and_migrate(&db_path).await?;
                debug!("core.db recreated at {}", db_path.display());

                Ok(Self { pool })
            }
        }
    }

    /// Open (or create) the database file and run migrations.
    async fn open_and_migrate(db_path: &Path) -> Result<SqlitePool> {
        let opts =
            SqliteConnectOptions::from_str(&format!("sqlite://{}?mode=rwc", db_path.display()))
                .map_err(|e| StarpodError::Database(format!("Invalid DB path: {}", e)))?
                .pragma("journal_mode", "WAL")
                .pragma("foreign_keys", "ON")
                .pragma("busy_timeout", "5000")
                .pragma("synchronous", "NORMAL");

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to open core db: {}", e)))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Core migration failed: {}", e)))?;

        Ok(pool)
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

    // ── Basic lifecycle ─────────────────────────────────────────────

    #[tokio::test]
    async fn in_memory_creates_all_tables() {
        let db = CoreDb::in_memory().await.unwrap();
        let pool = db.pool();

        // Auth tables
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_keys")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM telegram_links")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM auth_audit_log")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        // Session tables
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM session_metadata")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM session_messages")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM usage_stats")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM compaction_log")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        // Cron tables
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cron_jobs")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cron_runs")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn on_disk_creates_core_db() {
        let tmp = tempfile::tempdir().unwrap();
        let db = CoreDb::new(tmp.path()).await.unwrap();

        assert!(tmp.path().join("core.db").exists());

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn on_disk_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("deep").join("nested").join("db");
        let db = CoreDb::new(&nested).await.unwrap();

        assert!(nested.join("core.db").exists());
        drop(db);
    }

    #[tokio::test]
    async fn reopen_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();

        // First open — creates the DB
        let db1 = CoreDb::new(tmp.path()).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, display_name, role, is_active, created_at, updated_at) \
             VALUES ('u1', 'a@b.com', 'A', 'admin', 1, '2024-01-01', '2024-01-01')",
        )
        .execute(db1.pool())
        .await
        .unwrap();
        drop(db1);

        // Second open — should find existing data, not recreate
        let db2 = CoreDb::new(tmp.path()).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT email FROM users WHERE id = 'u1'")
            .fetch_one(db2.pool())
            .await
            .unwrap();
        assert_eq!(row.0, "a@b.com");
    }

    // ── Foreign key enforcement ─────────────────────────────────────

    #[tokio::test]
    async fn fk_rejects_invalid_api_key_user() {
        let db = CoreDb::in_memory().await.unwrap();

        let result = sqlx::query(
            "INSERT INTO api_keys (id, user_id, prefix, key_hash, created_at) \
             VALUES ('k1', 'nonexistent', 'sp_', 'hash', '2024-01-01')",
        )
        .execute(db.pool())
        .await;

        assert!(
            result.is_err(),
            "FK should reject api_key with invalid user_id"
        );
    }

    #[tokio::test]
    async fn fk_rejects_invalid_telegram_link_user() {
        let db = CoreDb::in_memory().await.unwrap();

        let result = sqlx::query(
            "INSERT INTO telegram_links (telegram_id, user_id, username, linked_at) \
             VALUES (123, 'nonexistent', 'bob', '2024-01-01')",
        )
        .execute(db.pool())
        .await;

        assert!(
            result.is_err(),
            "FK should reject telegram_link with invalid user_id"
        );
    }

    #[tokio::test]
    async fn fk_rejects_invalid_session_message() {
        let db = CoreDb::in_memory().await.unwrap();

        let result = sqlx::query(
            "INSERT INTO session_messages (session_id, role, content, timestamp) \
             VALUES ('nonexistent', 'user', 'hello', '2024-01-01')",
        )
        .execute(db.pool())
        .await;

        assert!(
            result.is_err(),
            "FK should reject message with invalid session_id"
        );
    }

    #[tokio::test]
    async fn fk_rejects_invalid_cron_run_job() {
        let db = CoreDb::in_memory().await.unwrap();

        let result = sqlx::query(
            "INSERT INTO cron_runs (id, job_id, started_at, status) \
             VALUES ('r1', 'nonexistent', 1000, 'pending')",
        )
        .execute(db.pool())
        .await;

        assert!(
            result.is_err(),
            "FK should reject cron_run with invalid job_id"
        );
    }

    // ── CASCADE deletes ─────────────────────────────────────────────

    #[tokio::test]
    async fn cascade_delete_user_removes_api_keys() {
        let db = CoreDb::in_memory().await.unwrap();
        let pool = db.pool();

        sqlx::query(
            "INSERT INTO users (id, email, role, is_active, created_at, updated_at) \
             VALUES ('u1', 'a@b.com', 'admin', 1, '2024-01-01', '2024-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO api_keys (id, user_id, prefix, key_hash, created_at) \
             VALUES ('k1', 'u1', 'sp_', 'hash1', '2024-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO api_keys (id, user_id, prefix, key_hash, created_at) \
             VALUES ('k2', 'u1', 'sp_', 'hash2', '2024-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();

        // Delete user
        sqlx::query("DELETE FROM users WHERE id = 'u1'")
            .execute(pool)
            .await
            .unwrap();

        // API keys should be gone
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_keys")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn cascade_delete_user_removes_telegram_links() {
        let db = CoreDb::in_memory().await.unwrap();
        let pool = db.pool();

        sqlx::query(
            "INSERT INTO users (id, role, is_active, created_at, updated_at) \
             VALUES ('u1', 'admin', 1, '2024-01-01', '2024-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO telegram_links (telegram_id, user_id, username, linked_at) \
             VALUES (999, 'u1', 'bob', '2024-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM users WHERE id = 'u1'")
            .execute(pool)
            .await
            .unwrap();

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM telegram_links")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn cascade_delete_session_removes_messages_and_compaction() {
        let db = CoreDb::in_memory().await.unwrap();
        let pool = db.pool();

        sqlx::query(
            "INSERT INTO session_metadata (id, created_at, last_message_at) \
             VALUES ('s1', '2024-01-01', '2024-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO session_messages (session_id, role, content, timestamp) \
             VALUES ('s1', 'user', 'hi', '2024-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO compaction_log (session_id, timestamp, trigger, pre_tokens, summary) \
             VALUES ('s1', '2024-01-01', 'auto', 1000, 'summary')",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM session_metadata WHERE id = 's1'")
            .execute(pool)
            .await
            .unwrap();

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM session_messages")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM compaction_log")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn cascade_delete_cron_job_removes_runs() {
        let db = CoreDb::in_memory().await.unwrap();
        let pool = db.pool();

        sqlx::query(
            "INSERT INTO cron_jobs (id, name, prompt, schedule_type, schedule_value, created_at) \
             VALUES ('j1', 'test', 'do stuff', 'interval', '60000', 1000)",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO cron_runs (id, job_id, started_at, status) \
             VALUES ('r1', 'j1', 2000, 'success')",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM cron_jobs WHERE id = 'j1'")
            .execute(pool)
            .await
            .unwrap();

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cron_runs")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }

    // ── Cross-domain queries (the whole point of consolidation) ─────

    #[tokio::test]
    async fn cross_domain_join_sessions_with_usage_by_user() {
        let db = CoreDb::in_memory().await.unwrap();
        let pool = db.pool();

        // Create a user
        sqlx::query(
            "INSERT INTO users (id, email, role, is_active, created_at, updated_at) \
             VALUES ('u1', 'alice@test.com', 'admin', 1, '2024-01-01', '2024-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();

        // Create sessions for this user
        sqlx::query(
            "INSERT INTO session_metadata (id, created_at, last_message_at, user_id) \
             VALUES ('s1', '2024-01-01', '2024-01-01', 'u1')",
        )
        .execute(pool)
        .await
        .unwrap();

        // Record usage
        sqlx::query(
            "INSERT INTO usage_stats (session_id, turn, input_tokens, output_tokens, cost_usd, timestamp, user_id) \
             VALUES ('s1', 1, 100, 200, 0.01, '2024-01-01', 'u1')"
        ).execute(pool).await.unwrap();

        // Cross-domain query: total cost per user (joins users + usage_stats)
        let row: (String, f64) = sqlx::query_as(
            "SELECT u.email, SUM(us.cost_usd) as total_cost \
             FROM users u \
             JOIN usage_stats us ON us.user_id = u.id \
             GROUP BY u.id",
        )
        .fetch_one(pool)
        .await
        .unwrap();

        assert_eq!(row.0, "alice@test.com");
        assert!((row.1 - 0.01).abs() < 0.001);
    }

    #[tokio::test]
    async fn pool_clone_shares_state() {
        let db = CoreDb::in_memory().await.unwrap();

        // Insert on original pool
        sqlx::query(
            "INSERT INTO users (id, role, is_active, created_at, updated_at) \
             VALUES ('u1', 'admin', 1, '2024-01-01', '2024-01-01')",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Read from cloned pool (simulates what stores do)
        let pool2 = db.pool().clone();
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&pool2)
            .await
            .unwrap();
        assert_eq!(row.0, 1);
    }
}
