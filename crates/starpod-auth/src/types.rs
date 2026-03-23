//! Core types for the authentication system.
//!
//! All types derive `Serialize`/`Deserialize` for API responses and are
//! independent of the storage layer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// User role controlling access to protected endpoints.
///
/// - `Admin` — full access, including settings and user management.
/// - `User`  — standard chat access; cannot modify settings or manage users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    User,
}

impl Role {
    /// Convert to a lowercase string for database storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::User => "user",
        }
    }

    /// Parse from a lowercase string (as stored in the database).
    /// Returns `None` for unrecognized values.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Role::Admin),
            "user" => Some(Role::User),
            _ => None,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A registered user in the auth database.
///
/// Users are identified by UUID. Deactivated users (`is_active == false`)
/// cannot authenticate via API keys or Telegram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    /// Optional email (unique if set).
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: Role,
    /// `false` means the user is soft-deleted — all auth attempts will fail.
    pub is_active: bool,
    /// Whether this user can browse the instance filesystem via the web UI.
    pub filesystem_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Metadata about an API key (never includes the actual key or hash).
///
/// Returned by listing endpoints. The `prefix` field (first 8 hex chars of the
/// random part) is safe to display — it helps users identify which key is which
/// without revealing the full key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyMeta {
    pub id: String,
    pub user_id: String,
    /// First 8 hex chars of the key (after `sp_live_`), used for DB lookup.
    pub prefix: String,
    /// Optional human-readable label.
    pub label: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// Updated on each successful authentication.
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Returned only at API key creation time — contains the full plaintext key.
///
/// The key is never stored or retrievable after creation. The caller must
/// save it immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCreated {
    pub meta: ApiKeyMeta,
    /// The full API key — shown only once, never stored.
    pub key: String,
}

/// A linked Telegram account.
///
/// One Telegram ID maps to exactly one user. Relinking replaces the previous
/// association (INSERT OR REPLACE).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramLink {
    pub telegram_id: i64,
    pub user_id: String,
    pub username: Option<String>,
    pub linked_at: DateTime<Utc>,
}

/// An entry in the auth audit log.
///
/// `user_id` is `None` for events where the user could not be identified
/// (e.g. failed authentication with an unknown key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub user_id: Option<String>,
    pub event_type: String,
    pub detail: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}
