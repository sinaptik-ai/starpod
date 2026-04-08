//! Connector template parser.
//!
//! Templates are `.toml` files in `.starpod/connectors/` that describe what a
//! service needs to connect. They are consumed once during setup to produce a
//! connector row in the database.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Result, StarpodError};

// ── Types ───────────────────────────────────────────────────────────────────

/// OAuth configuration for connectors that support browser-based sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub authorize_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    /// Vault key where the access token is stored after OAuth completes.
    pub token_key: String,
    /// Vault key for the refresh token. Presence enables auto-refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_key: Option<String>,
    /// Vault key for user-provided OAuth client ID.
    /// If omitted, Starpod uses its built-in OAuth app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id_key: Option<String>,
    /// Vault key for user-provided OAuth client secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret_key: Option<String>,
}

/// A connector template — describes a service and how to connect to it.
///
/// Loaded from `.toml` files in `.starpod/connectors/`. Read once during
/// connector setup, never at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorTemplate {
    /// Unique identifier (lowercase alphanumeric + hyphens).
    pub name: String,
    /// Human-readable label for UI.
    pub display_name: String,
    /// What this connector provides.
    pub description: String,
    /// Whether multiple instances can be created. When true, setup asks for an
    /// instance name and namespaces vault keys.
    #[serde(default)]
    pub multi_instance: bool,
    /// Logical vault key names required at runtime.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Logical vault keys that enhance functionality but aren't required.
    #[serde(default)]
    pub optional_secrets: Vec<String>,
    /// Default non-secret configuration values, overridable per-instance.
    #[serde(default)]
    pub config: HashMap<String, String>,
    /// OAuth setup path (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthConfig>,
    /// Socket Mode flag — connector uses an outbound WebSocket and is set up
    /// via a guided manifest install rather than OAuth distribution.
    /// Currently used by the Slack template.
    #[serde(default)]
    pub socket_mode: bool,
}

// ── Loading ─────────────────────────────────────────────────────────────────

/// Load a single connector template from a `.toml` file.
pub fn load_template(path: &Path) -> Result<ConnectorTemplate> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        StarpodError::Config(format!(
            "Failed to read connector template {}: {}",
            path.display(),
            e
        ))
    })?;
    let template: ConnectorTemplate = toml::from_str(&content).map_err(|e| {
        StarpodError::Config(format!(
            "Failed to parse connector template {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(template)
}

/// Load all `.toml` connector templates from a directory.
pub fn load_all_templates(dir: &Path) -> Result<Vec<ConnectorTemplate>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut templates = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| {
        StarpodError::Config(format!(
            "Failed to read connectors directory {}: {}",
            dir.display(),
            e
        ))
    })?;
    for entry in entries {
        let entry = entry
            .map_err(|e| StarpodError::Config(format!("Failed to read directory entry: {}", e)))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            match load_template(&path) {
                Ok(t) => templates.push(t),
                Err(e) => {
                    tracing::warn!(
                        "Skipping invalid connector template {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    }
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(templates)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_template() {
        let toml = r#"
name = "stripe"
display_name = "Stripe"
description = "Stripe payments API"
secrets = ["STRIPE_SECRET_KEY"]

[config]
base_url = "https://api.stripe.com"
"#;
        let t: ConnectorTemplate = toml::from_str(toml).unwrap();
        assert_eq!(t.name, "stripe");
        assert_eq!(t.display_name, "Stripe");
        assert!(!t.multi_instance);
        assert_eq!(t.secrets, vec!["STRIPE_SECRET_KEY"]);
        assert_eq!(t.config.get("base_url").unwrap(), "https://api.stripe.com");
        assert!(t.oauth.is_none());
    }

    #[test]
    fn parse_multi_instance_template() {
        let toml = r#"
name = "postgres"
display_name = "PostgreSQL"
description = "PostgreSQL database connection"
multi_instance = true
secrets = ["DATABASE_URL"]
"#;
        let t: ConnectorTemplate = toml::from_str(toml).unwrap();
        assert!(t.multi_instance);
        assert_eq!(t.secrets, vec!["DATABASE_URL"]);
    }

    #[test]
    fn parse_oauth_template() {
        let toml = r#"
name = "github"
display_name = "GitHub"
description = "GitHub access"
secrets = ["GITHUB_TOKEN"]

[oauth]
authorize_url = "https://github.com/login/oauth/authorize"
token_url = "https://github.com/login/oauth/access_token"
scopes = ["repo", "read:org"]
token_key = "GITHUB_TOKEN"
"#;
        let t: ConnectorTemplate = toml::from_str(toml).unwrap();
        let oauth = t.oauth.unwrap();
        assert_eq!(oauth.token_key, "GITHUB_TOKEN");
        assert_eq!(oauth.scopes, vec!["repo", "read:org"]);
        assert!(oauth.refresh_key.is_none());
    }

    #[test]
    fn parse_oauth_only_template() {
        let toml = r#"
name = "google-calendar"
display_name = "Google Calendar"
description = "Google Calendar access"

[oauth]
authorize_url = "https://accounts.google.com/o/oauth2/v2/auth"
token_url = "https://oauth2.googleapis.com/token"
scopes = ["https://www.googleapis.com/auth/calendar.readonly"]
token_key = "GOOGLE_CALENDAR_TOKEN"
refresh_key = "GOOGLE_CALENDAR_REFRESH_TOKEN"
"#;
        let t: ConnectorTemplate = toml::from_str(toml).unwrap();
        assert!(t.secrets.is_empty());
        let oauth = t.oauth.unwrap();
        assert_eq!(
            oauth.refresh_key.as_deref(),
            Some("GOOGLE_CALENDAR_REFRESH_TOKEN")
        );
    }

    #[test]
    fn load_all_from_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let templates = load_all_templates(dir.path()).unwrap();
        assert!(templates.is_empty());
    }

    #[test]
    fn load_all_from_dir_with_templates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("stripe.toml"),
            r#"
name = "stripe"
display_name = "Stripe"
description = "Payments"
secrets = ["STRIPE_SECRET_KEY"]
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("postgres.toml"),
            r#"
name = "postgres"
display_name = "PostgreSQL"
description = "Database"
multi_instance = true
secrets = ["DATABASE_URL"]
"#,
        )
        .unwrap();
        // Non-toml file should be ignored
        std::fs::write(dir.path().join("readme.md"), "# ignore me").unwrap();

        let templates = load_all_templates(dir.path()).unwrap();
        assert_eq!(templates.len(), 2);
        // Sorted alphabetically
        assert_eq!(templates[0].name, "postgres");
        assert_eq!(templates[1].name, "stripe");
    }

    #[test]
    fn load_all_skips_invalid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.toml"), "not valid toml {{{}}}").unwrap();
        std::fs::write(
            dir.path().join("good.toml"),
            r#"
name = "good"
display_name = "Good"
description = "Works"
"#,
        )
        .unwrap();
        let templates = load_all_templates(dir.path()).unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "good");
    }

    #[test]
    fn nonexistent_dir_returns_empty() {
        let templates = load_all_templates(Path::new("/nonexistent/path")).unwrap();
        assert!(templates.is_empty());
    }
}
