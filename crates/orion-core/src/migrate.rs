use rusqlite::Connection;

use crate::OrionError;

/// A single migration step.
pub struct Migration {
    /// Monotonically increasing version number (per namespace).
    pub version: u32,
    /// Descriptive name, e.g. "create_memory_fts".
    pub name: &'static str,
    /// The SQL to execute.
    pub sql: &'static str,
}

/// Run migrations against a connection.
///
/// `namespace` scopes version numbers so multiple crates can share a database
/// (e.g. "memory", "vault", "session").
///
/// Each migration runs in its own transaction. Already-applied migrations are skipped.
pub fn run_migrations(
    conn: &Connection,
    namespace: &str,
    migrations: &[Migration],
) -> Result<(), OrionError> {
    // Ensure the _migrations table exists
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            namespace TEXT NOT NULL,
            version INTEGER NOT NULL,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL,
            UNIQUE(namespace, version)
        );",
    )
    .map_err(|e| OrionError::Database(format!("Failed to create _migrations table: {}", e)))?;

    // Find the highest applied version for this namespace
    let current_version: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _migrations WHERE namespace = ?1",
            rusqlite::params![namespace],
            |row| row.get(0),
        )
        .map_err(|e| OrionError::Database(format!("Failed to query migration version: {}", e)))?;

    // Apply pending migrations in order
    for migration in migrations {
        if migration.version <= current_version {
            continue;
        }

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| OrionError::Database(format!("Failed to begin transaction: {}", e)))?;

        tx.execute_batch(migration.sql)
            .map_err(|e| {
                OrionError::Database(format!(
                    "Migration {}/{} ({}) failed: {}",
                    namespace, migration.version, migration.name, e
                ))
            })?;

        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO _migrations (namespace, version, name, applied_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![namespace, migration.version, migration.name, now],
        )
        .map_err(|e| OrionError::Database(format!("Failed to record migration: {}", e)))?;

        tx.commit()
            .map_err(|e| OrionError::Database(format!("Failed to commit migration: {}", e)))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_memory_db() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn test_run_migrations_basic() {
        let conn = open_memory_db();

        let migrations = &[
            Migration {
                version: 1,
                name: "create_items",
                sql: "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
            },
            Migration {
                version: 2,
                name: "add_items_desc",
                sql: "ALTER TABLE items ADD COLUMN description TEXT;",
            },
        ];

        run_migrations(&conn, "test", migrations).unwrap();

        // Table should exist with both columns
        conn.execute(
            "INSERT INTO items (name, description) VALUES ('x', 'desc')",
            [],
        )
        .unwrap();

        // Migrations should be recorded
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE namespace = 'test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_run_migrations_idempotent() {
        let conn = open_memory_db();

        let migrations = &[Migration {
            version: 1,
            name: "create_things",
            sql: "CREATE TABLE IF NOT EXISTS things (id INTEGER PRIMARY KEY);",
        }];

        // Run twice — should not error
        run_migrations(&conn, "test", migrations).unwrap();
        run_migrations(&conn, "test", migrations).unwrap();

        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE namespace = 'test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_namespaces_are_independent() {
        let conn = open_memory_db();

        let m1 = &[Migration {
            version: 1,
            name: "create_a",
            sql: "CREATE TABLE ns_a (id INTEGER PRIMARY KEY);",
        }];

        let m2 = &[Migration {
            version: 1,
            name: "create_b",
            sql: "CREATE TABLE ns_b (id INTEGER PRIMARY KEY);",
        }];

        run_migrations(&conn, "ns_a", m1).unwrap();
        run_migrations(&conn, "ns_b", m2).unwrap();

        // Both tables should exist
        conn.execute("INSERT INTO ns_a (id) VALUES (1)", []).unwrap();
        conn.execute("INSERT INTO ns_b (id) VALUES (1)", []).unwrap();
    }

    #[test]
    fn test_incremental_migration() {
        let conn = open_memory_db();

        // Apply v1 only
        let v1 = &[Migration {
            version: 1,
            name: "create_t",
            sql: "CREATE TABLE t (id INTEGER PRIMARY KEY);",
        }];
        run_migrations(&conn, "test", v1).unwrap();

        // Now apply v1 + v2
        let v1_v2 = &[
            Migration {
                version: 1,
                name: "create_t",
                sql: "CREATE TABLE t (id INTEGER PRIMARY KEY);",
            },
            Migration {
                version: 2,
                name: "add_t_name",
                sql: "ALTER TABLE t ADD COLUMN name TEXT;",
            },
        ];
        run_migrations(&conn, "test", v1_v2).unwrap();

        // v2 should have been applied
        conn.execute("INSERT INTO t (name) VALUES ('hello')", []).unwrap();

        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }
}
