mod schema;

use std::path::Path;
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, AeadCore, Nonce};
use chrono::Utc;
use rusqlite::Connection;
use tracing::debug;

use orion_core::{OrionError, Result};

/// Encrypted credential vault backed by SQLite + AES-256-GCM.
pub struct Vault {
    conn: Mutex<Connection>,
    cipher: Aes256Gcm,
}

impl Vault {
    /// Open (or create) a vault at the given database path.
    ///
    /// `master_key` must be exactly 32 bytes.
    pub fn new(db_path: &Path, master_key: &[u8; 32]) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)
            .map_err(|e| OrionError::Database(format!("Failed to open vault db: {}", e)))?;

        schema::migrate(&conn)?;

        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| OrionError::Vault(format!("Invalid master key: {}", e)))?;

        Ok(Self {
            conn: Mutex::new(conn),
            cipher,
        })
    }

    /// Lock the database connection.
    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("vault db mutex poisoned")
    }

    /// Retrieve and decrypt a value by key. Returns `None` if the key doesn't exist.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.db();
        let mut stmt = conn
            .prepare("SELECT encrypted_value, nonce FROM vault_entries WHERE key = ?1")
            .map_err(|e| OrionError::Database(format!("Prepare failed: {}", e)))?;

        let result: Option<(Vec<u8>, Vec<u8>)> = stmt
            .query_row(rusqlite::params![key], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()
            .map_err(|e| OrionError::Database(format!("Query failed: {}", e)))?;

        let (ciphertext, nonce_bytes) = match result {
            Some(r) => r,
            None => return Ok(None),
        };

        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|e| OrionError::Vault(format!("Decryption failed: {}", e)))?;

        let value = String::from_utf8(plaintext)
            .map_err(|e| OrionError::Vault(format!("Invalid UTF-8 in decrypted value: {}", e)))?;

        self.audit_with_conn(&conn, key, "get")?;
        debug!(key = %key, "Vault get");

        Ok(Some(value))
    }

    /// Encrypt and store a value. Overwrites if the key already exists.
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, value.as_bytes())
            .map_err(|e| OrionError::Vault(format!("Encryption failed: {}", e)))?;

        let now = Utc::now().to_rfc3339();

        let conn = self.db();
        conn.execute(
            "INSERT INTO vault_entries (key, encrypted_value, nonce, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(key) DO UPDATE SET
                 encrypted_value = excluded.encrypted_value,
                 nonce = excluded.nonce,
                 updated_at = excluded.updated_at",
            rusqlite::params![key, ciphertext, nonce.as_slice(), now],
        )
        .map_err(|e| OrionError::Database(format!("Insert failed: {}", e)))?;

        self.audit_with_conn(&conn, key, "set")?;
        debug!(key = %key, "Vault set");

        Ok(())
    }

    /// Delete a key from the vault.
    pub fn delete(&self, key: &str) -> Result<()> {
        let conn = self.db();
        conn.execute(
            "DELETE FROM vault_entries WHERE key = ?1",
            rusqlite::params![key],
        )
        .map_err(|e| OrionError::Database(format!("Delete failed: {}", e)))?;

        self.audit_with_conn(&conn, key, "delete")?;
        debug!(key = %key, "Vault delete");

        Ok(())
    }

    /// List all keys in the vault (without decrypting values).
    pub fn list_keys(&self) -> Result<Vec<String>> {
        let conn = self.db();
        let mut stmt = conn
            .prepare("SELECT key FROM vault_entries ORDER BY key")
            .map_err(|e| OrionError::Database(format!("Prepare failed: {}", e)))?;

        let keys = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| OrionError::Database(format!("Query failed: {}", e)))?
            .collect::<std::result::Result<Vec<String>, _>>()
            .map_err(|e| OrionError::Database(format!("Row read failed: {}", e)))?;

        Ok(keys)
    }

    /// Append an entry to the audit log (caller already holds the lock).
    fn audit_with_conn(&self, conn: &Connection, key: &str, action: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO vault_audit (key, action, timestamp) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, action, now],
        )
        .map_err(|e| OrionError::Database(format!("Audit log failed: {}", e)))?;
        Ok(())
    }
}

/// Helper trait to convert rusqlite's `optional()` pattern.
trait OptionalExt<T> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_key() -> [u8; 32] {
        [0xAB; 32]
    }

    #[test]
    fn test_set_and_get() {
        let tmp = TempDir::new().unwrap();
        let vault = Vault::new(&tmp.path().join("vault.db"), &test_key()).unwrap();

        vault.set("api_key", "sk-secret-123").unwrap();
        let val = vault.get("api_key").unwrap();
        assert_eq!(val.as_deref(), Some("sk-secret-123"));
    }

    #[test]
    fn test_get_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let vault = Vault::new(&tmp.path().join("vault.db"), &test_key()).unwrap();

        let val = vault.get("nope").unwrap();
        assert_eq!(val, None);
    }

    #[test]
    fn test_overwrite() {
        let tmp = TempDir::new().unwrap();
        let vault = Vault::new(&tmp.path().join("vault.db"), &test_key()).unwrap();

        vault.set("token", "old").unwrap();
        vault.set("token", "new").unwrap();
        let val = vault.get("token").unwrap();
        assert_eq!(val.as_deref(), Some("new"));
    }

    #[test]
    fn test_delete() {
        let tmp = TempDir::new().unwrap();
        let vault = Vault::new(&tmp.path().join("vault.db"), &test_key()).unwrap();

        vault.set("temp", "value").unwrap();
        vault.delete("temp").unwrap();
        let val = vault.get("temp").unwrap();
        assert_eq!(val, None);
    }

    #[test]
    fn test_list_keys() {
        let tmp = TempDir::new().unwrap();
        let vault = Vault::new(&tmp.path().join("vault.db"), &test_key()).unwrap();

        vault.set("beta", "2").unwrap();
        vault.set("alpha", "1").unwrap();
        vault.set("gamma", "3").unwrap();

        let keys = vault.list_keys().unwrap();
        assert_eq!(keys, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn test_wrong_key_cannot_decrypt() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("vault.db");

        // Set with one key
        let vault1 = Vault::new(&db_path, &[0xAA; 32]).unwrap();
        vault1.set("secret", "hidden").unwrap();
        drop(vault1);

        // Try to read with a different key
        let vault2 = Vault::new(&db_path, &[0xBB; 32]).unwrap();
        let result = vault2.get("secret");
        assert!(result.is_err(), "Should fail to decrypt with wrong key");
    }

    #[test]
    fn test_audit_log() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("vault.db");
        let vault = Vault::new(&db_path, &test_key()).unwrap();

        vault.set("k1", "v1").unwrap();
        vault.get("k1").unwrap();
        vault.delete("k1").unwrap();

        // Check audit log directly
        let conn = vault.db();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM vault_audit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3); // set + get + delete
    }
}
