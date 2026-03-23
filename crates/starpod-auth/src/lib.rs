//! Database-backed user authentication for Starpod.
//!
//! This crate provides per-user API keys (argon2id-hashed), Telegram account
//! linking, role-based access control (admin/user), in-memory rate limiting,
//! and an audit log — all backed by a SQLite database (`users.db`).
//!
//! ## Key concepts
//!
//! - **Users** are identified by UUID and can be admin or regular users.
//! - **API keys** follow the format `sp_live_` + 40 hex chars. Only the
//!   argon2id hash is stored; the plaintext is returned once at creation.
//! - **Telegram links** map a Telegram user ID to a database user for
//!   bot authentication.
//! - **Bootstrap** creates the first admin user on an empty database and
//!   optionally imports a legacy `STARPOD_API_KEY` for backward compatibility.
//!
//! ## Usage
//!
//! ```no_run
//! # async fn example() -> starpod_core::Result<()> {
//! use std::path::Path;
//! use starpod_auth::{AuthStore, Role};
//!
//! let store = AuthStore::new(Path::new(".starpod/db/users.db")).await?;
//! let user = store.create_user(None, Some("Alice"), Role::User).await?;
//! let key = store.create_api_key(&user.id, Some("web")).await?;
//! // key.key is the plaintext — show it once, then discard
//!
//! let authed = store.authenticate_api_key(&key.key).await?;
//! assert!(authed.is_some());
//! # Ok(())
//! # }
//! ```

pub mod api_key;
pub mod rate_limit;
mod schema;
pub mod types;

use std::path::Path;
use std::str::FromStr;

use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::{debug, info};
use uuid::Uuid;

use starpod_core::{StarpodError, Result};

pub use rate_limit::RateLimiter;
pub use types::*;

/// Database-backed authentication store.
///
/// Wraps a SQLite connection pool and provides methods for user management,
/// API key authentication, Telegram linking, and audit logging.
///
/// Thread-safe: can be wrapped in `Arc` and shared across async tasks.
pub struct AuthStore {
    pool: SqlitePool,
}

impl AuthStore {
    /// Open (or create) the auth store at the given database path.
    pub async fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let opts = SqliteConnectOptions::from_str(
            &format!("sqlite://{}?mode=rwc", db_path.display()),
        )
        .map_err(|e| StarpodError::Database(format!("Invalid DB path: {}", e)))?;

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to open auth db: {}", e)))?;

        schema::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    /// Create from an existing pool (for testing).
    #[cfg(test)]
    async fn from_pool(pool: SqlitePool) -> Result<Self> {
        schema::run_migrations(&pool).await?;
        Ok(Self { pool })
    }

    // ── User CRUD ────────────────────────────────────────────────────────

    /// Create a new user with a random UUID.
    ///
    /// Both `email` and `display_name` are optional. If `email` is provided,
    /// it must be unique across all users (enforced by the database).
    pub async fn create_user(
        &self,
        email: Option<&str>,
        display_name: Option<&str>,
        role: Role,
    ) -> Result<User> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let role_str = role.as_str();

        sqlx::query(
            "INSERT INTO users (id, email, display_name, role, is_active, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 1, ?, ?)"
        )
        .bind(&id)
        .bind(email)
        .bind(display_name)
        .bind(role_str)
        .bind(&now_str)
        .bind(&now_str)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to create user: {}", e)))?;

        debug!(user_id = %id, role = %role_str, "User created");

        Ok(User {
            id,
            email: email.map(String::from),
            display_name: display_name.map(String::from),
            role,
            is_active: true,
            filesystem_enabled: false,
            created_at: now,
            updated_at: now,
        })
    }

    /// Get a user by ID.
    pub async fn get_user(&self, id: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, display_name, role, is_active, filesystem_enabled, created_at, updated_at FROM users WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to get user: {}", e)))?;

        Ok(row.map(|r| row_to_user(&r)))
    }

    /// List all users.
    pub async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, email, display_name, role, is_active, filesystem_enabled, created_at, updated_at \
             FROM users ORDER BY created_at ASC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to list users: {}", e)))?;

        Ok(rows.iter().map(row_to_user).collect())
    }

    /// Update a user's fields. Only non-`None` values are applied (COALESCE).
    ///
    /// Pass `role: Some(Role::Admin)` to promote a user, or `role: None` to
    /// leave the role unchanged.
    pub async fn update_user(
        &self,
        id: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        role: Option<Role>,
        filesystem_enabled: Option<bool>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        let role_clause = role.map(|r| format!(", role = '{}'", r.as_str())).unwrap_or_default();
        let fs_clause = filesystem_enabled.map(|v| format!(", filesystem_enabled = {}", v as i32)).unwrap_or_default();

        let sql = format!(
            "UPDATE users SET email = COALESCE(?, email), display_name = COALESCE(?, display_name){}{}, \
             updated_at = ? WHERE id = ?",
            role_clause, fs_clause,
        );

        sqlx::query(&sql)
            .bind(email)
            .bind(display_name)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Failed to update user: {}", e)))?;

        Ok(())
    }

    /// Deactivate a user (soft-delete). Sets `is_active = false`.
    ///
    /// Deactivated users cannot authenticate via API keys or Telegram.
    /// Their data is preserved — use this instead of hard-deleting.
    pub async fn deactivate_user(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE users SET is_active = 0, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Failed to deactivate user: {}", e)))?;
        Ok(())
    }

    /// Reactivate a previously deactivated user. Sets `is_active = true`.
    pub async fn activate_user(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE users SET is_active = 1, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Failed to activate user: {}", e)))?;
        Ok(())
    }

    // ── API Keys ─────────────────────────────────────────────────────────

    /// Create a new API key for a user. Returns the full key (shown only once).
    pub async fn create_api_key(
        &self,
        user_id: &str,
        label: Option<&str>,
    ) -> Result<ApiKeyCreated> {
        let key = api_key::generate_key();
        let prefix = api_key::extract_prefix(&key)
            .ok_or_else(|| StarpodError::Auth("Failed to extract key prefix".into()))?
            .to_string();
        let hash = api_key::hash_key(&key)
            .map_err(|e| StarpodError::Auth(format!("Failed to hash key: {}", e)))?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        sqlx::query(
            "INSERT INTO api_keys (id, user_id, prefix, key_hash, label, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(user_id)
        .bind(&prefix)
        .bind(&hash)
        .bind(label)
        .bind(&now_str)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to create API key: {}", e)))?;

        debug!(user_id = %user_id, prefix = %prefix, "API key created");

        Ok(ApiKeyCreated {
            meta: ApiKeyMeta {
                id,
                user_id: user_id.to_string(),
                prefix,
                label: label.map(String::from),
                expires_at: None,
                revoked_at: None,
                last_used_at: None,
                created_at: now,
            },
            key,
        })
    }

    /// Import an existing plaintext key as a user's API key (for backward-compat bootstrap).
    pub async fn import_api_key(
        &self,
        user_id: &str,
        plaintext_key: &str,
        label: Option<&str>,
    ) -> Result<ApiKeyMeta> {
        let prefix = api_key::extract_prefix(plaintext_key)
            .unwrap_or_else(|| &plaintext_key[..plaintext_key.len().min(8)])
            .to_string();

        let hash = api_key::hash_key(plaintext_key)
            .map_err(|e| StarpodError::Auth(format!("Failed to hash key: {}", e)))?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        sqlx::query(
            "INSERT INTO api_keys (id, user_id, prefix, key_hash, label, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(user_id)
        .bind(&prefix)
        .bind(&hash)
        .bind(label)
        .bind(&now_str)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to import API key: {}", e)))?;

        info!(user_id = %user_id, prefix = %prefix, "API key imported");

        Ok(ApiKeyMeta {
            id,
            user_id: user_id.to_string(),
            prefix,
            label: label.map(String::from),
            expires_at: None,
            revoked_at: None,
            last_used_at: None,
            created_at: now,
        })
    }

    /// Authenticate a request by API key. Returns the user if valid.
    pub async fn authenticate_api_key(&self, key: &str) -> Result<Option<User>> {
        // Extract prefix: sp_live_ keys use the standard prefix, others use first 8 chars
        let prefix = api_key::extract_prefix(key)
            .unwrap_or_else(|| &key[..key.len().min(8)]);

        let candidates = sqlx::query(
                "SELECT ak.id AS ak_id, ak.key_hash, u.id, u.email, u.display_name, u.role, u.is_active, \
                 u.filesystem_enabled, u.created_at, u.updated_at \
                 FROM api_keys ak JOIN users u ON ak.user_id = u.id \
                 WHERE ak.prefix = ? AND ak.revoked_at IS NULL AND u.is_active = 1"
            )
            .bind(prefix)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Auth query failed: {}", e)))?;

        for row in &candidates {
            let hash: String = row.get("key_hash");
            if api_key::verify_key(key, &hash) {
                let ak_id: String = row.get("ak_id");
                // Update last_used_at
                let now = Utc::now().to_rfc3339();
                let _ = sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
                    .bind(&now)
                    .bind(&ak_id)
                    .execute(&self.pool)
                    .await;

                return Ok(Some(User {
                    id: row.get("id"),
                    email: row.get("email"),
                    display_name: row.get("display_name"),
                    role: Role::from_str(row.get::<&str, _>("role")).unwrap_or(Role::User),
                    is_active: row.get::<bool, _>("is_active"),
                    filesystem_enabled: row.get::<bool, _>("filesystem_enabled"),
                    created_at: parse_dt(row.get("created_at")),
                    updated_at: parse_dt(row.get("updated_at")),
                }));
            }
        }

        Ok(None)
    }

    /// List API keys for a user (metadata only, no hashes).
    pub async fn list_api_keys(&self, user_id: &str) -> Result<Vec<ApiKeyMeta>> {
        let rows = sqlx::query(
            "SELECT id, user_id, prefix, label, expires_at, revoked_at, last_used_at, created_at \
             FROM api_keys WHERE user_id = ? ORDER BY created_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to list API keys: {}", e)))?;

        Ok(rows.iter().map(row_to_api_key_meta).collect())
    }

    /// Revoke an API key by its database ID.
    ///
    /// Revoked keys immediately fail authentication. The key record is
    /// preserved for audit purposes.
    pub async fn revoke_api_key(&self, key_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE api_keys SET revoked_at = ? WHERE id = ?")
            .bind(&now)
            .bind(key_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Failed to revoke API key: {}", e)))?;
        Ok(())
    }

    // ── Telegram Links ───────────────────────────────────────────────────

    /// Link a Telegram account to a user.
    ///
    /// If the `telegram_id` is already linked to another user, the old link
    /// is replaced (one Telegram account → one user).
    pub async fn link_telegram(
        &self,
        user_id: &str,
        telegram_id: i64,
        username: Option<&str>,
    ) -> Result<TelegramLink> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        sqlx::query(
            "INSERT OR REPLACE INTO telegram_links (telegram_id, user_id, username, linked_at) \
             VALUES (?, ?, ?, ?)"
        )
        .bind(telegram_id)
        .bind(user_id)
        .bind(username)
        .bind(&now_str)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to link Telegram: {}", e)))?;

        debug!(user_id = %user_id, telegram_id = %telegram_id, "Telegram account linked");

        Ok(TelegramLink {
            telegram_id,
            user_id: user_id.to_string(),
            username: username.map(String::from),
            linked_at: now,
        })
    }

    /// Unlink a Telegram account.
    pub async fn unlink_telegram(&self, telegram_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM telegram_links WHERE telegram_id = ?")
            .bind(telegram_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Failed to unlink Telegram: {}", e)))?;
        Ok(())
    }

    /// Authenticate a Telegram user by their numeric ID.
    ///
    /// Returns the linked user if the Telegram ID is linked to an active user,
    /// or `None` if unlinked or the user is deactivated.
    pub async fn authenticate_telegram(&self, telegram_id: i64) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT u.id, u.email, u.display_name, u.role, u.is_active, u.filesystem_enabled, u.created_at, u.updated_at \
             FROM telegram_links tl JOIN users u ON tl.user_id = u.id \
             WHERE tl.telegram_id = ? AND u.is_active = 1"
        )
        .bind(telegram_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Telegram auth query failed: {}", e)))?;

        Ok(row.map(|r| row_to_user(&r)))
    }

    /// Get the Telegram link for a specific user.
    pub async fn get_telegram_link_for_user(&self, user_id: &str) -> Result<Option<TelegramLink>> {
        let row = sqlx::query(
            "SELECT telegram_id, user_id, username, linked_at FROM telegram_links WHERE user_id = ?"
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to get Telegram link: {}", e)))?;

        Ok(row.map(|r| TelegramLink {
            telegram_id: r.get("telegram_id"),
            user_id: r.get("user_id"),
            username: r.get("username"),
            linked_at: parse_dt(r.get("linked_at")),
        }))
    }

    /// Unlink a Telegram account by user ID.
    pub async fn unlink_telegram_by_user(&self, user_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM telegram_links WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Failed to unlink Telegram: {}", e)))?;
        Ok(())
    }

    /// List all Telegram links.
    pub async fn list_telegram_links(&self) -> Result<Vec<TelegramLink>> {
        let rows = sqlx::query(
            "SELECT telegram_id, user_id, username, linked_at FROM telegram_links ORDER BY linked_at DESC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to list Telegram links: {}", e)))?;

        Ok(rows.iter().map(|r| TelegramLink {
            telegram_id: r.get("telegram_id"),
            user_id: r.get("user_id"),
            username: r.get("username"),
            linked_at: parse_dt(r.get("linked_at")),
        }).collect())
    }

    // ── Audit Log ────────────────────────────────────────────────────────

    /// Log an authentication event to the audit table.
    ///
    /// Use `user_id: None` when the user could not be identified (e.g. a
    /// failed auth attempt with an unknown key).
    pub async fn log_event(
        &self,
        user_id: Option<&str>,
        event_type: &str,
        detail: Option<&str>,
        ip_address: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO auth_audit_log (user_id, event_type, detail, ip_address, created_at) \
             VALUES (?, ?, ?, ?, ?)"
        )
        .bind(user_id)
        .bind(event_type)
        .bind(detail)
        .bind(ip_address)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to log event: {}", e)))?;
        Ok(())
    }

    /// Get recent audit log entries, most recent first.
    pub async fn recent_audit(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let rows = sqlx::query(
            "SELECT id, user_id, event_type, detail, ip_address, created_at \
             FROM auth_audit_log ORDER BY created_at DESC LIMIT ?"
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Auth(format!("Failed to get audit log: {}", e)))?;

        Ok(rows.iter().map(|r| AuditEntry {
            id: r.get("id"),
            user_id: r.get("user_id"),
            event_type: r.get("event_type"),
            detail: r.get("detail"),
            ip_address: r.get("ip_address"),
            created_at: parse_dt(r.get("created_at")),
        }).collect())
    }

    // ── Bootstrap ────────────────────────────────────────────────────────

    /// Bootstrap the admin user on first startup.
    ///
    /// If no users exist, creates an admin user. If `existing_api_key` is provided,
    /// imports it as the admin's key (backward compat). Otherwise generates a new key.
    ///
    /// Returns the admin user and the API key (plaintext, for logging).
    pub async fn bootstrap_admin(
        &self,
        existing_api_key: Option<&str>,
    ) -> Result<Option<(User, String)>> {
        // Check if any users exist already
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Count query failed: {}", e)))?;

        if count > 0 {
            return Ok(None); // Already bootstrapped
        }

        let admin = self.create_user(None, Some("Admin"), Role::Admin).await?;
        // Enable filesystem access for the bootstrap admin
        self.update_user(&admin.id, None, None, None, Some(true)).await?;
        let admin = self.get_user(&admin.id).await?.unwrap_or(admin);

        let key_str = if let Some(existing) = existing_api_key {
            self.import_api_key(&admin.id, existing, Some("Imported from STARPOD_API_KEY")).await?;
            info!("Imported existing STARPOD_API_KEY as admin API key");
            existing.to_string()
        } else {
            let created = self.create_api_key(&admin.id, Some("Auto-generated admin key")).await?;
            info!(key = %created.key, "Generated new admin API key — save this!");
            created.key
        };

        self.log_event(Some(&admin.id), "bootstrap", Some("Admin user created"), None).await?;

        Ok(Some((admin, key_str)))
    }

    /// Check if any users exist in the database.
    ///
    /// Used by the gateway to decide whether to enforce authentication:
    /// when `false` (fresh install), all requests are allowed.
    pub async fn has_users(&self) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StarpodError::Auth(format!("Count query failed: {}", e)))?;
        Ok(count > 0)
    }
}

// ── Row converters ───────────────────────────────────────────────────────

fn row_to_user(row: &sqlx::sqlite::SqliteRow) -> User {
    User {
        id: row.get("id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        role: Role::from_str(row.get::<&str, _>("role")).unwrap_or(Role::User),
        is_active: row.get::<bool, _>("is_active"),
        filesystem_enabled: row.get::<bool, _>("filesystem_enabled"),
        created_at: parse_dt(row.get("created_at")),
        updated_at: parse_dt(row.get("updated_at")),
    }
}

fn row_to_api_key_meta(row: &sqlx::sqlite::SqliteRow) -> ApiKeyMeta {
    ApiKeyMeta {
        id: row.get("id"),
        user_id: row.get("user_id"),
        prefix: row.get("prefix"),
        label: row.get("label"),
        expires_at: row.get::<Option<String>, _>("expires_at").and_then(|s| parse_dt_opt(&s)),
        revoked_at: row.get::<Option<String>, _>("revoked_at").and_then(|s| parse_dt_opt(&s)),
        last_used_at: row.get::<Option<String>, _>("last_used_at").and_then(|s| parse_dt_opt(&s)),
        created_at: parse_dt(row.get("created_at")),
    }
}

fn parse_dt(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_dt_opt(s: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> AuthStore {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        AuthStore::from_pool(pool).await.unwrap()
    }

    #[tokio::test]
    async fn create_and_get_user() {
        let store = test_store().await;
        let user = store.create_user(Some("test@example.com"), Some("Test"), Role::User).await.unwrap();
        assert_eq!(user.role, Role::User);
        assert!(user.is_active);

        let fetched = store.get_user(&user.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, user.id);
        assert_eq!(fetched.email.as_deref(), Some("test@example.com"));
    }

    #[tokio::test]
    async fn list_users() {
        let store = test_store().await;
        store.create_user(None, Some("A"), Role::Admin).await.unwrap();
        store.create_user(None, Some("B"), Role::User).await.unwrap();

        let users = store.list_users().await.unwrap();
        assert_eq!(users.len(), 2);
    }

    #[tokio::test]
    async fn update_user() {
        let store = test_store().await;
        let user = store.create_user(None, Some("Old"), Role::User).await.unwrap();
        store.update_user(&user.id, Some("new@example.com"), Some("New"), None, None).await.unwrap();

        let fetched = store.get_user(&user.id).await.unwrap().unwrap();
        assert_eq!(fetched.email.as_deref(), Some("new@example.com"));
        assert_eq!(fetched.display_name.as_deref(), Some("New"));
    }

    #[tokio::test]
    async fn deactivate_user() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.deactivate_user(&user.id).await.unwrap();

        let fetched = store.get_user(&user.id).await.unwrap().unwrap();
        assert!(!fetched.is_active);
    }

    #[tokio::test]
    async fn api_key_create_and_authenticate() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        let created = store.create_api_key(&user.id, Some("test key")).await.unwrap();

        assert!(created.key.starts_with("sp_live_"));
        assert_eq!(created.meta.label.as_deref(), Some("test key"));

        let authed = store.authenticate_api_key(&created.key).await.unwrap().unwrap();
        assert_eq!(authed.id, user.id);
    }

    #[tokio::test]
    async fn api_key_wrong_key_fails() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.create_api_key(&user.id, None).await.unwrap();

        let result = store.authenticate_api_key("sp_live_0000000000000000000000000000000000000000").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn revoked_key_fails_auth() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        let created = store.create_api_key(&user.id, None).await.unwrap();

        store.revoke_api_key(&created.meta.id).await.unwrap();

        let result = store.authenticate_api_key(&created.key).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn deactivated_user_fails_auth() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        let created = store.create_api_key(&user.id, None).await.unwrap();

        store.deactivate_user(&user.id).await.unwrap();

        let result = store.authenticate_api_key(&created.key).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_api_keys() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.create_api_key(&user.id, Some("key1")).await.unwrap();
        store.create_api_key(&user.id, Some("key2")).await.unwrap();

        let keys = store.list_api_keys(&user.id).await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn telegram_link_and_auth() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.link_telegram(&user.id, 123456789, Some("alice")).await.unwrap();

        let authed = store.authenticate_telegram(123456789).await.unwrap().unwrap();
        assert_eq!(authed.id, user.id);
    }

    #[tokio::test]
    async fn telegram_unlinked_fails() {
        let store = test_store().await;
        let result = store.authenticate_telegram(999999).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn telegram_unlink() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.link_telegram(&user.id, 123, None).await.unwrap();
        store.unlink_telegram(123).await.unwrap();

        let result = store.authenticate_telegram(123).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_telegram_links() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.link_telegram(&user.id, 111, Some("alice")).await.unwrap();
        store.link_telegram(&user.id, 222, Some("bob")).await.unwrap();

        let links = store.list_telegram_links().await.unwrap();
        assert_eq!(links.len(), 2);
    }

    #[tokio::test]
    async fn audit_log() {
        let store = test_store().await;
        store.log_event(Some("user1"), "login", Some("via API key"), Some("127.0.0.1")).await.unwrap();
        store.log_event(None, "failed_auth", Some("invalid key"), Some("1.2.3.4")).await.unwrap();

        let entries = store.recent_audit(10).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].event_type, "failed_auth"); // most recent first
    }

    #[tokio::test]
    async fn bootstrap_admin_creates_user_and_key() {
        let store = test_store().await;
        let result = store.bootstrap_admin(None).await.unwrap();
        assert!(result.is_some());

        let (admin, key) = result.unwrap();
        assert_eq!(admin.role, Role::Admin);
        assert!(key.starts_with("sp_live_"));

        // Second call should return None (already bootstrapped)
        let result2 = store.bootstrap_admin(None).await.unwrap();
        assert!(result2.is_none());
    }

    #[tokio::test]
    async fn bootstrap_admin_with_existing_key() {
        let store = test_store().await;
        let legacy_key = "my-old-secret-key";
        let result = store.bootstrap_admin(Some(legacy_key)).await.unwrap();
        assert!(result.is_some());

        let (_, returned_key) = result.unwrap();
        assert_eq!(returned_key, legacy_key);

        // Legacy key should authenticate
        let authed = store.authenticate_api_key(legacy_key).await.unwrap();
        assert!(authed.is_some());
        assert_eq!(authed.unwrap().role, Role::Admin);
    }

    #[tokio::test]
    async fn update_user_role() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.update_user(&user.id, None, None, Some(Role::Admin), None).await.unwrap();

        let fetched = store.get_user(&user.id).await.unwrap().unwrap();
        assert_eq!(fetched.role, Role::Admin);
    }

    #[tokio::test]
    async fn has_users_empty() {
        let store = test_store().await;
        assert!(!store.has_users().await.unwrap());
    }

    #[tokio::test]
    async fn has_users_with_user() {
        let store = test_store().await;
        store.create_user(None, None, Role::User).await.unwrap();
        assert!(store.has_users().await.unwrap());
    }

    #[tokio::test]
    async fn duplicate_email_rejected() {
        let store = test_store().await;
        store.create_user(Some("dup@example.com"), None, Role::User).await.unwrap();
        let result = store.create_user(Some("dup@example.com"), None, Role::User).await;
        assert!(result.is_err(), "Duplicate email should be rejected by UNIQUE constraint");
    }

    #[tokio::test]
    async fn null_email_allows_multiple() {
        let store = test_store().await;
        store.create_user(None, None, Role::User).await.unwrap();
        store.create_user(None, None, Role::User).await.unwrap();
        let users = store.list_users().await.unwrap();
        assert_eq!(users.len(), 2, "Multiple users with NULL email should be allowed");
    }

    #[tokio::test]
    async fn get_nonexistent_user() {
        let store = test_store().await;
        let result = store.get_user("nonexistent-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn authenticate_empty_key() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.create_api_key(&user.id, None).await.unwrap();
        let result = store.authenticate_api_key("").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn authenticate_updates_last_used() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        let created = store.create_api_key(&user.id, None).await.unwrap();

        // Initially last_used_at should be None
        let keys = store.list_api_keys(&user.id).await.unwrap();
        assert!(keys[0].last_used_at.is_none());

        // After authentication, last_used_at should be set
        store.authenticate_api_key(&created.key).await.unwrap();
        let keys = store.list_api_keys(&user.id).await.unwrap();
        assert!(keys[0].last_used_at.is_some());
    }

    #[tokio::test]
    async fn multiple_keys_per_user() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        let k1 = store.create_api_key(&user.id, Some("key1")).await.unwrap();
        let k2 = store.create_api_key(&user.id, Some("key2")).await.unwrap();

        // Both keys should authenticate
        let u1 = store.authenticate_api_key(&k1.key).await.unwrap().unwrap();
        let u2 = store.authenticate_api_key(&k2.key).await.unwrap().unwrap();
        assert_eq!(u1.id, user.id);
        assert_eq!(u2.id, user.id);

        // Revoking one should not affect the other
        store.revoke_api_key(&k1.meta.id).await.unwrap();
        assert!(store.authenticate_api_key(&k1.key).await.unwrap().is_none());
        assert!(store.authenticate_api_key(&k2.key).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn telegram_relink_to_different_user() {
        let store = test_store().await;
        let alice = store.create_user(None, Some("Alice"), Role::User).await.unwrap();
        let bob = store.create_user(None, Some("Bob"), Role::User).await.unwrap();

        // Link to Alice
        store.link_telegram(&alice.id, 999, None).await.unwrap();
        let authed = store.authenticate_telegram(999).await.unwrap().unwrap();
        assert_eq!(authed.id, alice.id);

        // Relink same telegram_id to Bob (INSERT OR REPLACE)
        store.link_telegram(&bob.id, 999, None).await.unwrap();
        let authed = store.authenticate_telegram(999).await.unwrap().unwrap();
        assert_eq!(authed.id, bob.id, "Relink should point to the new user");

        // Should only be one link total
        let links = store.list_telegram_links().await.unwrap();
        assert_eq!(links.len(), 1);
    }

    #[tokio::test]
    async fn deactivated_user_telegram_auth_fails() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.link_telegram(&user.id, 111, None).await.unwrap();

        store.deactivate_user(&user.id).await.unwrap();

        let result = store.authenticate_telegram(111).await.unwrap();
        assert!(result.is_none(), "Deactivated user should not authenticate via Telegram");
    }

    #[tokio::test]
    async fn audit_log_entries_have_correct_fields() {
        let store = test_store().await;
        store.log_event(
            Some("uid"),
            "api_key_created",
            Some("label: test"),
            Some("10.0.0.1"),
        ).await.unwrap();

        let entries = store.recent_audit(1).await.unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.user_id.as_deref(), Some("uid"));
        assert_eq!(e.event_type, "api_key_created");
        assert_eq!(e.detail.as_deref(), Some("label: test"));
        assert_eq!(e.ip_address.as_deref(), Some("10.0.0.1"));
    }

    #[tokio::test]
    async fn audit_log_respects_limit() {
        let store = test_store().await;
        for i in 0..10 {
            store.log_event(None, &format!("event_{}", i), None, None).await.unwrap();
        }
        let entries = store.recent_audit(3).await.unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn get_telegram_link_for_user() {
        let store = test_store().await;
        let user = store.create_user(None, Some("Alice"), Role::User).await.unwrap();
        store.link_telegram(&user.id, 12345, Some("alice")).await.unwrap();

        let link = store.get_telegram_link_for_user(&user.id).await.unwrap().unwrap();
        assert_eq!(link.telegram_id, 12345);
        assert_eq!(link.username.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn get_telegram_link_for_user_none() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        let link = store.get_telegram_link_for_user(&user.id).await.unwrap();
        assert!(link.is_none());
    }

    #[tokio::test]
    async fn unlink_telegram_by_user() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.link_telegram(&user.id, 999, None).await.unwrap();

        store.unlink_telegram_by_user(&user.id).await.unwrap();
        let result = store.authenticate_telegram(999).await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn role_display() {
        assert_eq!(Role::Admin.to_string(), "admin");
        assert_eq!(Role::User.to_string(), "user");
    }

    #[test]
    fn role_from_str() {
        assert_eq!(Role::from_str("admin"), Some(Role::Admin));
        assert_eq!(Role::from_str("user"), Some(Role::User));
        assert_eq!(Role::from_str("unknown"), None);
    }

    #[test]
    fn role_serde_roundtrip() {
        let json = serde_json::to_string(&Role::Admin).unwrap();
        assert_eq!(json, "\"admin\"");
        let parsed: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Role::Admin);
    }

    #[test]
    fn user_serializes_correctly() {
        let user = User {
            id: "test-id".into(),
            email: Some("test@example.com".into()),
            display_name: Some("Test".into()),
            role: Role::User,
            is_active: true,
            filesystem_enabled: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&user).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["id"], "test-id");
        assert_eq!(parsed["role"], "user");
        assert_eq!(parsed["is_active"], true);
        assert_eq!(parsed["filesystem_enabled"], false);
    }

    // ── filesystem_enabled tests ────────────────────────────────────────

    #[tokio::test]
    async fn create_user_defaults_filesystem_disabled() {
        let store = test_store().await;
        let user = store.create_user(None, Some("Test"), Role::User).await.unwrap();
        assert!(!user.filesystem_enabled);

        let fetched = store.get_user(&user.id).await.unwrap().unwrap();
        assert!(!fetched.filesystem_enabled);
    }

    #[tokio::test]
    async fn update_user_enables_filesystem() {
        let store = test_store().await;
        let user = store.create_user(None, Some("Test"), Role::User).await.unwrap();
        assert!(!user.filesystem_enabled);

        store.update_user(&user.id, None, None, None, Some(true)).await.unwrap();
        let fetched = store.get_user(&user.id).await.unwrap().unwrap();
        assert!(fetched.filesystem_enabled);
    }

    #[tokio::test]
    async fn update_user_disables_filesystem() {
        let store = test_store().await;
        let user = store.create_user(None, Some("Test"), Role::User).await.unwrap();
        store.update_user(&user.id, None, None, None, Some(true)).await.unwrap();

        store.update_user(&user.id, None, None, None, Some(false)).await.unwrap();
        let fetched = store.get_user(&user.id).await.unwrap().unwrap();
        assert!(!fetched.filesystem_enabled);
    }

    #[tokio::test]
    async fn update_user_none_preserves_filesystem() {
        let store = test_store().await;
        let user = store.create_user(None, Some("Test"), Role::User).await.unwrap();
        store.update_user(&user.id, None, None, None, Some(true)).await.unwrap();

        // Update name only — filesystem should remain enabled
        store.update_user(&user.id, None, Some("NewName"), None, None).await.unwrap();
        let fetched = store.get_user(&user.id).await.unwrap().unwrap();
        assert!(fetched.filesystem_enabled);
        assert_eq!(fetched.display_name.as_deref(), Some("NewName"));
    }

    #[tokio::test]
    async fn list_users_includes_filesystem_field() {
        let store = test_store().await;
        let u1 = store.create_user(None, Some("A"), Role::User).await.unwrap();
        store.create_user(None, Some("B"), Role::User).await.unwrap();
        store.update_user(&u1.id, None, None, None, Some(true)).await.unwrap();

        let users = store.list_users().await.unwrap();
        assert_eq!(users.len(), 2);
        let a = users.iter().find(|u| u.display_name.as_deref() == Some("A")).unwrap();
        let b = users.iter().find(|u| u.display_name.as_deref() == Some("B")).unwrap();
        assert!(a.filesystem_enabled);
        assert!(!b.filesystem_enabled);
    }

    #[tokio::test]
    async fn bootstrap_admin_has_filesystem_enabled() {
        let store = test_store().await;
        let result = store.bootstrap_admin(None).await.unwrap();
        let (admin, _key) = result.unwrap();
        assert!(admin.filesystem_enabled, "Bootstrap admin should have filesystem enabled");

        // Verify it persists
        let fetched = store.get_user(&admin.id).await.unwrap().unwrap();
        assert!(fetched.filesystem_enabled);
    }

    #[tokio::test]
    async fn api_key_auth_returns_filesystem_field() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.update_user(&user.id, None, None, None, Some(true)).await.unwrap();
        let key = store.create_api_key(&user.id, None).await.unwrap();

        let authed = store.authenticate_api_key(&key.key).await.unwrap().unwrap();
        assert!(authed.filesystem_enabled);
    }

    #[tokio::test]
    async fn telegram_auth_returns_filesystem_field() {
        let store = test_store().await;
        let user = store.create_user(None, None, Role::User).await.unwrap();
        store.update_user(&user.id, None, None, None, Some(true)).await.unwrap();
        store.link_telegram(&user.id, 12345, None).await.unwrap();

        let authed = store.authenticate_telegram(12345).await.unwrap().unwrap();
        assert!(authed.filesystem_enabled);
    }

    #[test]
    fn user_serialization_includes_filesystem_enabled() {
        let user = User {
            id: "u1".into(),
            email: None,
            display_name: None,
            role: Role::Admin,
            is_active: true,
            filesystem_enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&user).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["filesystem_enabled"], true);
    }
}
