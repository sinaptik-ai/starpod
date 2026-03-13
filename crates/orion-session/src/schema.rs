use rusqlite::Connection;

use orion_core::OrionError;

/// Run session database migrations.
pub fn migrate(conn: &Connection) -> Result<(), OrionError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS session_metadata (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            last_message_at TEXT NOT NULL,
            is_closed INTEGER NOT NULL DEFAULT 0,
            summary TEXT,
            message_count INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS usage_stats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            turn INTEGER NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read INTEGER NOT NULL DEFAULT 0,
            cache_write INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0.0,
            model TEXT,
            timestamp TEXT NOT NULL
        );
        ",
    )
    .map_err(|e| OrionError::Database(format!("Session migration failed: {}", e)))?;

    Ok(())
}
