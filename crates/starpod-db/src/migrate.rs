use std::path::Path;

use sqlx::SqlitePool;
use tracing::{info, warn};

use starpod_core::{StarpodError, Result};

/// Check whether any legacy database files exist in `db_dir`.
pub fn has_legacy_dbs(db_dir: &Path) -> bool {
    db_dir.join("users.db").exists()
        || db_dir.join("session.db").exists()
        || db_dir.join("cron.db").exists()
}

/// Migrate data from legacy database files into the unified core.db.
///
/// Order matters: users first (FK target), then sessions, then cron.
/// After successful migration, legacy files are renamed to `*.db.migrated`.
pub async fn migrate_legacy_dbs(pool: &SqlitePool, db_dir: &Path) -> Result<()> {
    let users_db = db_dir.join("users.db");
    let session_db = db_dir.join("session.db");
    let cron_db = db_dir.join("cron.db");

    // 1. Migrate users.db (must be first — other tables reference users)
    if users_db.exists() {
        migrate_users(pool, &users_db).await?;
        rename_legacy(&users_db)?;
        info!("Migrated users.db → core.db");
    }

    // 2. Migrate session.db
    if session_db.exists() {
        migrate_sessions(pool, &session_db).await?;
        rename_legacy(&session_db)?;
        info!("Migrated session.db → core.db");
    }

    // 3. Migrate cron.db
    if cron_db.exists() {
        migrate_cron(pool, &cron_db).await?;
        rename_legacy(&cron_db)?;
        info!("Migrated cron.db → core.db");
    }

    Ok(())
}

fn rename_legacy(path: &Path) -> Result<()> {
    let mut dest = path.as_os_str().to_os_string();
    dest.push(".migrated");
    std::fs::rename(path, &dest).map_err(|e| {
        StarpodError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to rename {}: {}", path.display(), e),
        ))
    })?;
    // Also rename WAL/SHM files if they exist
    for suffix in &["-wal", "-shm"] {
        let wal = path.with_extension(format!("db{}", suffix));
        if wal.exists() {
            let mut wal_dest = wal.as_os_str().to_os_string();
            wal_dest.push(".migrated");
            let _ = std::fs::rename(&wal, &wal_dest);
        }
    }
    Ok(())
}

async fn migrate_users(pool: &SqlitePool, legacy_path: &Path) -> Result<()> {
    let path_str = legacy_path.display().to_string();

    sqlx::query(&format!("ATTACH DATABASE '{}' AS legacy", path_str))
        .execute(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Failed to attach users.db: {}", e)))?;

    // Users
    let result = sqlx::query(
        "INSERT OR IGNORE INTO users (id, email, display_name, role, is_active, created_at, updated_at, filesystem_enabled) \
         SELECT id, email, display_name, role, is_active, created_at, updated_at, filesystem_enabled \
         FROM legacy.users"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate users table (may not exist in legacy): {}", e);
    }

    // API keys
    let result = sqlx::query(
        "INSERT OR IGNORE INTO api_keys (id, user_id, prefix, key_hash, label, expires_at, revoked_at, last_used_at, created_at) \
         SELECT id, user_id, prefix, key_hash, label, expires_at, revoked_at, last_used_at, created_at \
         FROM legacy.api_keys"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate api_keys table: {}", e);
    }

    // Telegram links
    let result = sqlx::query(
        "INSERT OR IGNORE INTO telegram_links (telegram_id, user_id, username, linked_at) \
         SELECT telegram_id, user_id, username, linked_at \
         FROM legacy.telegram_links"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate telegram_links table: {}", e);
    }

    // Audit log
    let result = sqlx::query(
        "INSERT OR IGNORE INTO auth_audit_log (id, user_id, event_type, detail, ip_address, created_at) \
         SELECT id, user_id, event_type, detail, ip_address, created_at \
         FROM legacy.auth_audit_log"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate auth_audit_log table: {}", e);
    }

    sqlx::query("DETACH DATABASE legacy")
        .execute(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Failed to detach users.db: {}", e)))?;

    Ok(())
}

async fn migrate_sessions(pool: &SqlitePool, legacy_path: &Path) -> Result<()> {
    let path_str = legacy_path.display().to_string();

    sqlx::query(&format!("ATTACH DATABASE '{}' AS legacy", path_str))
        .execute(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Failed to attach session.db: {}", e)))?;

    // Session metadata
    let result = sqlx::query(
        "INSERT OR IGNORE INTO session_metadata \
         (id, created_at, last_message_at, is_closed, summary, message_count, channel, channel_session_key, title, user_id, is_read, triggered_by) \
         SELECT id, created_at, last_message_at, is_closed, summary, message_count, \
                COALESCE(channel, 'main'), channel_session_key, title, \
                COALESCE(user_id, 'admin'), COALESCE(is_read, 1), triggered_by \
         FROM legacy.session_metadata"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate session_metadata: {}", e);
    }

    // Session messages
    let result = sqlx::query(
        "INSERT OR IGNORE INTO session_messages (id, session_id, role, content, timestamp) \
         SELECT id, session_id, role, content, timestamp \
         FROM legacy.session_messages"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate session_messages: {}", e);
    }

    // Usage stats
    let result = sqlx::query(
        "INSERT OR IGNORE INTO usage_stats \
         (id, session_id, turn, input_tokens, output_tokens, cache_read, cache_write, cost_usd, model, timestamp, user_id) \
         SELECT id, session_id, turn, input_tokens, output_tokens, cache_read, cache_write, cost_usd, model, timestamp, \
                COALESCE(user_id, 'admin') \
         FROM legacy.usage_stats"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate usage_stats: {}", e);
    }

    // Compaction log
    let result = sqlx::query(
        "INSERT OR IGNORE INTO compaction_log (id, session_id, timestamp, trigger, pre_tokens, summary, messages_compacted) \
         SELECT id, session_id, timestamp, trigger, pre_tokens, summary, messages_compacted \
         FROM legacy.compaction_log"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate compaction_log: {}", e);
    }

    sqlx::query("DETACH DATABASE legacy")
        .execute(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Failed to detach session.db: {}", e)))?;

    Ok(())
}

async fn migrate_cron(pool: &SqlitePool, legacy_path: &Path) -> Result<()> {
    let path_str = legacy_path.display().to_string();

    sqlx::query(&format!("ATTACH DATABASE '{}' AS legacy", path_str))
        .execute(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Failed to attach cron.db: {}", e)))?;

    // Cron jobs
    let result = sqlx::query(
        "INSERT OR IGNORE INTO cron_jobs \
         (id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, \
          created_at, last_run_at, next_run_at, retry_count, max_retries, last_error, \
          retry_at, timeout_secs, session_mode, user_id) \
         SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, \
                created_at, last_run_at, next_run_at, \
                COALESCE(retry_count, 0), COALESCE(max_retries, 3), last_error, \
                retry_at, COALESCE(timeout_secs, 7200), COALESCE(session_mode, 'isolated'), user_id \
         FROM legacy.cron_jobs"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate cron_jobs: {}", e);
    }

    // Cron runs
    let result = sqlx::query(
        "INSERT OR IGNORE INTO cron_runs (id, job_id, started_at, completed_at, status, result_summary, session_id) \
         SELECT id, job_id, started_at, completed_at, status, result_summary, session_id \
         FROM legacy.cron_runs"
    )
    .execute(pool)
    .await;
    if let Err(e) = &result {
        warn!("Failed to migrate cron_runs: {}", e);
    }

    sqlx::query("DETACH DATABASE legacy")
        .execute(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Failed to detach cron.db: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CoreDb;

    #[tokio::test]
    async fn no_legacy_dbs_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!has_legacy_dbs(tmp.path()));
    }

    #[tokio::test]
    async fn legacy_migration_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let db_dir = tmp.path();

        // Create a legacy users.db with one user
        let legacy_pool = SqlitePool::connect(
            &format!("sqlite://{}?mode=rwc", db_dir.join("users.db").display())
        ).await.unwrap();
        sqlx::query(
            "CREATE TABLE users (id TEXT PRIMARY KEY, email TEXT, display_name TEXT, \
             role TEXT NOT NULL DEFAULT 'user', is_active INTEGER NOT NULL DEFAULT 1, \
             created_at TEXT NOT NULL, updated_at TEXT NOT NULL, filesystem_enabled INTEGER NOT NULL DEFAULT 0)"
        ).execute(&legacy_pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, display_name, role, created_at, updated_at) \
             VALUES ('u1', 'test@test.com', 'Test', 'admin', '2024-01-01', '2024-01-01')"
        ).execute(&legacy_pool).await.unwrap();
        legacy_pool.close().await;

        // Open core.db — should auto-migrate
        let db = CoreDb::new(db_dir).await.unwrap();

        // Verify user was migrated
        let row: (String,) = sqlx::query_as("SELECT email FROM users WHERE id = 'u1'")
            .fetch_one(db.pool()).await.unwrap();
        assert_eq!(row.0, "test@test.com");

        // Legacy file should be renamed
        assert!(!db_dir.join("users.db").exists());
        assert!(db_dir.join("users.db.migrated").exists());
    }
}
