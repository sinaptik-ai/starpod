//! Slack Web API client for the outbound calls the bot needs.
//!
//! Milestone 2 needs only three endpoints:
//!
//! - `auth.test` — called once at startup to fetch our own `bot_user_id`
//!   so we can filter self-authored messages and avoid infinite loops.
//! - `chat.postMessage` — how we reply to users, posting into the same
//!   thread as the triggering message.
//! - `reactions.add` (optional convenience) — used by the handler to
//!   mark "thinking" / "done" states on the triggering message. Safe to
//!   ignore individual failures.
//!
//! All three are `Bearer xoxb-...` authenticated. The `xapp-...` token is
//! used only by `SlackApi::open_connection` in `connections.rs` — do not
//! mix them.
//!
//! This is a minimal wrapper — not a general-purpose Slack SDK. It stays
//! intentionally thin so that failure modes are easy to reason about and
//! the compile time is kept low.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::connections::SLACK_API_BASE;
use crate::error::{Result, SlackError};

/// Thin Slack Web API client bound to a bot token (`xoxb-...`).
///
/// Cloning is cheap — internally wraps a shared `reqwest::Client`.
#[derive(Clone)]
pub struct SlackWebClient {
    http: Client,
    bot_token: String,
    base_url: String,
}

impl SlackWebClient {
    /// Create a client for the given bot token. Uses a shared default
    /// `reqwest::Client` — callers that need custom timeouts should use
    /// [`SlackWebClient::with_http`] instead.
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            bot_token: bot_token.into(),
            base_url: SLACK_API_BASE.to_string(),
        }
    }

    /// Create a client with a caller-supplied HTTP client (for custom
    /// timeouts, proxies, or instrumentation).
    pub fn with_http(http: Client, bot_token: impl Into<String>) -> Self {
        Self {
            http,
            bot_token: bot_token.into(),
            base_url: SLACK_API_BASE.to_string(),
        }
    }

    /// Override the base URL. Used by integration tests to point at a
    /// local mock Slack API. Must NOT include a trailing `/`.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Call `auth.test` to validate the bot token and retrieve our own
    /// identity. The returned `user_id` is the one we use to filter
    /// self-authored messages.
    ///
    /// <https://docs.slack.dev/reference/methods/auth.test>
    pub async fn auth_test(&self) -> Result<AuthTestResponse> {
        let url = format!("{}/api/auth.test", self.base_url);
        let resp: RawAuthTest = self
            .http
            .post(&url)
            .headers(self.auth_headers())
            .send()
            .await?
            .json()
            .await?;

        if !resp.ok {
            return Err(SlackError::WebApi(
                resp.error.unwrap_or_else(|| "auth.test failed".into()),
            ));
        }
        Ok(AuthTestResponse {
            url: resp.url.unwrap_or_default(),
            team: resp.team.unwrap_or_default(),
            team_id: resp.team_id.unwrap_or_default(),
            user: resp.user.unwrap_or_default(),
            user_id: resp.user_id.unwrap_or_default(),
            bot_id: resp.bot_id.unwrap_or_default(),
        })
    }

    /// Post a message to a channel, optionally threaded under an existing
    /// message via `thread_ts`.
    ///
    /// Returns the `ts` of the posted message so the caller can chain
    /// follow-up operations (e.g. updates, reactions).
    ///
    /// <https://docs.slack.dev/reference/methods/chat.postMessage>
    pub async fn chat_post_message(&self, args: ChatPostMessageArgs<'_>) -> Result<String> {
        let url = format!("{}/api/chat.postMessage", self.base_url);

        // Slack's Web API accepts JSON only if the request is Bearer-auth
        // *and* explicitly sets the JSON content type. Form-encoded is
        // the historical default — both work, but JSON is simpler for
        // payloads with nested blocks.
        let body = ChatPostMessageBody {
            channel: args.channel,
            text: args.text,
            thread_ts: args.thread_ts,
            mrkdwn: args.mrkdwn,
            unfurl_links: args.unfurl_links,
            unfurl_media: args.unfurl_media,
        };

        let resp: RawPostMessage = self
            .http
            .post(&url)
            .headers(self.auth_headers_json())
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if !resp.ok {
            let err = resp
                .error
                .unwrap_or_else(|| "chat.postMessage failed".into());
            return Err(SlackError::WebApi(err));
        }
        debug!(
            channel = args.channel,
            ts = resp.ts.as_deref().unwrap_or("<no ts>"),
            "slack chat.postMessage ok"
        );
        Ok(resp.ts.unwrap_or_default())
    }

    /// Download a file uploaded to Slack via its `url_private` /
    /// `url_private_download` URL. Slack's file storage requires the bot
    /// token in the `Authorization: Bearer` header — anonymous fetches
    /// return an HTML login page, which is the most common foot-gun for
    /// "why is my image empty?" bugs.
    ///
    /// Requires the `files:read` OAuth scope on the bot user.
    pub async fn download_file(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self
            .http
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", self.bot_token))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            return Err(SlackError::WebApi(format!(
                "file download failed: HTTP {} for {}",
                status, url
            )));
        }

        // Defensive sanity check: if the response is HTML, the bot
        // token was rejected and Slack served the login page instead of
        // the file. Surface a helpful error rather than handing back
        // garbage bytes.
        if let Some(ct) = resp.headers().get(CONTENT_TYPE) {
            if let Ok(ct_str) = ct.to_str() {
                if ct_str.starts_with("text/html") {
                    return Err(SlackError::WebApi(format!(
                        "file download returned HTML (token likely missing files:read scope) for {}",
                        url
                    )));
                }
            }
        }

        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// Add an emoji reaction to a message. Errors are returned so the
    /// caller can decide whether to log or surface them; reactions are
    /// cosmetic, so most callers should treat failures as non-fatal.
    ///
    /// <https://docs.slack.dev/reference/methods/reactions.add>
    pub async fn reactions_add(&self, channel: &str, ts: &str, name: &str) -> Result<()> {
        let url = format!("{}/api/reactions.add", self.base_url);
        let body = serde_json::json!({
            "channel": channel,
            "timestamp": ts,
            "name": name,
        });
        let resp: RawOk = self
            .http
            .post(&url)
            .headers(self.auth_headers_json())
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if !resp.ok {
            return Err(SlackError::WebApi(
                resp.error.unwrap_or_else(|| "reactions.add failed".into()),
            ));
        }
        Ok(())
    }

    fn auth_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        let value = format!("Bearer {}", self.bot_token);
        if let Ok(v) = HeaderValue::from_str(&value) {
            h.insert(AUTHORIZATION, v);
        }
        h.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        h
    }

    fn auth_headers_json(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        let value = format!("Bearer {}", self.bot_token);
        if let Ok(v) = HeaderValue::from_str(&value) {
            h.insert(AUTHORIZATION, v);
        }
        h.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        h
    }
}

/// Arguments for [`SlackWebClient::chat_post_message`]. All fields use
/// borrowed types so callers don't have to allocate per call.
#[derive(Debug, Clone)]
pub struct ChatPostMessageArgs<'a> {
    /// Channel ID (`C...`, `G...`, `D...`) or user ID for DMs via IM.
    pub channel: &'a str,
    /// Message text — will be rendered as Slack mrkdwn if `mrkdwn` is true.
    pub text: &'a str,
    /// Parent message timestamp if this is a threaded reply.
    pub thread_ts: Option<&'a str>,
    /// Enable Slack's mrkdwn parsing (default: true).
    pub mrkdwn: bool,
    /// Unfurl link previews (default: true).
    pub unfurl_links: bool,
    /// Unfurl media previews (default: true).
    pub unfurl_media: bool,
}

impl<'a> ChatPostMessageArgs<'a> {
    /// Convenience constructor with sensible defaults (mrkdwn on, unfurl
    /// on).
    pub fn new(channel: &'a str, text: &'a str) -> Self {
        Self {
            channel,
            text,
            thread_ts: None,
            mrkdwn: true,
            unfurl_links: true,
            unfurl_media: true,
        }
    }

    /// Reply in the same thread as the triggering message.
    pub fn in_thread(mut self, thread_ts: &'a str) -> Self {
        self.thread_ts = Some(thread_ts);
        self
    }
}

/// Parsed result of `auth.test`.
#[derive(Debug, Clone)]
pub struct AuthTestResponse {
    pub url: String,
    pub team: String,
    pub team_id: String,
    pub user: String,
    pub user_id: String,
    pub bot_id: String,
}

// ── wire types (private) ────────────────────────────────────────────

#[derive(Serialize)]
struct ChatPostMessageBody<'a> {
    channel: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<&'a str>,
    mrkdwn: bool,
    unfurl_links: bool,
    unfurl_media: bool,
}

#[derive(Deserialize)]
struct RawOk {
    ok: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct RawPostMessage {
    ok: bool,
    error: Option<String>,
    ts: Option<String>,
}

#[derive(Deserialize)]
struct RawAuthTest {
    ok: bool,
    error: Option<String>,
    url: Option<String>,
    team: Option<String>,
    team_id: Option<String>,
    user: Option<String>,
    user_id: Option<String>,
    bot_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_args_defaults() {
        let a = ChatPostMessageArgs::new("C123", "hello");
        assert_eq!(a.channel, "C123");
        assert!(a.mrkdwn);
        assert!(a.thread_ts.is_none());
        let threaded = a.in_thread("1700000000.000100");
        assert_eq!(threaded.thread_ts, Some("1700000000.000100"));
    }

    #[test]
    fn post_message_body_skips_none_thread_ts() {
        let body = ChatPostMessageBody {
            channel: "C123",
            text: "hi",
            thread_ts: None,
            mrkdwn: true,
            unfurl_links: true,
            unfurl_media: true,
        };
        let s = serde_json::to_string(&body).unwrap();
        assert!(!s.contains("thread_ts"), "thread_ts should be elided");
    }

    #[test]
    fn post_message_body_includes_thread_ts_when_set() {
        let body = ChatPostMessageBody {
            channel: "C123",
            text: "hi",
            thread_ts: Some("1700000000.000100"),
            mrkdwn: true,
            unfurl_links: true,
            unfurl_media: true,
        };
        let s = serde_json::to_string(&body).unwrap();
        assert!(s.contains("1700000000.000100"));
    }
}
