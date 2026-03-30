pub mod env;
pub mod known_hosts;
#[cfg(feature = "secret-proxy")]
pub mod opaque;
mod schema;

use std::path::Path;
use std::str::FromStr;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use chrono::Utc;
use rand::RngCore;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::debug;

use starpod_core::{Result, StarpodError};

// ── Vault entry metadata ─────────────────────────────────────────────────────

/// Metadata for a vault entry (returned by [`Vault::get_entry`] / [`Vault::list_entries`]).
///
/// Does **not** include the decrypted value — only classification and timestamps.
#[derive(Debug, Clone)]
pub struct VaultEntry {
    pub key: String,
    /// Whether this entry holds a secret that should be opaque-ified when the
    /// proxy is enabled. `true` (default) for API keys/tokens, `false` for
    /// non-sensitive config like `SENTRY_DSN`.
    pub is_secret: bool,
    /// Hostnames where this secret may be sent (e.g. `["api.openai.com"]`).
    /// `None` means unrestricted.
    pub allowed_hosts: Option<Vec<String>>,
    pub created_at: String,
    pub updated_at: String,
}

// ── System keys ──────────────────────────────────────────────────────────────

/// Environment variable names that are system-managed secrets.
///
/// These keys hold LLM provider credentials, service tokens, and platform
/// secrets. The agent must never read or overwrite them at runtime.
///
/// Used by [`is_system_key`] and the `EnvGet` tool to block agent access.
///
/// # Keys
///
/// | Category | Keys |
/// |----------|------|
/// | LLM providers | `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GROQ_API_KEY`, `DEEPSEEK_API_KEY`, `OPENROUTER_API_KEY` |
/// | Services | `BRAVE_API_KEY`, `TELEGRAM_BOT_TOKEN` |
///
/// Note: `STARPOD_API_KEY` is NOT a vault secret — it is pre-seeded into the
/// auth database (`core.db`) at build time via `bootstrap_admin()`.
pub const SYSTEM_KEYS: &[&str] = &[
    // LLM providers
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GROQ_API_KEY",
    "DEEPSEEK_API_KEY",
    "OPENROUTER_API_KEY",
    // Services
    "BRAVE_API_KEY",
    "TELEGRAM_BOT_TOKEN",
];

/// Returns `true` if `key` is a system-managed secret that the agent
/// must not read or modify at runtime.
///
/// Comparison is case-insensitive.
///
/// ```
/// assert!(starpod_vault::is_system_key("ANTHROPIC_API_KEY"));
/// assert!(starpod_vault::is_system_key("anthropic_api_key"));
/// assert!(!starpod_vault::is_system_key("MY_CUSTOM_VAR"));
/// ```
pub fn is_system_key(key: &str) -> bool {
    let upper = key.to_uppercase();
    SYSTEM_KEYS.iter().any(|&k| k == upper)
}

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

        let opts =
            SqliteConnectOptions::from_str(&format!("sqlite://{}?mode=rwc", db_path.display()))
                .map_err(|e| StarpodError::Database(format!("Invalid DB path: {}", e)))?
                .pragma("journal_mode", "WAL")
                .pragma("busy_timeout", "5000")
                .pragma("synchronous", "NORMAL");

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to open vault db: {}", e)))?;

        schema::run_migrations(&pool).await?;

        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| StarpodError::Vault(format!("Invalid master key: {}", e)))?;

        Ok(Self { pool, cipher })
    }

    /// Create a Vault from an existing pool (for testing).
    #[cfg(test)]
    async fn from_pool(pool: SqlitePool, master_key: &[u8; 32]) -> Result<Self> {
        schema::run_migrations(&pool).await?;
        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| StarpodError::Vault(format!("Invalid master key: {}", e)))?;
        Ok(Self { pool, cipher })
    }

    /// Retrieve and decrypt a value by key. Returns `None` if the key doesn't exist.
    pub async fn get(&self, key: &str, user_id: Option<&str>) -> Result<Option<String>> {
        let row = sqlx::query("SELECT encrypted_value, nonce FROM vault_entries WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Query failed: {}", e)))?;

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
            .map_err(|e| StarpodError::Vault(format!("Decryption failed: {}", e)))?;

        let value = String::from_utf8(plaintext)
            .map_err(|e| StarpodError::Vault(format!("Invalid UTF-8 in decrypted value: {}", e)))?;

        self.audit(key, "get", user_id).await?;
        debug!(key = %key, "Vault get");

        Ok(Some(value))
    }

    /// Encrypt and store a value. Overwrites if the key already exists.
    pub async fn set(&self, key: &str, value: &str, user_id: Option<&str>) -> Result<()> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, value.as_bytes())
            .map_err(|e| StarpodError::Vault(format!("Encryption failed: {}", e)))?;

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
        .map_err(|e| StarpodError::Database(format!("Insert failed: {}", e)))?;

        self.audit(key, "set", user_id).await?;
        debug!(key = %key, "Vault set");

        Ok(())
    }

    /// Delete a key from the vault.
    pub async fn delete(&self, key: &str, user_id: Option<&str>) -> Result<()> {
        sqlx::query("DELETE FROM vault_entries WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Delete failed: {}", e)))?;

        self.audit(key, "delete", user_id).await?;
        debug!(key = %key, "Vault delete");

        Ok(())
    }

    /// List all keys in the vault (without decrypting values).
    pub async fn list_keys(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT key FROM vault_entries ORDER BY key")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Query failed: {}", e)))?;

        let keys: Vec<String> = rows.iter().map(|row| row.get("key")).collect();
        Ok(keys)
    }

    /// Append an entry to the audit log.
    pub async fn audit(&self, key: &str, action: &str, user_id: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO vault_audit (key, action, timestamp, user_id) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(key)
        .bind(action)
        .bind(&now)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Audit log failed: {}", e)))?;
        Ok(())
    }

    /// Log an env var access by the agent (e.g. via EnvGet tool).
    ///
    /// Records a `"env_read"` entry in the audit log without decrypting
    /// anything — just tracks that the agent accessed this key.
    pub async fn log_env_read(&self, key: &str, user_id: Option<&str>) -> Result<()> {
        self.audit(key, "env_read", user_id).await
    }

    // ── Metadata-aware API (Phase 1: secret proxy) ──────────────────

    /// Store a value with secret classification and allowed-host metadata.
    ///
    /// Like [`set`](Self::set) but also persists `is_secret` and
    /// `allowed_hosts` for the proxy to enforce host binding.
    pub async fn set_with_meta(
        &self,
        key: &str,
        value: &str,
        is_secret: bool,
        allowed_hosts: Option<&[String]>,
        user_id: Option<&str>,
    ) -> Result<()> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, value.as_bytes())
            .map_err(|e| StarpodError::Vault(format!("Encryption failed: {}", e)))?;

        let now = Utc::now().to_rfc3339();
        let hosts_json: Option<String> =
            allowed_hosts.map(|h| serde_json::to_string(h).unwrap_or_default());

        sqlx::query(
            "INSERT INTO vault_entries (key, encrypted_value, nonce, is_secret, allowed_hosts, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(key) DO UPDATE SET
                 encrypted_value = excluded.encrypted_value,
                 nonce = excluded.nonce,
                 is_secret = excluded.is_secret,
                 allowed_hosts = excluded.allowed_hosts,
                 updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(&ciphertext)
        .bind(nonce.as_slice())
        .bind(is_secret as i32)
        .bind(&hosts_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Insert failed: {}", e)))?;

        self.audit(key, "set", user_id).await?;
        debug!(key = %key, is_secret = %is_secret, "Vault set_with_meta");

        Ok(())
    }

    /// Update only the metadata (`is_secret`, `allowed_hosts`) for an existing entry.
    ///
    /// Does **not** touch the encrypted value or nonce. Returns `Ok(false)` if the
    /// key does not exist.
    pub async fn update_meta(
        &self,
        key: &str,
        is_secret: bool,
        allowed_hosts: Option<&[String]>,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let hosts_json: Option<String> =
            allowed_hosts.map(|h| serde_json::to_string(h).unwrap_or_default());

        let result = sqlx::query(
            "UPDATE vault_entries SET is_secret = ?1, allowed_hosts = ?2, updated_at = ?3
             WHERE key = ?4",
        )
        .bind(is_secret as i32)
        .bind(&hosts_json)
        .bind(&now)
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Update failed: {}", e)))?;

        if result.rows_affected() > 0 {
            self.audit(key, "update_meta", None).await?;
            debug!(key = %key, is_secret = %is_secret, "Vault update_meta");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Retrieve metadata for a single vault entry (without decrypting the value).
    pub async fn get_entry(&self, key: &str) -> Result<Option<VaultEntry>> {
        let row = sqlx::query(
            "SELECT key, is_secret, allowed_hosts, created_at, updated_at
             FROM vault_entries WHERE key = ?1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Query failed: {}", e)))?;

        Ok(row.map(|r| VaultEntry {
            key: r.get("key"),
            is_secret: r.get::<i32, _>("is_secret") != 0,
            allowed_hosts: r
                .get::<Option<String>, _>("allowed_hosts")
                .and_then(|s| serde_json::from_str(&s).ok()),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    /// List all vault entries with metadata (no decrypted values).
    pub async fn list_entries(&self) -> Result<Vec<VaultEntry>> {
        let rows = sqlx::query(
            "SELECT key, is_secret, allowed_hosts, created_at, updated_at
             FROM vault_entries ORDER BY key",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Query failed: {}", e)))?;

        Ok(rows
            .iter()
            .map(|r| VaultEntry {
                key: r.get("key"),
                is_secret: r.get::<i32, _>("is_secret") != 0,
                allowed_hosts: r
                    .get::<Option<String>, _>("allowed_hosts")
                    .and_then(|s| serde_json::from_str(&s).ok()),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    /// Expose the cipher for opaque token operations.
    #[cfg(feature = "secret-proxy")]
    pub fn cipher(&self) -> &Aes256Gcm {
        &self.cipher
    }
}

/// Derive or load the 32-byte master key for a vault instance.
///
/// On first call, generates a random key and stores it at `db_dir/.vault_key`.
/// On subsequent calls, reads from that file. The key file is per-instance
/// and should never be committed to version control.
pub fn derive_master_key(db_dir: &Path) -> Result<[u8; 32]> {
    let key_path = db_dir.join(".vault_key");

    if key_path.exists() {
        let data = std::fs::read(&key_path)
            .map_err(|e| StarpodError::Vault(format!("Failed to read vault key: {}", e)))?;
        if data.len() != 32 {
            return Err(StarpodError::Vault(format!(
                "Vault key file has invalid length ({} bytes, expected 32)",
                data.len()
            )));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&data);
        Ok(key)
    } else {
        std::fs::create_dir_all(db_dir)
            .map_err(|e| StarpodError::Vault(format!("Failed to create db dir: {}", e)))?;
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        std::fs::write(&key_path, key)
            .map_err(|e| StarpodError::Vault(format!("Failed to write vault key: {}", e)))?;
        // Best-effort: restrict permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
        }
        debug!("Generated new vault master key at {}", key_path.display());
        Ok(key)
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
        vault.set("api_key", "sk-secret-123", None).await.unwrap();
        let val = vault.get("api_key", None).await.unwrap();
        assert_eq!(val.as_deref(), Some("sk-secret-123"));
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let vault = setup().await;
        let val = vault.get("nope", None).await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_overwrite() {
        let vault = setup().await;
        vault.set("token", "old", None).await.unwrap();
        vault.set("token", "new", None).await.unwrap();
        let val = vault.get("token", None).await.unwrap();
        assert_eq!(val.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn test_delete() {
        let vault = setup().await;
        vault.set("temp", "value", None).await.unwrap();
        vault.delete("temp", None).await.unwrap();
        let val = vault.get("temp", None).await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_list_keys() {
        let vault = setup().await;
        vault.set("beta", "2", None).await.unwrap();
        vault.set("alpha", "1", None).await.unwrap();
        vault.set("gamma", "3", None).await.unwrap();

        let keys = vault.list_keys().await.unwrap();
        assert_eq!(keys, vec!["alpha", "beta", "gamma"]);
    }

    #[tokio::test]
    async fn test_wrong_key_cannot_decrypt() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        // Set with one key
        let vault1 = Vault::from_pool(pool.clone(), &[0xAA; 32]).await.unwrap();
        vault1.set("secret", "hidden", None).await.unwrap();

        // Try to read with a different key
        let vault2 = Vault::from_pool(pool, &[0xBB; 32]).await.unwrap();
        let result = vault2.get("secret", None).await;
        assert!(result.is_err(), "Should fail to decrypt with wrong key");
    }

    #[tokio::test]
    async fn test_audit_log() {
        let vault = setup().await;
        vault.set("k1", "v1", None).await.unwrap();
        vault.get("k1", None).await.unwrap();
        vault.delete("k1", None).await.unwrap();

        // Check audit log directly
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vault_audit")
            .fetch_one(&vault.pool)
            .await
            .unwrap();
        assert_eq!(count.0, 3); // set + get + delete
    }

    #[tokio::test]
    async fn test_audit_log_tracks_user_id() {
        let vault = setup().await;

        vault.set("k1", "v1", Some("alice")).await.unwrap();
        vault.get("k1", Some("bob")).await.unwrap();
        vault.delete("k1", None).await.unwrap();
        vault.log_env_read("HOME", Some("charlie")).await.unwrap();

        let rows = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT action, user_id FROM vault_audit ORDER BY id",
        )
        .fetch_all(&vault.pool)
        .await
        .unwrap();

        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], ("set".to_string(), Some("alice".to_string())));
        assert_eq!(rows[1], ("get".to_string(), Some("bob".to_string())));
        assert_eq!(rows[2], ("delete".to_string(), None));
        assert_eq!(
            rows[3],
            ("env_read".to_string(), Some("charlie".to_string()))
        );
    }

    // ── derive_master_key tests ───────────────────────────────────

    #[test]
    fn test_derive_master_key_creates_new() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_dir = tmp.path().join("db");
        // db_dir doesn't exist yet — derive_master_key should create it
        let key = derive_master_key(&db_dir).unwrap();
        assert_eq!(key.len(), 32);
        assert!(db_dir.join(".vault_key").exists());
    }

    #[test]
    fn test_derive_master_key_reads_existing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_dir = tmp.path().join("db");

        let key1 = derive_master_key(&db_dir).unwrap();
        let key2 = derive_master_key(&db_dir).unwrap();
        // Same key on second call
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_derive_master_key_rejects_wrong_length() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_dir = tmp.path().join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        // Write a key with wrong length
        std::fs::write(db_dir.join(".vault_key"), [0u8; 16]).unwrap();

        let result = derive_master_key(&db_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid length"));
    }

    #[test]
    fn test_derive_master_key_different_dirs_different_keys() {
        let tmp = tempfile::TempDir::new().unwrap();
        let key1 = derive_master_key(&tmp.path().join("a")).unwrap();
        let key2 = derive_master_key(&tmp.path().join("b")).unwrap();
        assert_ne!(key1, key2);
    }

    // ── is_system_key tests ──────────────────────────────────────

    #[test]
    fn test_system_keys_are_recognized() {
        for key in super::SYSTEM_KEYS {
            assert!(super::is_system_key(key), "{} should be a system key", key);
        }
    }

    #[test]
    fn test_system_keys_case_insensitive() {
        assert!(super::is_system_key("anthropic_api_key"));
        assert!(super::is_system_key("Telegram_Bot_Token"));
    }

    #[test]
    fn test_non_system_keys() {
        assert!(!super::is_system_key("HOME"));
        assert!(!super::is_system_key("DB_PASSWORD"));
        assert!(!super::is_system_key("MY_SECRET"));
        assert!(!super::is_system_key("CUSTOM_TOKEN"));
    }

    // ── set_with_meta / get_entry / list_entries tests ───────────

    #[tokio::test]
    async fn test_set_with_meta_and_get_entry() {
        let vault = setup().await;
        let hosts = vec!["api.github.com".to_string()];
        vault
            .set_with_meta("GH_TOKEN", "ghp_abc", true, Some(&hosts), None)
            .await
            .unwrap();

        let entry = vault.get_entry("GH_TOKEN").await.unwrap().unwrap();
        assert_eq!(entry.key, "GH_TOKEN");
        assert!(entry.is_secret);
        assert_eq!(
            entry.allowed_hosts,
            Some(vec!["api.github.com".to_string()])
        );

        // Value should also be retrievable
        let val = vault.get("GH_TOKEN", None).await.unwrap();
        assert_eq!(val.as_deref(), Some("ghp_abc"));
    }

    #[tokio::test]
    async fn test_set_with_meta_non_secret() {
        let vault = setup().await;
        vault
            .set_with_meta("SENTRY_DSN", "https://sentry.io/123", false, None, None)
            .await
            .unwrap();

        let entry = vault.get_entry("SENTRY_DSN").await.unwrap().unwrap();
        assert!(!entry.is_secret);
        assert!(entry.allowed_hosts.is_none());
    }

    #[tokio::test]
    async fn test_set_with_meta_overwrites() {
        let vault = setup().await;
        vault
            .set_with_meta("KEY", "old", true, None, None)
            .await
            .unwrap();
        vault
            .set_with_meta(
                "KEY",
                "new",
                false,
                Some(&["example.com".to_string()]),
                None,
            )
            .await
            .unwrap();

        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        assert!(!entry.is_secret);
        assert_eq!(entry.allowed_hosts, Some(vec!["example.com".to_string()]));
        assert_eq!(
            vault.get("KEY", None).await.unwrap().as_deref(),
            Some("new")
        );
    }

    #[tokio::test]
    async fn test_plain_set_preserves_defaults() {
        let vault = setup().await;
        // Plain set() should get is_secret=1, allowed_hosts=NULL
        vault.set("TOKEN", "val", None).await.unwrap();

        let entry = vault.get_entry("TOKEN").await.unwrap().unwrap();
        assert!(entry.is_secret); // default 1
        assert!(entry.allowed_hosts.is_none()); // default NULL
    }

    #[tokio::test]
    async fn test_list_entries() {
        let vault = setup().await;
        vault
            .set_with_meta("B_KEY", "v", true, None, None)
            .await
            .unwrap();
        vault
            .set_with_meta(
                "A_KEY",
                "v",
                false,
                Some(&["api.example.com".to_string()]),
                None,
            )
            .await
            .unwrap();

        let entries = vault.list_entries().await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "A_KEY");
        assert!(!entries[0].is_secret);
        assert_eq!(entries[1].key, "B_KEY");
        assert!(entries[1].is_secret);
    }

    #[tokio::test]
    async fn test_get_entry_nonexistent() {
        let vault = setup().await;
        assert!(vault.get_entry("NOPE").await.unwrap().is_none());
    }

    // ── update_meta tests ────────────────────────────────────────

    #[tokio::test]
    async fn test_update_meta_changes_is_secret() {
        let vault = setup().await;
        vault.set("TOKEN", "val", None).await.unwrap();

        // Default is_secret = true
        let entry = vault.get_entry("TOKEN").await.unwrap().unwrap();
        assert!(entry.is_secret);

        // Flip to non-secret
        assert!(vault.update_meta("TOKEN", false, None).await.unwrap());

        let entry = vault.get_entry("TOKEN").await.unwrap().unwrap();
        assert!(!entry.is_secret);

        // Value should be unchanged
        assert_eq!(
            vault.get("TOKEN", None).await.unwrap().as_deref(),
            Some("val")
        );
    }

    #[tokio::test]
    async fn test_update_meta_changes_hosts() {
        let vault = setup().await;
        vault.set("KEY", "v", None).await.unwrap();

        // Initially no hosts
        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        assert!(entry.allowed_hosts.is_none());

        // Set hosts
        let hosts = vec!["api.example.com".to_string()];
        assert!(vault.update_meta("KEY", true, Some(&hosts)).await.unwrap());

        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        assert_eq!(
            entry.allowed_hosts,
            Some(vec!["api.example.com".to_string()])
        );
    }

    #[tokio::test]
    async fn test_update_meta_clears_hosts() {
        let vault = setup().await;
        vault
            .set_with_meta("KEY", "v", true, Some(&["host.com".to_string()]), None)
            .await
            .unwrap();

        // Clear hosts by passing None
        assert!(vault.update_meta("KEY", true, None).await.unwrap());

        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        assert!(entry.allowed_hosts.is_none());
    }

    #[tokio::test]
    async fn test_update_meta_nonexistent_key() {
        let vault = setup().await;
        // Returns false for missing key
        assert!(!vault.update_meta("NOPE", true, None).await.unwrap());
    }

    #[tokio::test]
    async fn test_update_meta_audit_logged() {
        let vault = setup().await;
        vault.set("KEY", "v", None).await.unwrap();
        vault.update_meta("KEY", false, None).await.unwrap();

        let rows = sqlx::query_as::<_, (String,)>("SELECT action FROM vault_audit ORDER BY id")
            .fetch_all(&vault.pool)
            .await
            .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "set");
        assert_eq!(rows[1].0, "update_meta");
    }

    // ── Metadata stress tests ────────────────────────────────────

    #[tokio::test]
    async fn test_set_with_meta_many_hosts() {
        let vault = setup().await;
        let hosts: Vec<String> = (0..200).map(|i| format!("host-{i}.example.com")).collect();
        vault
            .set_with_meta("KEY", "v", true, Some(&hosts), None)
            .await
            .unwrap();

        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        assert_eq!(entry.allowed_hosts.unwrap().len(), 200);
    }

    #[tokio::test]
    async fn test_set_with_meta_unicode_hosts() {
        let vault = setup().await;
        let hosts = vec!["api.例え.jp".to_string(), "api.مثال.com".to_string()];
        vault
            .set_with_meta("KEY", "v", true, Some(&hosts), None)
            .await
            .unwrap();

        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        assert_eq!(entry.allowed_hosts.unwrap(), hosts);
    }

    #[tokio::test]
    async fn test_set_with_meta_empty_hosts_vec() {
        let vault = setup().await;
        // Empty vec (not None) — should store as "[]"
        vault
            .set_with_meta("KEY", "v", true, Some(&[]), None)
            .await
            .unwrap();

        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        assert_eq!(entry.allowed_hosts, Some(vec![]));
    }

    #[tokio::test]
    async fn test_rapid_meta_updates() {
        let vault = setup().await;
        vault.set("KEY", "v", None).await.unwrap();

        // Rapidly toggle is_secret
        for i in 0..50 {
            let is_secret = i % 2 == 0;
            vault.update_meta("KEY", is_secret, None).await.unwrap();
        }

        // Final state should be is_secret = false (49 is odd, 49%2=1, so false)
        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        assert!(!entry.is_secret);

        // Value should be untouched through all meta updates
        assert_eq!(vault.get("KEY", None).await.unwrap().as_deref(), Some("v"));
    }

    #[tokio::test]
    async fn test_set_overwrite_does_not_clobber_metadata() {
        let vault = setup().await;
        // Set with metadata
        vault
            .set_with_meta("KEY", "v1", false, Some(&["host.com".to_string()]), None)
            .await
            .unwrap();

        // Plain set() overwrites value but should NOT touch is_secret/allowed_hosts
        vault.set("KEY", "v2", None).await.unwrap();

        let entry = vault.get_entry("KEY").await.unwrap().unwrap();
        // is_secret and allowed_hosts should be unchanged from the set_with_meta call
        assert!(!entry.is_secret);
        assert_eq!(entry.allowed_hosts, Some(vec!["host.com".to_string()]));
        // Value should be updated
        assert_eq!(vault.get("KEY", None).await.unwrap().as_deref(), Some("v2"));
    }

    #[tokio::test]
    async fn test_special_chars_in_key_name() {
        let vault = setup().await;
        // Keys with unusual but valid characters
        for key in &["MY_KEY_123", "a", "A_B_C_D_E_F"] {
            vault
                .set_with_meta(key, "val", true, None, None)
                .await
                .unwrap();
            let entry = vault.get_entry(key).await.unwrap().unwrap();
            assert_eq!(entry.key, *key);
        }
    }

    #[tokio::test]
    async fn test_list_entries_empty() {
        let vault = setup().await;
        let entries = vault.list_entries().await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_delete_cleans_up_metadata() {
        let vault = setup().await;
        vault
            .set_with_meta("KEY", "v", true, Some(&["h.com".to_string()]), None)
            .await
            .unwrap();
        vault.delete("KEY", None).await.unwrap();

        // Entry should be completely gone
        assert!(vault.get_entry("KEY").await.unwrap().is_none());
        assert!(vault.get("KEY", None).await.unwrap().is_none());
        assert!(vault.list_entries().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_concurrent_set_with_meta() {
        // SQLite WAL mode should handle concurrent writes
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let vault = Vault::from_pool(pool, &test_key()).await.unwrap();

        let mut handles = vec![];
        for i in 0..20 {
            let vault_pool = vault.pool.clone();
            let cipher = vault.cipher.clone();
            handles.push(tokio::spawn(async move {
                let v = Vault {
                    pool: vault_pool,
                    cipher,
                };
                let key = format!("KEY_{i}");
                v.set_with_meta(&key, &format!("val_{i}"), true, None, None)
                    .await
                    .unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let entries = vault.list_entries().await.unwrap();
        assert_eq!(entries.len(), 20);
    }
}
