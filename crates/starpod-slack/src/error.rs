//! Error type for the `starpod-slack` crate.
//!
//! The crate uses a single [`SlackError`] enum that implements [`From`]
//! conversions for the underlying transport errors (`reqwest`, `tungstenite`,
//! `serde_json`). At the public API boundary, errors are converted into
//! [`starpod_core::StarpodError::Channel`] so they fit the workspace-wide
//! `Result` type.

use thiserror::Error;

/// All errors produced by the Slack Socket Mode client.
#[derive(Debug, Error)]
pub enum SlackError {
    /// `apps.connections.open` returned `ok: false` or an HTTP error.
    ///
    /// The `String` is the `error` field from the Slack response (e.g.
    /// `"invalid_auth"`, `"not_allowed_token_type"`) or a synthetic message
    /// describing an HTTP-level failure.
    #[error("apps.connections.open failed: {0}")]
    ConnectionsOpen(String),

    /// HTTP transport error talking to `slack.com`.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// WebSocket transport error.
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    /// Failed to parse a JSON envelope received from Slack.
    #[error("envelope parse error: {0}")]
    Envelope(#[from] serde_json::Error),

    /// Slack closed the WebSocket without sending a `disconnect` message.
    ///
    /// Treated as a transient error — the outer reconnect loop will retry
    /// with exponential backoff.
    #[error("WebSocket closed unexpectedly")]
    UnexpectedClose,

    /// The configured `xapp-...` token is missing or has the wrong shape.
    ///
    /// Slack app-level tokens always start with `xapp-`. Anything else is
    /// rejected before the first network call so we don't leak credentials
    /// in error messages from the upstream API.
    #[error("invalid app-level token: {0}")]
    InvalidAppToken(&'static str),

    /// A Slack Web API call returned `ok: false`.
    ///
    /// The `String` carries the `error` field from the response (e.g.
    /// `"channel_not_found"`, `"not_in_channel"`, `"invalid_auth"`).
    #[error("Slack Web API error: {0}")]
    WebApi(String),

    /// SQLite error in the dedup store.
    #[error("dedup database error: {0}")]
    Database(String),
}

impl SlackError {
    /// Whether the outer reconnect loop should retry after this error.
    ///
    /// Transient errors (network blips, unexpected closes) return `true`.
    /// Permanent errors (invalid token, malformed envelope from Slack —
    /// which would be a Slack-side bug) return `false`.
    pub fn is_retriable(&self) -> bool {
        match self {
            SlackError::Http(_) | SlackError::WebSocket(_) | SlackError::UnexpectedClose => true,
            // `ConnectionsOpen` covers both transient HTTP failures and
            // permanent auth failures. We treat it as retriable; the caller
            // can decide to surface persistent failures via metrics.
            SlackError::ConnectionsOpen(_) => true,
            // A malformed envelope is almost certainly a transient
            // protocol issue or a new envelope type we don't recognise.
            // We retry rather than crash the bot.
            SlackError::Envelope(_) => true,
            // Configuration errors are permanent — no point reconnecting.
            SlackError::InvalidAppToken(_) => false,
            // Web API failures (chat.postMessage, auth.test) are caller
            // errors, not transport errors. We keep the receive loop
            // running; the caller decides what to do with the specific
            // call's failure.
            SlackError::WebApi(_) => true,
            // Transient SQLite issues (locks, I/O) shouldn't kill the bot.
            SlackError::Database(_) => true,
        }
    }
}

impl From<SlackError> for starpod_core::StarpodError {
    fn from(err: SlackError) -> Self {
        starpod_core::StarpodError::Channel(format!("slack: {err}"))
    }
}

/// Crate-internal `Result` alias.
pub type Result<T> = std::result::Result<T, SlackError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_app_token_is_not_retriable() {
        let err = SlackError::InvalidAppToken("missing prefix");
        assert!(!err.is_retriable());
    }

    #[test]
    fn unexpected_close_is_retriable() {
        let err = SlackError::UnexpectedClose;
        assert!(err.is_retriable());
    }

    #[test]
    fn connections_open_is_retriable() {
        let err = SlackError::ConnectionsOpen("invalid_auth".into());
        assert!(err.is_retriable());
    }

    #[test]
    fn converts_to_starpod_channel_error() {
        let err = SlackError::InvalidAppToken("test");
        let core_err: starpod_core::StarpodError = err.into();
        assert!(matches!(core_err, starpod_core::StarpodError::Channel(_)));
    }
}
