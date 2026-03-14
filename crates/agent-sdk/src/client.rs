//! Anthropic Messages API client with streaming, retries, and provider detection.

use std::env;
use std::pin::Pin;
use std::time::Duration;

use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tokio_stream::StreamExt;
use tracing::{debug, warn};

use crate::error::{AgentError, Result};

// ---------------------------------------------------------------------------
// Provider detection
// ---------------------------------------------------------------------------

/// Which backend provider to use for the Messages API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    Bedrock,
    Vertex,
}

impl Provider {
    /// Detect the provider from environment variables.
    ///
    /// - `CLAUDE_CODE_USE_BEDROCK=1` -> [`Provider::Bedrock`]
    /// - `CLAUDE_CODE_USE_VERTEX=1`  -> [`Provider::Vertex`]
    /// - otherwise                   -> [`Provider::Anthropic`]
    pub fn from_env() -> Self {
        if env::var("CLAUDE_CODE_USE_BEDROCK")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            Provider::Bedrock
        } else if env::var("CLAUDE_CODE_USE_VERTEX")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            Provider::Vertex
        } else {
            Provider::Anthropic
        }
    }
}

// ---------------------------------------------------------------------------
// API types – request
// ---------------------------------------------------------------------------

/// Parameters for extended-thinking / budget control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingParam {
    /// The thinking budget token (e.g. `"enabled"` or a specific budget).
    #[serde(rename = "type")]
    pub kind: String,
    /// Optional budget tokens limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u64>,
}

/// Cache control marker for prompt caching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub kind: String,
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self {
            kind: "ephemeral".to_string(),
        }
    }
}

/// A tool definition sent with the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// A system prompt content block (supports cache_control).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// A single content block inside an API message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ApiContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },

    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },

    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

/// A message in the conversation sent to the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMessage {
    pub role: String,
    pub content: Vec<ApiContentBlock>,
}

/// The full request body for `POST /v1/messages`.
#[derive(Debug, Clone, Serialize)]
pub struct CreateMessageRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<SystemBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingParam>,
}

// ---------------------------------------------------------------------------
// API types – response
// ---------------------------------------------------------------------------

/// Token usage returned by the API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
}

/// A full (non-streaming) response from the Messages API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageResponse {
    pub id: String,
    pub role: String,
    pub content: Vec<ApiContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: ApiUsage,
}

/// Error payload returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
}

/// Error wrapper as returned in the top-level JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    pub error: ApiError,
}

// ---------------------------------------------------------------------------
// Streaming types
// ---------------------------------------------------------------------------

/// Delta for a text content block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },

    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },

    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
}

/// Delta that comes with `message_delta` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDelta {
    pub stop_reason: Option<String>,
}

/// Server-sent events emitted during streaming.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    MessageStart {
        message: MessageResponse,
    },
    ContentBlockStart {
        index: usize,
        content_block: ApiContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: ContentDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: MessageDelta,
        usage: ApiUsage,
    },
    MessageStop,
    Ping,
    Error {
        error: ApiError,
    },
}

// Raw JSON shapes coming off the SSE wire (used for deserialization only).

#[derive(Deserialize)]
struct RawMessageStart {
    message: MessageResponse,
}

#[derive(Deserialize)]
struct RawContentBlockStart {
    index: usize,
    content_block: ApiContentBlock,
}

#[derive(Deserialize)]
struct RawContentBlockDelta {
    index: usize,
    delta: ContentDelta,
}

#[derive(Deserialize)]
struct RawContentBlockStop {
    index: usize,
}

#[derive(Deserialize)]
struct RawMessageDelta {
    delta: MessageDelta,
    usage: ApiUsage,
}

#[derive(Deserialize)]
struct RawError {
    error: ApiError,
}

// ---------------------------------------------------------------------------
// Retry configuration
// ---------------------------------------------------------------------------

/// Configuration for exponential-backoff retries.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (not counting the initial request).
    pub max_retries: u32,
    /// Initial back-off duration.
    pub initial_backoff: Duration,
    /// Multiplicative factor applied after each attempt.
    pub backoff_multiplier: f64,
    /// Upper bound on the back-off duration.
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(60),
        }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Anthropic Messages API client.
///
/// Supports non-streaming and streaming requests, automatic retries for
/// rate-limit (429) and overloaded (529) responses, and provider detection
/// for Bedrock / Vertex environments.
pub struct ApiClient {
    http: reqwest::Client,
    api_key: String,
    api_url: String,
    provider: Provider,
    retry_config: RetryConfig,
}

impl ApiClient {
    // -- constructors -------------------------------------------------------

    /// Create a new client reading `ANTHROPIC_API_KEY` from the environment.
    ///
    /// Returns [`AgentError::AuthenticationFailed`] when the variable is
    /// missing or empty.
    pub fn new() -> Result<Self> {
        let key = env::var("ANTHROPIC_API_KEY").map_err(|_| {
            AgentError::AuthenticationFailed(
                "ANTHROPIC_API_KEY environment variable is not set".into(),
            )
        })?;
        if key.is_empty() {
            return Err(AgentError::AuthenticationFailed(
                "ANTHROPIC_API_KEY environment variable is empty".into(),
            ));
        }
        Ok(Self::build(key))
    }

    /// Create a client with an explicit API key.
    pub fn with_api_key(key: impl Into<String>) -> Self {
        Self::build(key.into())
    }

    fn build(api_key: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            api_url: ANTHROPIC_API_URL.to_string(),
            provider: Provider::from_env(),
            retry_config: RetryConfig::default(),
        }
    }

    // -- configuration helpers ----------------------------------------------

    /// Override the default retry configuration.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Override the API base URL (useful for testing or proxies).
    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    /// Return the detected provider.
    pub fn provider(&self) -> Provider {
        self.provider
    }

    // -- request helpers ----------------------------------------------------

    fn default_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key).map_err(|_| {
                AgentError::AuthenticationFailed(
                    "API key contains invalid header characters".into(),
                )
            })?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_API_VERSION),
        );
        Ok(headers)
    }

    /// Determine the back-off duration for a given attempt.
    fn backoff_duration(&self, attempt: u32) -> Duration {
        let secs =
            self.retry_config.initial_backoff.as_secs_f64() * self.retry_config.backoff_multiplier.powi(attempt as i32);
        let max_secs = self.retry_config.max_backoff.as_secs_f64();
        // Clamp before converting to Duration to avoid panic on infinity/NaN.
        let clamped = secs.min(max_secs);
        Duration::from_secs_f64(clamped)
    }

    /// Return `true` for status codes that should be retried.
    fn is_retryable(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS || status.as_u16() == 529
    }

    /// Parse an `Retry-After` header (seconds) into a [`Duration`], if present.
    fn retry_after_duration(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
        headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
    }

    // -- non-streaming ------------------------------------------------------

    /// Send a non-streaming request to the Messages API and return the
    /// complete [`MessageResponse`].
    ///
    /// Automatically retries on 429 / 529 with exponential back-off.
    pub async fn create_message(&self, request: &CreateMessageRequest) -> Result<MessageResponse> {
        let mut req = request.clone();
        req.stream = false;

        let body = serde_json::to_value(&req)?;

        let mut attempt: u32 = 0;
        loop {
            let response = self
                .http
                .post(&self.api_url)
                .headers(self.default_headers()?)
                .json(&body)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let msg: MessageResponse = response.json().await?;
                return Ok(msg);
            }

            // Read body for error details before deciding to retry.
            let error_body = response.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if Self::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = Self::retry_after_duration(
                    // We already consumed the response; parse from the error
                    // body if needed. The header was on the response itself,
                    // but we don't have it anymore. Fall back to exponential.
                    &HeaderMap::new(),
                )
                .unwrap_or_else(|| self.backoff_duration(attempt));
                warn!(
                    status = status.as_u16(),
                    attempt,
                    wait_secs = wait.as_secs_f64(),
                    "Retryable API error, backing off"
                );
                sleep(wait).await;
                attempt += 1;
                continue;
            }

            return Err(Self::status_to_error(status, &error_text));
        }
    }

    // -- streaming ----------------------------------------------------------

    /// Send a streaming request and return an async [`Stream`] of
    /// [`StreamEvent`] items.
    ///
    /// Retries are handled at the connection level: if the initial request
    /// receives a 429 / 529 before any bytes are sent, the client backs off
    /// and retries. Once the SSE stream has started, errors are propagated
    /// as [`StreamEvent::Error`] items rather than retried.
    pub async fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        let mut req = request.clone();
        req.stream = true;

        let body = serde_json::to_value(&req)?;

        // Attempt connection with retry logic.
        let response = self.send_with_retry(&body).await?;

        let byte_stream = response.bytes_stream();
        let event_stream = sse_stream(byte_stream);

        Ok(Box::pin(event_stream))
    }

    /// Internal helper: send a POST and retry on 429/529 until we get a
    /// successful (streaming-capable) response.
    async fn send_with_retry(&self, body: &serde_json::Value) -> Result<reqwest::Response> {
        let mut attempt: u32 = 0;
        loop {
            let response = self
                .http
                .post(&self.api_url)
                .headers(self.default_headers()?)
                .json(body)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                return Ok(response);
            }

            let retry_after = Self::retry_after_duration(response.headers());
            let error_body = response.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if Self::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = retry_after.unwrap_or_else(|| self.backoff_duration(attempt));
                warn!(
                    status = status.as_u16(),
                    attempt,
                    wait_secs = wait.as_secs_f64(),
                    "Retryable API error on stream connect, backing off"
                );
                sleep(wait).await;
                attempt += 1;
                continue;
            }

            return Err(Self::status_to_error(status, &error_text));
        }
    }

    // -- error mapping ------------------------------------------------------

    fn status_to_error(status: StatusCode, body: &str) -> AgentError {
        // Try to parse structured error from body.
        let detail = serde_json::from_str::<ApiErrorResponse>(body)
            .map(|e| e.error.message)
            .unwrap_or_else(|_| body.to_string());

        match status.as_u16() {
            401 => AgentError::AuthenticationFailed(detail),
            403 => AgentError::AuthenticationFailed(detail),
            400 => AgentError::InvalidRequest(detail),
            402 => AgentError::BillingError(detail),
            429 => AgentError::RateLimited(detail),
            529 => AgentError::ServerError(format!("overloaded: {detail}")),
            500..=599 => AgentError::ServerError(detail),
            _ => AgentError::Api(format!("HTTP {status}: {detail}")),
        }
    }
}

// ---------------------------------------------------------------------------
// SSE parser
// ---------------------------------------------------------------------------

/// Parse a `bytes_stream()` from reqwest into a stream of [`StreamEvent`]s.
///
/// The Anthropic SSE protocol sends lines of the form:
/// ```text
/// event: <event_type>
/// data: <json_payload>
/// ```
fn sse_stream(
    byte_stream: impl Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent>> + Send + 'static {
    async_stream::stream! {
        let line_stream = chunked_lines(byte_stream);
        tokio::pin!(line_stream);

        let mut current_event_type = String::new();

        while let Some(line_result) = line_stream.next().await {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };

            if line.starts_with("event:") {
                current_event_type = line["event:".len()..].trim().to_string();
            } else if line.starts_with("data:") {
                let data = line["data:".len()..].trim();
                yield parse_stream_event(&current_event_type, data);
            }
            // Skip empty lines and comments.
        }
    }
}

/// Break a byte-stream into newline-delimited strings.
fn chunked_lines(
    byte_stream: impl Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<String>> + Send + 'static {
    async_stream::stream! {
        let mut buf = String::new();
        tokio::pin!(byte_stream);

        while let Some(chunk) = byte_stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Err(AgentError::Http(e));
                    return;
                }
            };

            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim_end_matches('\r').to_string();
                buf = buf[pos + 1..].to_string();
                if !line.is_empty() {
                    yield Ok(line);
                }
            }
        }

        // Flush remainder.
        let remainder = buf.trim().to_string();
        if !remainder.is_empty() {
            yield Ok(remainder);
        }
    }
}

/// Map an SSE event type + JSON data into a [`StreamEvent`].
fn parse_stream_event(event_type: &str, data: &str) -> Result<StreamEvent> {
    match event_type {
        "message_start" => {
            let raw: RawMessageStart = serde_json::from_str(data)?;
            Ok(StreamEvent::MessageStart {
                message: raw.message,
            })
        }
        "content_block_start" => {
            let raw: RawContentBlockStart = serde_json::from_str(data)?;
            Ok(StreamEvent::ContentBlockStart {
                index: raw.index,
                content_block: raw.content_block,
            })
        }
        "content_block_delta" => {
            let raw: RawContentBlockDelta = serde_json::from_str(data)?;
            Ok(StreamEvent::ContentBlockDelta {
                index: raw.index,
                delta: raw.delta,
            })
        }
        "content_block_stop" => {
            let raw: RawContentBlockStop = serde_json::from_str(data)?;
            Ok(StreamEvent::ContentBlockStop { index: raw.index })
        }
        "message_delta" => {
            let raw: RawMessageDelta = serde_json::from_str(data)?;
            Ok(StreamEvent::MessageDelta {
                delta: raw.delta,
                usage: raw.usage,
            })
        }
        "message_stop" => Ok(StreamEvent::MessageStop),
        "ping" => Ok(StreamEvent::Ping),
        "error" => {
            let raw: RawError = serde_json::from_str(data)?;
            Ok(StreamEvent::Error { error: raw.error })
        }
        other => {
            debug!(event_type = other, "Unknown SSE event type, ignoring");
            Ok(StreamEvent::Ping) // Treat unknown events as no-ops.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize tests that mutate environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn provider_default_is_anthropic() {
        // Clear env vars that could interfere.
        env::remove_var("CLAUDE_CODE_USE_BEDROCK");
        env::remove_var("CLAUDE_CODE_USE_VERTEX");
        assert_eq!(Provider::from_env(), Provider::Anthropic);
    }

    #[test]
    fn serialize_request_omits_none_fields() {
        let req = CreateMessageRequest {
            model: "claude-haiku-4-5".into(),
            max_tokens: 1024,
            messages: vec![ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "Hello".into(),
                    cache_control: None,
                }],
            }],
            system: None,
            tools: None,
            stream: false,
            metadata: None,
            thinking: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(!json.as_object().unwrap().contains_key("system"));
        assert!(!json.as_object().unwrap().contains_key("tools"));
        assert!(!json.as_object().unwrap().contains_key("metadata"));
        assert!(!json.as_object().unwrap().contains_key("thinking"));
    }

    #[test]
    fn tool_use_content_block_roundtrips() {
        let block = ApiContentBlock::ToolUse {
            id: "tu_123".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        };
        let json = serde_json::to_string(&block).unwrap();
        let back: ApiContentBlock = serde_json::from_str(&json).unwrap();
        match back {
            ApiContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "tu_123");
                assert_eq!(name, "bash");
                assert_eq!(input, serde_json::json!({"command": "ls"}));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn tool_result_content_block_roundtrips() {
        let block = ApiContentBlock::ToolResult {
            tool_use_id: "tu_123".into(),
            content: serde_json::json!("output text"),
            is_error: Some(false),
            cache_control: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        let back: ApiContentBlock = serde_json::from_str(&json).unwrap();
        match back {
            ApiContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "tu_123");
                assert_eq!(content, serde_json::json!("output text"));
                assert_eq!(is_error, Some(false));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parse_message_start_event() {
        let data = r#"{"message":{"id":"msg_1","role":"assistant","content":[],"model":"claude-haiku-4-5","stop_reason":null,"usage":{"input_tokens":10,"output_tokens":0}}}"#;
        let event = parse_stream_event("message_start", data).unwrap();
        match event {
            StreamEvent::MessageStart { message } => {
                assert_eq!(message.id, "msg_1");
                assert_eq!(message.usage.input_tokens, 10);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parse_content_block_delta_event() {
        let data = r#"{"index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let event = parse_stream_event("content_block_delta", data).unwrap();
        match event {
            StreamEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 0);
                match delta {
                    ContentDelta::TextDelta { text } => assert_eq!(text, "Hello"),
                    _ => panic!("wrong delta variant"),
                }
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn status_to_error_maps_correctly() {
        let err = ApiClient::status_to_error(StatusCode::UNAUTHORIZED, r#"{"error":{"type":"auth","message":"bad key"}}"#);
        assert!(matches!(err, AgentError::AuthenticationFailed(_)));

        let err = ApiClient::status_to_error(StatusCode::TOO_MANY_REQUESTS, "rate limited");
        assert!(matches!(err, AgentError::RateLimited(_)));

        let err = ApiClient::status_to_error(StatusCode::BAD_REQUEST, "invalid");
        assert!(matches!(err, AgentError::InvalidRequest(_)));
    }

    #[test]
    fn backoff_duration_increases() {
        let client = ApiClient::with_api_key("test-key");
        let d0 = client.backoff_duration(0);
        let d1 = client.backoff_duration(1);
        let d2 = client.backoff_duration(2);
        assert!(d1 > d0);
        assert!(d2 > d1);
        // Should be capped at max_backoff.
        let d100 = client.backoff_duration(100);
        assert!(d100 <= client.retry_config.max_backoff);
    }

    #[test]
    fn with_api_key_stores_key() {
        let client = ApiClient::with_api_key("sk-ant-test-123");
        assert_eq!(client.api_key, "sk-ant-test-123");
    }

    #[test]
    fn new_fails_when_env_var_missing() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prev = env::var("ANTHROPIC_API_KEY").ok();
        env::remove_var("ANTHROPIC_API_KEY");
        let result = ApiClient::new();
        if let Some(v) = prev { env::set_var("ANTHROPIC_API_KEY", v); }
        match result {
            Err(AgentError::AuthenticationFailed(msg)) => {
                assert!(msg.contains("not set"), "expected 'not set' in: {msg}");
            }
            Err(other) => panic!("expected AuthenticationFailed, got: {other:?}"),
            Ok(_) => panic!("expected error when ANTHROPIC_API_KEY is unset"),
        }
    }

    #[test]
    fn new_fails_when_env_var_empty() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prev = env::var("ANTHROPIC_API_KEY").ok();
        env::set_var("ANTHROPIC_API_KEY", "");
        let result = ApiClient::new();
        match prev {
            Some(v) => env::set_var("ANTHROPIC_API_KEY", v),
            None => env::remove_var("ANTHROPIC_API_KEY"),
        }
        match result {
            Err(AgentError::AuthenticationFailed(msg)) => {
                assert!(msg.contains("empty"), "expected 'empty' in: {msg}");
            }
            Err(other) => panic!("expected AuthenticationFailed, got: {other:?}"),
            Ok(_) => panic!("expected error when ANTHROPIC_API_KEY is empty"),
        }
    }

    #[test]
    fn new_succeeds_when_env_var_set() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prev = env::var("ANTHROPIC_API_KEY").ok();
        env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-env");
        let result = ApiClient::new();
        match prev {
            Some(v) => env::set_var("ANTHROPIC_API_KEY", v),
            None => env::remove_var("ANTHROPIC_API_KEY"),
        }
        assert!(result.is_ok());
    }
}
