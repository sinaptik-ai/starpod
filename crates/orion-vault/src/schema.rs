use rusqlite::Connection;

use orion_core::OrionError;

/// Run vault database migrations.
pub fn migrate(conn: &Connection) -> Result<(), OrionError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS vault_entries (
            key TEXT PRIMARY KEY,
            encrypted_value BLOB NOT NULL,
            nonce BLOB NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS vault_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            key TEXT NOT NULL,
            action TEXT NOT NULL,
            timestamp TEXT NOT NULL
        );
        ",
    )
    .map_err(|e| OrionError::Database(format!("Vault migration failed: {}", e)))?;

    Ok(())
}
