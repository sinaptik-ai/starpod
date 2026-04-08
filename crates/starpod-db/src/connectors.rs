//! Connector store — CRUD for the `connectors` table in `core.db`.

use std::collections::HashMap;

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use starpod_core::{Result, StarpodError};

// ── Row type ────────────────────────────────────────────────────────────────

/// A connector instance as stored in the database.
#[derive(Debug, Clone)]
pub struct ConnectorRow {
    pub name: String,
    pub connector_type: String,
    pub display_name: String,
    pub description: String,
    pub auth_method: String,
    /// Resolved vault key names (e.g. `["GITHUB_TOKEN"]`).
    pub secrets: Vec<String>,
    /// Config key-value pairs (e.g. `{"base_url": "https://api.github.com"}`).
    pub config: HashMap<String, String>,
    pub oauth_token_url: Option<String>,
    pub oauth_token_key: Option<String>,
    pub oauth_refresh_key: Option<String>,
    pub oauth_expires_at: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

// ── Store ───────────────────────────────────────────────────────────────────

/// Manages connector rows in `core.db`.
///
/// Created from a `SqlitePool` (via `CoreDb::pool().clone()`).
#[derive(Clone)]
pub struct ConnectorStore {
    pool: SqlitePool,
}

impl ConnectorStore {
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// List all configured connectors.
    pub async fn list(&self) -> Result<Vec<ConnectorRow>> {
        let rows = sqlx::query(
            "SELECT name, type, display_name, description, auth_method, \
             secrets, config, oauth_token_url, oauth_token_key, \
             oauth_refresh_key, oauth_expires_at, status, \
             created_at, updated_at \
             FROM connectors ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("connector list: {e}")))?;

        rows.iter().map(row_to_connector).collect()
    }

    /// Get a single connector by name.
    pub async fn get(&self, name: &str) -> Result<Option<ConnectorRow>> {
        let row = sqlx::query(
            "SELECT name, type, display_name, description, auth_method, \
             secrets, config, oauth_token_url, oauth_token_key, \
             oauth_refresh_key, oauth_expires_at, status, \
             created_at, updated_at \
             FROM connectors WHERE name = ?1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("connector get: {e}")))?;

        row.as_ref().map(row_to_connector).transpose()
    }

    /// Insert a new connector.
    pub async fn insert(&self, row: &ConnectorRow) -> Result<()> {
        let secrets_json = serde_json::to_string(&row.secrets).unwrap_or_default();
        let config_json = serde_json::to_string(&row.config).unwrap_or_default();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO connectors \
             (name, type, display_name, description, auth_method, \
              secrets, config, oauth_token_url, oauth_token_key, \
              oauth_refresh_key, oauth_expires_at, status, \
              created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .bind(&row.name)
        .bind(&row.connector_type)
        .bind(&row.display_name)
        .bind(&row.description)
        .bind(&row.auth_method)
        .bind(&secrets_json)
        .bind(&config_json)
        .bind(&row.oauth_token_url)
        .bind(&row.oauth_token_key)
        .bind(&row.oauth_refresh_key)
        .bind(&row.oauth_expires_at)
        .bind(&row.status)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("connector insert: {e}")))?;

        Ok(())
    }

    /// Update the status of a connector.
    pub async fn update_status(&self, name: &str, status: &str) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let result =
            sqlx::query("UPDATE connectors SET status = ?1, updated_at = ?2 WHERE name = ?3")
                .bind(status)
                .bind(&now)
                .bind(name)
                .execute(&self.pool)
                .await
                .map_err(|e| StarpodError::Database(format!("connector update_status: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Update OAuth expiry timestamp.
    pub async fn update_oauth_expiry(&self, name: &str, expires_at: &str) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE connectors SET oauth_expires_at = ?1, updated_at = ?2 WHERE name = ?3",
        )
        .bind(expires_at)
        .bind(&now)
        .bind(name)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("connector update_oauth_expiry: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Update the OAuth refresh key vault reference.
    pub async fn update_oauth_refresh_key(&self, name: &str, refresh_key: &str) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE connectors SET oauth_refresh_key = ?1, updated_at = ?2 WHERE name = ?3",
        )
        .bind(refresh_key)
        .bind(&now)
        .bind(name)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("connector update_oauth_refresh_key: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Update a connector's config overrides.
    pub async fn update_config(
        &self,
        name: &str,
        config: &HashMap<String, String>,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let config_json = serde_json::to_string(config).unwrap_or_default();
        let result =
            sqlx::query("UPDATE connectors SET config = ?1, updated_at = ?2 WHERE name = ?3")
                .bind(&config_json)
                .bind(&now)
                .bind(name)
                .execute(&self.pool)
                .await
                .map_err(|e| StarpodError::Database(format!("connector update_config: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete a connector by name.
    pub async fn delete(&self, name: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM connectors WHERE name = ?1")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("connector delete: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn row_to_connector(row: &sqlx::sqlite::SqliteRow) -> Result<ConnectorRow> {
    let secrets_str: String = row.get("secrets");
    let config_str: String = row.get("config");

    let secrets: Vec<String> = serde_json::from_str(&secrets_str).unwrap_or_default();
    let config: HashMap<String, String> = serde_json::from_str(&config_str).unwrap_or_default();

    Ok(ConnectorRow {
        name: row.get("name"),
        connector_type: row.get("type"),
        display_name: row.get("display_name"),
        description: row.get("description"),
        auth_method: row.get("auth_method"),
        secrets,
        config,
        oauth_token_url: row.get("oauth_token_url"),
        oauth_token_key: row.get("oauth_token_key"),
        oauth_refresh_key: row.get("oauth_refresh_key"),
        oauth_expires_at: row.get("oauth_expires_at"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use starpod_db_test_pool::test_pool;

    /// Create a test pool with core.db migrations applied.
    mod starpod_db_test_pool {
        use sqlx::SqlitePool;

        pub async fn test_pool() -> SqlitePool {
            let db = crate::CoreDb::in_memory().await.unwrap();
            db.pool().clone()
        }
    }

    #[tokio::test]
    async fn crud_lifecycle() {
        let pool = test_pool().await;
        let store = ConnectorStore::from_pool(pool);

        // Initially empty
        let list = store.list().await.unwrap();
        assert!(list.is_empty());

        // Insert
        let row = ConnectorRow {
            name: "github".into(),
            connector_type: "github".into(),
            display_name: "GitHub".into(),
            description: "GitHub access".into(),
            auth_method: "token".into(),
            secrets: vec!["GITHUB_TOKEN".into()],
            config: [("base_url".into(), "https://api.github.com".into())]
                .into_iter()
                .collect(),
            oauth_token_url: None,
            oauth_token_key: None,
            oauth_refresh_key: None,
            oauth_expires_at: None,
            status: "connected".into(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        store.insert(&row).await.unwrap();

        // Get
        let fetched = store.get("github").await.unwrap().unwrap();
        assert_eq!(fetched.name, "github");
        assert_eq!(fetched.connector_type, "github");
        assert_eq!(fetched.secrets, vec!["GITHUB_TOKEN"]);
        assert_eq!(
            fetched.config.get("base_url").unwrap(),
            "https://api.github.com"
        );

        // List
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 1);

        // Update status
        store.update_status("github", "error").await.unwrap();
        let fetched = store.get("github").await.unwrap().unwrap();
        assert_eq!(fetched.status, "error");

        // Delete
        let deleted = store.delete("github").await.unwrap();
        assert!(deleted);
        assert!(store.get("github").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn multi_instance() {
        let pool = test_pool().await;
        let store = ConnectorStore::from_pool(pool);

        for (name, secret) in [
            ("analytics-db", "ANALYTICS_DB_DATABASE_URL"),
            ("users-db", "USERS_DB_DATABASE_URL"),
        ] {
            let row = ConnectorRow {
                name: name.into(),
                connector_type: "postgres".into(),
                display_name: "PostgreSQL".into(),
                description: format!("{} database", name),
                auth_method: "token".into(),
                secrets: vec![secret.into()],
                config: HashMap::new(),
                oauth_token_url: None,
                oauth_token_key: None,
                oauth_refresh_key: None,
                oauth_expires_at: None,
                status: "connected".into(),
                created_at: String::new(),
                updated_at: String::new(),
            };
            store.insert(&row).await.unwrap();
        }

        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "analytics-db");
        assert_eq!(list[1].name, "users-db");
    }

    #[tokio::test]
    async fn oauth_connector() {
        let pool = test_pool().await;
        let store = ConnectorStore::from_pool(pool);

        let row = ConnectorRow {
            name: "google-calendar".into(),
            connector_type: "google-calendar".into(),
            display_name: "Google Calendar".into(),
            description: "Calendar access".into(),
            auth_method: "oauth".into(),
            secrets: vec!["GOOGLE_CALENDAR_TOKEN".into()],
            config: HashMap::new(),
            oauth_token_url: Some("https://oauth2.googleapis.com/token".into()),
            oauth_token_key: Some("GOOGLE_CALENDAR_TOKEN".into()),
            oauth_refresh_key: Some("GOOGLE_CALENDAR_REFRESH_TOKEN".into()),
            oauth_expires_at: Some("2026-04-01T14:00:00Z".into()),
            status: "connected".into(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        store.insert(&row).await.unwrap();

        let fetched = store.get("google-calendar").await.unwrap().unwrap();
        assert_eq!(fetched.auth_method, "oauth");
        assert_eq!(
            fetched.oauth_refresh_key.as_deref(),
            Some("GOOGLE_CALENDAR_REFRESH_TOKEN")
        );

        // Update expiry
        store
            .update_oauth_expiry("google-calendar", "2026-04-02T14:00:00Z")
            .await
            .unwrap();
        let fetched = store.get("google-calendar").await.unwrap().unwrap();
        assert_eq!(
            fetched.oauth_expires_at.as_deref(),
            Some("2026-04-02T14:00:00Z")
        );
    }
}
