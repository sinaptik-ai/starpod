//! Client for the Slack `apps.connections.open` Web API method.
//!
//! This is the only HTTP call we make in Milestone 1. It exchanges an
//! app-level token (`xapp-...`) for a single-use, short-lived `wss://` URL
//! that the Socket Mode receive loop then connects to.
//!
//! ## Token shape
//!
//! Slack app-level tokens always start with the literal prefix `xapp-`. We
//! validate this client-side before making any network call so that an
//! obviously malformed configuration value (e.g. a bot token `xoxb-...`
//! pasted into the wrong field) fails immediately with a clear error,
//! without ever sending the value to `slack.com`.
//!
//! ## Endpoint override
//!
//! The base URL is configurable via [`SlackApi::with_base_url`] so that the
//! integration test can point the client at a local mock server. In
//! production callers always use [`SlackApi::new`] which targets
//! `https://slack.com`.

use serde::Deserialize;

use crate::error::{Result, SlackError};

/// Production base URL for the Slack Web API.
pub const SLACK_API_BASE: &str = "https://slack.com";

/// Slack Web API client scoped to the calls `starpod-slack` needs.
///
/// In Milestone 1 the only method exposed is [`SlackApi::open_connection`].
/// Milestone 2 will add `chat.postMessage`, `auth.test`, and
/// `reactions.add`.
#[derive(Debug, Clone)]
pub struct SlackApi {
    http: reqwest::Client,
    base_url: String,
}

impl SlackApi {
    /// Construct a client targeting the real Slack API.
    pub fn new() -> Self {
        Self::with_http(reqwest::Client::new())
    }

    /// Construct a client with a caller-provided HTTP client.
    ///
    /// Useful for sharing a connection pool with other Slack-related
    /// callers in the same process.
    pub fn with_http(http: reqwest::Client) -> Self {
        Self {
            http,
            base_url: SLACK_API_BASE.to_string(),
        }
    }

    /// Override the base URL — used by tests to target a mock server.
    ///
    /// The base URL must NOT contain a trailing slash and must NOT include
    /// the API path. For example: `"http://127.0.0.1:54321"`.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Call `apps.connections.open` and return a fresh `wss://` URL.
    ///
    /// The returned URL is single-use and short-lived (Slack documents ~30s
    /// before it expires). The caller should pass it directly to the
    /// WebSocket connect step without delay.
    ///
    /// # Errors
    ///
    /// Returns [`SlackError::InvalidAppToken`] if `app_token` does not start
    /// with `xapp-` (validated before any network call).
    ///
    /// Returns [`SlackError::Http`] for transport-level failures.
    ///
    /// Returns [`SlackError::ConnectionsOpen`] if Slack returns a non-2xx
    /// response or `ok: false`. The wrapped string is Slack's `error`
    /// field (e.g. `"invalid_auth"`, `"not_allowed_token_type"`,
    /// `"missing_scope"`) or a synthetic message describing the HTTP
    /// failure.
    pub async fn open_connection(&self, app_token: &str) -> Result<String> {
        if !app_token.starts_with("xapp-") {
            return Err(SlackError::InvalidAppToken(
                "expected xapp-... app-level token",
            ));
        }

        let url = format!("{}/api/apps.connections.open", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(app_token)
            // Slack requires this header even though the body is empty.
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=utf-8",
            )
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            return Err(SlackError::ConnectionsOpen(format!("HTTP {status}")));
        }

        let parsed: AppsConnectionsOpenResponse = resp.json().await?;
        if !parsed.ok {
            return Err(SlackError::ConnectionsOpen(
                parsed.error.unwrap_or_else(|| "unknown_error".to_string()),
            ));
        }
        parsed
            .url
            .ok_or_else(|| SlackError::ConnectionsOpen("missing url field in response".into()))
    }
}

impl Default for SlackApi {
    fn default() -> Self {
        Self::new()
    }
}

/// JSON shape returned by `apps.connections.open`.
///
/// On success: `{ "ok": true, "url": "wss://..." }`.
/// On failure: `{ "ok": false, "error": "invalid_auth" }`.
#[derive(Debug, Deserialize)]
struct AppsConnectionsOpenResponse {
    ok: bool,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_token_without_xapp_prefix() {
        let api = SlackApi::new();
        let err = api
            .open_connection("xoxb-not-an-app-token")
            .await
            .unwrap_err();
        match err {
            SlackError::InvalidAppToken(_) => {}
            other => panic!("expected InvalidAppToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_empty_token() {
        let api = SlackApi::new();
        assert!(matches!(
            api.open_connection("").await.unwrap_err(),
            SlackError::InvalidAppToken(_)
        ));
    }

    #[tokio::test]
    async fn parses_success_response() {
        let body = r#"{"ok":true,"url":"wss://wss-primary.slack.com/link/?ticket=abc"}"#;
        let parsed: AppsConnectionsOpenResponse = serde_json::from_str(body).unwrap();
        assert!(parsed.ok);
        assert_eq!(
            parsed.url.as_deref(),
            Some("wss://wss-primary.slack.com/link/?ticket=abc")
        );
        assert!(parsed.error.is_none());
    }

    #[tokio::test]
    async fn parses_error_response() {
        let body = r#"{"ok":false,"error":"invalid_auth"}"#;
        let parsed: AppsConnectionsOpenResponse = serde_json::from_str(body).unwrap();
        assert!(!parsed.ok);
        assert_eq!(parsed.error.as_deref(), Some("invalid_auth"));
        assert!(parsed.url.is_none());
    }
}
