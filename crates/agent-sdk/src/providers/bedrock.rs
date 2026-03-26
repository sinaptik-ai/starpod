//! AWS Bedrock provider for Claude models.
//!
//! Uses Bedrock's InvokeModel API which accepts the same Messages API format
//! as direct Anthropic, with AWS SigV4 authentication and different URL structure.

use std::env;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    sign, PayloadChecksumKind, SignableBody, SignableRequest, SigningParams, SigningSettings,
};
use aws_sigv4::sign::v4;
use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tokio::time::sleep;
use tokio_stream::StreamExt;
use tracing::{debug, warn};

use crate::client::{CreateMessageRequest, MessageResponse, RetryConfig, StreamEvent};
use crate::error::{AgentError, Result};
use crate::models::ModelRegistry;
use crate::provider::{CostRates, LlmProvider, ProviderCapabilities};

use super::anthropic::{strip_tool_result_names, AnthropicProvider};

const BEDROCK_API_VERSION: &str = "bedrock-2023-05-31";

/// AWS Bedrock provider for Claude models.
///
/// Routes Claude API calls through AWS Bedrock using SigV4 authentication.
/// The request/response format is identical to the Anthropic Messages API —
/// only authentication and URL structure differ.
pub struct BedrockProvider {
    http: reqwest::Client,
    region: String,
    credentials: Credentials,
    retry_config: RetryConfig,
    pricing: Option<Arc<ModelRegistry>>,
}

impl BedrockProvider {
    /// Create from standard AWS environment variables.
    ///
    /// Reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and optionally
    /// `AWS_SESSION_TOKEN` and `AWS_REGION` (defaults to `us-east-1`).
    pub fn from_env() -> Result<Self> {
        let access_key = env::var("AWS_ACCESS_KEY_ID").map_err(|_| {
            AgentError::AuthenticationFailed(
                "AWS_ACCESS_KEY_ID environment variable is not set".into(),
            )
        })?;
        let secret_key = env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
            AgentError::AuthenticationFailed(
                "AWS_SECRET_ACCESS_KEY environment variable is not set".into(),
            )
        })?;
        let session_token = env::var("AWS_SESSION_TOKEN").ok();
        let region = env::var("AWS_REGION")
            .or_else(|_| env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_string());

        Ok(Self::new(access_key, secret_key, session_token, region))
    }

    /// Create with explicit credentials and region.
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
        region: impl Into<String>,
    ) -> Self {
        let credentials = Credentials::new(
            access_key_id,
            secret_access_key,
            session_token,
            None, // expiry
            "starpod-bedrock",
        );

        Self {
            http: reqwest::Client::new(),
            region: region.into(),
            credentials,
            retry_config: RetryConfig::default(),
            pricing: None,
        }
    }

    /// Create with a specific region (reads credentials from env).
    pub fn with_region(region: impl Into<String>) -> Result<Self> {
        let mut provider = Self::from_env()?;
        provider.region = region.into();
        Ok(provider)
    }

    /// Attach a pricing registry for cost lookups.
    pub fn with_pricing(mut self, registry: Arc<ModelRegistry>) -> Self {
        self.pricing = Some(registry);
        self
    }

    /// Override the default retry configuration.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Build the InvokeModel URL for a given model ID.
    fn invoke_url(&self, model_id: &str) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke",
            self.region, model_id
        )
    }

    /// Build the InvokeModelWithResponseStream URL for a given model ID.
    fn invoke_stream_url(&self, model_id: &str) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke-with-response-stream",
            self.region, model_id
        )
    }

    /// Sign a request with AWS SigV4.
    fn sign_request(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<Vec<(String, String)>> {
        let mut settings = SigningSettings::default();
        settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;

        let identity = self.credentials.clone().into();
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name("bedrock")
            .time(SystemTime::now())
            .settings(settings)
            .build()
            .map_err(|e| AgentError::AuthenticationFailed(format!("SigV4 params error: {e}")))?;

        let params: SigningParams = params.into();

        let signable = SignableRequest::new(
            method,
            url,
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            SignableBody::Bytes(body),
        )
        .map_err(|e| AgentError::AuthenticationFailed(format!("SigV4 signable error: {e}")))?;

        let (instructions, _signature) = sign(signable, &params)
            .map_err(|e| AgentError::AuthenticationFailed(format!("SigV4 signing error: {e}")))?
            .into_parts();

        let mut signed_headers = Vec::new();
        for (name, value) in instructions.headers() {
            signed_headers.push((
                name.to_string(),
                String::from_utf8_lossy(value.as_bytes()).to_string(),
            ));
        }

        Ok(signed_headers)
    }

    /// Build the Bedrock request body.
    ///
    /// Same as Anthropic Messages API but with `anthropic_version` in the body
    /// instead of as a header, and without the `model` field (model is in the URL).
    fn build_body(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<serde_json::Value> {
        let mut req = request.clone();
        req.stream = false; // Will be removed; streaming is controlled by endpoint URL
        strip_tool_result_names(&mut req);

        let mut body = serde_json::to_value(&req)?;

        // Remove fields that Bedrock doesn't accept in the body:
        // - `model`: specified in the URL path
        // - `stream`: controlled by endpoint (invoke vs invoke-with-response-stream)
        if let Some(obj) = body.as_object_mut() {
            obj.remove("model");
            obj.remove("stream");
            // Add Bedrock-specific anthropic_version
            obj.insert(
                "anthropic_version".to_string(),
                serde_json::Value::String(BEDROCK_API_VERSION.to_string()),
            );
        }

        Ok(body)
    }

    /// Send a signed POST request to Bedrock with retry logic.
    async fn send_signed(
        &self,
        url: &str,
        body: &[u8],
    ) -> Result<reqwest::Response> {
        let mut attempt: u32 = 0;

        loop {
            // Compute payload hash
            let payload_hash = hex::encode(Sha256::digest(body));

            // Build base headers
            let host = url
                .strip_prefix("https://")
                .and_then(|s| s.split('/').next())
                .unwrap_or_default();

            let base_headers = vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("host".to_string(), host.to_string()),
                (
                    "x-amz-content-sha256".to_string(),
                    payload_hash.to_string(),
                ),
            ];

            // Sign the request
            let signed_headers = self.sign_request("POST", url, &base_headers, body)?;

            // Build reqwest headers
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

            for (name, value) in &base_headers {
                if name != "content-type" {
                    if let (Ok(name), Ok(val)) = (
                        reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                        HeaderValue::from_str(value),
                    ) {
                        headers.insert(name, val);
                    }
                }
            }
            for (name, value) in &signed_headers {
                if let (Ok(name), Ok(val)) = (
                    reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    headers.insert(name, val);
                }
            }

            let response = self
                .http
                .post(url)
                .headers(headers)
                .body(body.to_vec())
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                return Ok(response);
            }

            let error_body = response.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if AnthropicProvider::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = self.backoff_duration(attempt);
                warn!(
                    status = status.as_u16(),
                    attempt,
                    wait_secs = wait.as_secs_f64(),
                    "Retryable Bedrock API error, backing off"
                );
                sleep(wait).await;
                attempt += 1;
                continue;
            }

            return Err(Self::status_to_error(status, &error_text));
        }
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

    fn status_to_error(status: StatusCode, body: &str) -> AgentError {
        // Bedrock returns similar error shapes; reuse Anthropic's mapper
        AnthropicProvider::status_to_error(status, body)
    }
}

#[async_trait]
impl LlmProvider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_use: true,
            thinking: true,
            // Bedrock supports prompt caching for Anthropic models
            prompt_caching: true,
        }
    }

    fn cost_rates(&self, model: &str) -> CostRates {
        if let Some(ref registry) = self.pricing {
            if let Some(rates) = registry.get_pricing("bedrock", model) {
                return rates;
            }
            // Fall back to anthropic pricing since Bedrock uses same models
            if let Some(rates) = registry.get_pricing("anthropic", model) {
                return rates;
            }
        }
        // Hardcoded fallback — same as Anthropic (Bedrock pricing is identical)
        let cache = (Some(0.1), Some(1.25));
        match model {
            m if m.contains("opus") => CostRates {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            m if m.contains("sonnet") => CostRates {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            m if m.contains("haiku") => CostRates {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            _ => CostRates {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
        }
    }

    async fn create_message(&self, request: &CreateMessageRequest) -> Result<MessageResponse> {
        let body = self.build_body(request)?;
        let body_bytes = serde_json::to_vec(&body)?;
        let url = self.invoke_url(&request.model);

        let response = self.send_signed(&url, &body_bytes).await?;
        let msg: MessageResponse = response.json().await?;
        Ok(msg)
    }

    async fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        let body = self.build_body(request)?;
        let body_bytes = serde_json::to_vec(&body)?;
        let url = self.invoke_stream_url(&request.model);

        let response = self.send_signed(&url, &body_bytes).await?;

        // Bedrock streaming uses AWS Event Stream binary framing, not SSE.
        // Each frame contains a JSON payload with the same event structure as
        // Anthropic's SSE, wrapped in binary length-prefixed messages.
        let byte_stream = response.bytes_stream();
        let event_stream = aws_event_stream(byte_stream);

        Ok(Box::pin(event_stream))
    }
}

// ---------------------------------------------------------------------------
// AWS Event Stream decoder for Bedrock streaming responses
// ---------------------------------------------------------------------------
//
// Bedrock's `invoke-with-response-stream` returns binary-framed messages per
// the AWS Event Stream spec. Each message has:
//
//   [4 bytes: total_len] [4 bytes: headers_len] [4 bytes: prelude_crc]
//   [headers...] [payload...] [4 bytes: message_crc]
//
// The payload is a JSON object containing an Anthropic SSE-style event, e.g.:
//   {"bytes":"<base64-encoded-json>"}
//
// The base64-decoded JSON is the same format as Anthropic's SSE data lines.

/// Parse AWS Event Stream binary frames into `StreamEvent`s.
fn aws_event_stream(
    byte_stream: impl Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent>> + Send + 'static {
    use base64::Engine;

    async_stream::stream! {
        let mut buf = bytes::BytesMut::new();
        tokio::pin!(byte_stream);

        while let Some(chunk) = byte_stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Err(AgentError::Http(e));
                    return;
                }
            };

            buf.extend_from_slice(&chunk);

            // Process complete frames from the buffer
            loop {
                // Need at least 12 bytes for the prelude (total_len + headers_len + prelude_crc)
                if buf.len() < 12 {
                    break;
                }

                let total_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

                // Wait for the complete message
                if buf.len() < total_len {
                    break;
                }

                let headers_len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
                // prelude_crc at bytes 8..12 (skip validation for simplicity)

                let headers_start = 12;
                let headers_end = headers_start + headers_len;
                let payload_end = total_len - 4; // last 4 bytes are message CRC

                // Parse headers to find :event-type and :content-type
                let mut event_type = String::new();
                let headers_data = &buf[headers_start..headers_end];
                parse_event_stream_headers(headers_data, &mut event_type);

                // Extract payload
                let payload = &buf[headers_end..payload_end];

                if !payload.is_empty() {
                    // Bedrock wraps the Anthropic JSON in {"bytes":"<base64>"} for "chunk" events
                    if let Ok(wrapper) = serde_json::from_slice::<serde_json::Value>(payload) {
                        if let Some(b64) = wrapper.get("bytes").and_then(|v| v.as_str()) {
                            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(b64) {
                                if let Ok(json_str) = std::str::from_utf8(&decoded) {
                                    // The decoded JSON contains a "type" field matching Anthropic SSE event types
                                    if let Ok(event_obj) = serde_json::from_str::<serde_json::Value>(json_str) {
                                        if let Some(evt_type) = event_obj.get("type").and_then(|v| v.as_str()) {
                                            match parse_bedrock_event(evt_type, json_str) {
                                                Ok(event) => yield Ok(event),
                                                Err(e) => {
                                                    debug!(error = %e, "Failed to parse Bedrock event");
                                                    yield Err(e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if event_type == "exception" || event_type.contains("Exception") {
                        // Error event
                        let error_text = String::from_utf8_lossy(payload).to_string();
                        yield Err(AgentError::ServerError(format!("Bedrock stream error: {error_text}")));
                    }
                }

                // Consume the frame from the buffer
                let _ = buf.split_to(total_len);
            }
        }
    }
}

/// Parse AWS Event Stream headers to extract the :event-type value.
///
/// Header format: [1 byte: name_len] [name bytes] [1 byte: value_type]
/// For type 7 (string): [2 bytes: value_len] [value bytes]
fn parse_event_stream_headers(mut data: &[u8], event_type: &mut String) {
    while data.len() > 2 {
        let name_len = data[0] as usize;
        data = &data[1..];
        if data.len() < name_len + 1 {
            break;
        }
        let name = std::str::from_utf8(&data[..name_len]).unwrap_or_default();
        data = &data[name_len..];

        let value_type = data[0];
        data = &data[1..];

        match value_type {
            7 => {
                // String type: 2-byte big-endian length + value
                if data.len() < 2 {
                    break;
                }
                let value_len = u16::from_be_bytes([data[0], data[1]]) as usize;
                data = &data[2..];
                if data.len() < value_len {
                    break;
                }
                let value = std::str::from_utf8(&data[..value_len]).unwrap_or_default();
                if name == ":event-type" {
                    *event_type = value.to_string();
                }
                data = &data[value_len..];
            }
            // Skip other types — we only need :event-type for routing
            _ => break,
        }
    }
}

/// Parse a decoded Bedrock event JSON into a `StreamEvent`.
///
/// The JSON has the same structure as Anthropic SSE events, with an additional
/// `"type"` field that mirrors the SSE `event:` line.
fn parse_bedrock_event(event_type: &str, json: &str) -> Result<StreamEvent> {
    use crate::client::{
        ApiContentBlock, ApiUsage, ContentDelta, MessageDelta, MessageResponse,
    };

    match event_type {
        "message_start" => {
            #[derive(serde::Deserialize)]
            struct Wrapper { message: MessageResponse }
            let w: Wrapper = serde_json::from_str(json)?;
            Ok(StreamEvent::MessageStart { message: w.message })
        }
        "content_block_start" => {
            #[derive(serde::Deserialize)]
            struct Wrapper { index: usize, content_block: ApiContentBlock }
            let w: Wrapper = serde_json::from_str(json)?;
            Ok(StreamEvent::ContentBlockStart { index: w.index, content_block: w.content_block })
        }
        "content_block_delta" => {
            #[derive(serde::Deserialize)]
            struct Wrapper { index: usize, delta: ContentDelta }
            let w: Wrapper = serde_json::from_str(json)?;
            Ok(StreamEvent::ContentBlockDelta { index: w.index, delta: w.delta })
        }
        "content_block_stop" => {
            #[derive(serde::Deserialize)]
            struct Wrapper { index: usize }
            let w: Wrapper = serde_json::from_str(json)?;
            Ok(StreamEvent::ContentBlockStop { index: w.index })
        }
        "message_delta" => {
            #[derive(serde::Deserialize)]
            struct Wrapper { delta: MessageDelta, usage: ApiUsage }
            let w: Wrapper = serde_json::from_str(json)?;
            Ok(StreamEvent::MessageDelta { delta: w.delta, usage: w.usage })
        }
        "message_stop" => Ok(StreamEvent::MessageStop),
        "ping" => Ok(StreamEvent::Ping),
        "error" => {
            #[derive(serde::Deserialize)]
            struct Wrapper { error: crate::client::ApiError }
            let w: Wrapper = serde_json::from_str(json)?;
            Ok(StreamEvent::Error { error: w.error })
        }
        other => {
            debug!(event_type = other, "Unknown Bedrock event type, treating as ping");
            Ok(StreamEvent::Ping)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_url_format() {
        let provider = BedrockProvider::new(
            "AKIATEST",
            "secret",
            None,
            "us-east-1",
        );
        assert_eq!(
            provider.invoke_url("us.anthropic.claude-sonnet-4-6-20250514-v1:0"),
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/us.anthropic.claude-sonnet-4-6-20250514-v1:0/invoke"
        );
    }

    #[test]
    fn invoke_stream_url_format() {
        let provider = BedrockProvider::new(
            "AKIATEST",
            "secret",
            None,
            "eu-west-1",
        );
        assert_eq!(
            provider.invoke_stream_url("anthropic.claude-3-5-haiku-20241022-v1:0"),
            "https://bedrock-runtime.eu-west-1.amazonaws.com/model/anthropic.claude-3-5-haiku-20241022-v1:0/invoke-with-response-stream"
        );
    }

    #[test]
    fn build_body_removes_model_and_adds_version() {
        let provider = BedrockProvider::new(
            "AKIATEST",
            "secret",
            None,
            "us-east-1",
        );

        let request = CreateMessageRequest {
            model: "us.anthropic.claude-sonnet-4-6-20250514-v1:0".to_string(),
            max_tokens: 1024,
            messages: vec![],
            system: None,
            tools: None,
            stream: false,
            metadata: None,
            thinking: None,
        };

        let body = provider.build_body(&request).unwrap();
        let obj = body.as_object().unwrap();

        // model should be removed (it's in the URL)
        assert!(!obj.contains_key("model"));
        // stream should be removed (controlled by endpoint URL)
        assert!(!obj.contains_key("stream"));
        // anthropic_version should be added
        assert_eq!(obj["anthropic_version"], BEDROCK_API_VERSION);
    }

    #[test]
    fn cost_rates_match_anthropic() {
        let provider = BedrockProvider::new(
            "AKIATEST",
            "secret",
            None,
            "us-east-1",
        );

        let sonnet = provider.cost_rates("us.anthropic.claude-sonnet-4-6-20250514-v1:0");
        assert!((sonnet.input_per_million - 3.0).abs() < 1e-9);
        assert!((sonnet.output_per_million - 15.0).abs() < 1e-9);

        let haiku = provider.cost_rates("anthropic.claude-3-5-haiku-20241022-v1:0");
        assert!((haiku.input_per_million - 1.0).abs() < 1e-9);
    }

    #[test]
    fn backoff_duration_increases() {
        let provider = BedrockProvider::new("AKIATEST", "secret", None, "us-east-1");
        let d0 = provider.backoff_duration(0);
        let d1 = provider.backoff_duration(1);
        let d2 = provider.backoff_duration(2);
        assert!(d1 > d0);
        assert!(d2 > d1);
        let d100 = provider.backoff_duration(100);
        assert!(d100 <= provider.retry_config.max_backoff);
    }
}
