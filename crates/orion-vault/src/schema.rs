use rusqlite::Connection;

use orion_core::{Migration, OrionError};

/// Vault migrations.
pub fn migrations() -> &'static [Migration] {
    &[Migration {
        version: 1,
        name: "create_vault_tables",
        sql: "
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
    }]
}

/// Run vault database migrations.
pub fn migrate(conn: &Connection) -> Result<(), OrionError> {
    orion_core::run_migrations(conn, "vault", migrations())
}
