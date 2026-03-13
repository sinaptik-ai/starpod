mod schema;

use std::path::Path;
use std::str::FromStr;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, AeadCore, Nonce};
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::debug;

use orion_core::{OrionError, Result};

/// Encrypted credential vault backed by SQLite + AES-256-GCM.
pub struct Vault {
    pool: SqlitePool,
    cipher: Aes256Gcm,
}

impl Vault {
    /// Open (or create) a vault at the given database path.
    ///
    /// `master_key` must be exactly 32 bytes.
    pub async fn new(db_path: &Path, master_key: &[u8; 32]) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let opts = SqliteConnectOptions::from_str(
            &format!("sqlite://{}?mode=rwc", db_path.display()),
        )
        .map_err(|e| OrionError::Database(format!("Invalid DB path: {}", e)))?;

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(|e| OrionError::Database(format!("Failed to open vault db: {}", e)))?;

        schema::run_migrations(&pool).await?;

        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| OrionError::Vault(format!("Invalid master key: {}", e)))?;

        Ok(Self { pool, cipher })
    }

    /// Create a Vault from an existing pool (for testing).
    #[cfg(test)]
    async fn from_pool(pool: SqlitePool, master_key: &[u8; 32]) -> Result<Self> {
        schema::run_migrations(&pool).await?;
        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| OrionError::Vault(format!("Invalid master key: {}", e)))?;
        Ok(Self { pool, cipher })
    }

    /// Retrieve and decrypt a value by key. Returns `None` if the key doesn't exist.
    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT encrypted_value, nonce FROM vault_entries WHERE key = ?1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| OrionError::Database(format!("Query failed: {}", e)))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let ciphertext: Vec<u8> = row.get("encrypted_value");
        let nonce_bytes: Vec<u8> = row.get("nonce");

        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|e| OrionError::Vault(format!("Decryption failed: {}", e)))?;

        let value = String::from_utf8(plaintext)
            .map_err(|e| OrionError::Vault(format!("Invalid UTF-8 in decrypted value: {}", e)))?;

        self.audit(key, "get").await?;
        debug!(key = %key, "Vault get");

        Ok(Some(value))
    }

    /// Encrypt and store a value. Overwrites if the key already exists.
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, value.as_bytes())
            .map_err(|e| OrionError::Vault(format!("Encryption failed: {}", e)))?;

        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO vault_entries (key, encrypted_value, nonce, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(key) DO UPDATE SET
                 encrypted_value = excluded.encrypted_value,
                 nonce = excluded.nonce,
                 updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(&ciphertext)
        .bind(nonce.as_slice())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| OrionError::Database(format!("Insert failed: {}", e)))?;

        self.audit(key, "set").await?;
        debug!(key = %key, "Vault set");

        Ok(())
    }

    /// Delete a key from the vault.
    pub async fn delete(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM vault_entries WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| OrionError::Database(format!("Delete failed: {}", e)))?;

        self.audit(key, "delete").await?;
        debug!(key = %key, "Vault delete");

        Ok(())
    }

    /// List all keys in the vault (without decrypting values).
    pub async fn list_keys(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT key FROM vault_entries ORDER BY key")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| OrionError::Database(format!("Query failed: {}", e)))?;

        let keys: Vec<String> = rows.iter().map(|row| row.get("key")).collect();
        Ok(keys)
    }

    /// Append an entry to the audit log.
    async fn audit(&self, key: &str, action: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO vault_audit (key, action, timestamp) VALUES (?1, ?2, ?3)")
            .bind(key)
            .bind(action)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(|e| OrionError::Database(format!("Audit log failed: {}", e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [0xAB; 32]
    }

    async fn setup() -> Vault {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        Vault::from_pool(pool, &test_key()).await.unwrap()
    }

    #[tokio::test]
    async fn test_set_and_get() {
        let vault = setup().await;
        vault.set("api_key", "sk-secret-123").await.unwrap();
        let val = vault.get("api_key").await.unwrap();
        assert_eq!(val.as_deref(), Some("sk-secret-123"));
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let vault = setup().await;
        let val = vault.get("nope").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_overwrite() {
        let vault = setup().await;
        vault.set("token", "old").await.unwrap();
        vault.set("token", "new").await.unwrap();
        let val = vault.get("token").await.unwrap();
        assert_eq!(val.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn test_delete() {
        let vault = setup().await;
        vault.set("temp", "value").await.unwrap();
        vault.delete("temp").await.unwrap();
        let val = vault.get("temp").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_list_keys() {
        let vault = setup().await;
        vault.set("beta", "2").await.unwrap();
        vault.set("alpha", "1").await.unwrap();
        vault.set("gamma", "3").await.unwrap();

        let keys = vault.list_keys().await.unwrap();
        assert_eq!(keys, vec!["alpha", "beta", "gamma"]);
    }

    #[tokio::test]
    async fn test_wrong_key_cannot_decrypt() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        // Set with one key
        let vault1 = Vault::from_pool(pool.clone(), &[0xAA; 32]).await.unwrap();
        vault1.set("secret", "hidden").await.unwrap();

        // Try to read with a different key
        let vault2 = Vault::from_pool(pool, &[0xBB; 32]).await.unwrap();
        let result = vault2.get("secret").await;
        assert!(result.is_err(), "Should fail to decrypt with wrong key");
    }

    #[tokio::test]
    async fn test_audit_log() {
        let vault = setup().await;
        vault.set("k1", "v1").await.unwrap();
        vault.get("k1").await.unwrap();
        vault.delete("k1").await.unwrap();

        // Check audit log directly
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vault_audit")
            .fetch_one(&vault.pool)
            .await
            .unwrap();
        assert_eq!(count.0, 3); // set + get + delete
    }
}
