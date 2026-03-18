//! Anthropic Messages API provider.

use std::env;
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::time::sleep;
use tokio_stream::StreamExt;
use tracing::{debug, warn};

use crate::client::{
    ApiContentBlock, ApiError, ApiErrorResponse, ApiUsage, ContentDelta, CreateMessageRequest,
    MessageDelta, MessageResponse, RetryConfig, StreamEvent,
};
use crate::error::{AgentError, Result};
use crate::provider::{CostRates, LlmProvider, ProviderCapabilities};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Anthropic Messages API provider.
pub struct AnthropicProvider {
    http: reqwest::Client,
    api_key: String,
    api_url: String,
    retry_config: RetryConfig,
}

impl AnthropicProvider {
    /// Create from `ANTHROPIC_API_KEY` environment variable.
    pub fn from_env() -> Result<Self> {
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
        Ok(Self::build(key, ANTHROPIC_API_URL.to_string()))
    }

    /// Create with an explicit API key.
    pub fn with_api_key(key: impl Into<String>) -> Self {
        Self::build(key.into(), ANTHROPIC_API_URL.to_string())
    }

    /// Create with an explicit API key and base URL.
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self::build(api_key.into(), base_url.into())
    }

    fn build(api_key: String, api_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            api_url,
            retry_config: RetryConfig::default(),
        }
    }

    /// Override the default retry configuration.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

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

    fn backoff_duration(&self, attempt: u32) -> Duration {
        let secs = self.retry_config.initial_backoff.as_secs_f64()
            * self
                .retry_config
                .backoff_multiplier
                .powi(attempt as i32);
        let max_secs = self.retry_config.max_backoff.as_secs_f64();
        Duration::from_secs_f64(secs.min(max_secs))
    }

    fn is_retryable(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS || status.as_u16() == 529
    }

    fn status_to_error(status: StatusCode, body: &str) -> AgentError {
        let detail = serde_json::from_str::<ApiErrorResponse>(body)
            .map(|e| e.error.message)
            .unwrap_or_else(|_| body.to_string());

        match status.as_u16() {
            401 | 403 => AgentError::AuthenticationFailed(detail),
            400 => AgentError::InvalidRequest(detail),
            402 => AgentError::BillingError(detail),
            429 => AgentError::RateLimited(detail),
            529 => AgentError::ServerError(format!("overloaded: {detail}")),
            500..=599 => AgentError::ServerError(detail),
            _ => AgentError::Api(format!("HTTP {status}: {detail}")),
        }
    }

    /// Internal helper: send a POST with retry on 429/529.
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

            let error_body = response.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if Self::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = self.backoff_duration(attempt);
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
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_use: true,
            thinking: true,
            prompt_caching: true,
        }
    }

    fn cost_rates(&self, model: &str) -> CostRates {
        match model {
            m if m.contains("opus") => CostRates {
                input_per_million: 15.0,
                output_per_million: 75.0,
            },
            m if m.contains("sonnet") => CostRates {
                input_per_million: 3.0,
                output_per_million: 15.0,
            },
            m if m.contains("haiku") => CostRates {
                input_per_million: 0.25,
                output_per_million: 1.25,
            },
            _ => CostRates {
                input_per_million: 3.0,
                output_per_million: 15.0,
            },
        }
    }

    async fn create_message(&self, request: &CreateMessageRequest) -> Result<MessageResponse> {
        let mut req = request.clone();
        req.stream = false;
        strip_tool_result_names(&mut req);

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

            let error_body = response.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if Self::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = self.backoff_duration(attempt);
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

    async fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        let mut req = request.clone();
        req.stream = true;
        strip_tool_result_names(&mut req);

        let body = serde_json::to_value(&req)?;
        let response = self.send_with_retry(&body).await?;

        let byte_stream = response.bytes_stream();
        let event_stream = sse_stream(byte_stream);

        Ok(Box::pin(event_stream))
    }
}

// ---------------------------------------------------------------------------
// SSE parser (Anthropic format)
// ---------------------------------------------------------------------------

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
        }
    }
}

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

        let remainder = buf.trim().to_string();
        if !remainder.is_empty() {
            yield Ok(remainder);
        }
    }
}

// Raw JSON shapes for Anthropic SSE deserialization.

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
            Ok(StreamEvent::Ping)
        }
    }
}

/// Strip the `name` field from `ToolResult` blocks.
///
/// The Anthropic API does not accept `name` on `tool_result` content blocks
/// (it's only needed by Gemini's `functionResponse`). Sending it causes
/// "Extra inputs are not permitted" validation errors.
fn strip_tool_result_names(req: &mut CreateMessageRequest) {
    for msg in &mut req.messages {
        for block in &mut msg.content {
            if let ApiContentBlock::ToolResult { name, .. } = block {
                *name = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let err = AnthropicProvider::status_to_error(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"type":"auth","message":"bad key"}}"#,
        );
        assert!(matches!(err, AgentError::AuthenticationFailed(_)));

        let err =
            AnthropicProvider::status_to_error(StatusCode::TOO_MANY_REQUESTS, "rate limited");
        assert!(matches!(err, AgentError::RateLimited(_)));

        let err = AnthropicProvider::status_to_error(StatusCode::BAD_REQUEST, "invalid");
        assert!(matches!(err, AgentError::InvalidRequest(_)));
    }

    #[test]
    fn backoff_duration_increases() {
        let provider = AnthropicProvider::with_api_key("test-key");
        let d0 = provider.backoff_duration(0);
        let d1 = provider.backoff_duration(1);
        let d2 = provider.backoff_duration(2);
        assert!(d1 > d0);
        assert!(d2 > d1);
        let d100 = provider.backoff_duration(100);
        assert!(d100 <= provider.retry_config.max_backoff);
    }

    #[test]
    fn cost_rates_by_model() {
        let provider = AnthropicProvider::with_api_key("test-key");
        let opus = provider.cost_rates("claude-opus-4-6");
        assert!(opus.input_per_million > 10.0);

        let haiku = provider.cost_rates("claude-haiku-4-5");
        assert!(haiku.input_per_million < 1.0);
    }
}
