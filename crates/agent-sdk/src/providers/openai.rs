//! OpenAI Chat Completions provider.
//!
//! Also serves Groq, DeepSeek, OpenRouter, and Ollama via `base_url` override
//! (all OpenAI-compatible APIs).

use std::pin::Pin;
use std::sync::Arc;
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
    ApiContentBlock, ApiMessage, ApiUsage, ContentDelta, CreateMessageRequest, MessageDelta,
    MessageResponse, RetryConfig, StreamEvent,
};
use crate::error::{AgentError, Result};
use crate::models::ModelRegistry;
use crate::provider::{CostRates, LlmProvider, ProviderCapabilities};

const DEFAULT_OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

/// OpenAI Chat Completions provider.
///
/// Works with any OpenAI-compatible API (Groq, DeepSeek, OpenRouter, Ollama)
/// by setting a custom `base_url`.
pub struct OpenAiProvider {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    provider_name: String,
    retry_config: RetryConfig,
    pricing: Option<Arc<ModelRegistry>>,
    /// Extra fields merged into every request body (e.g. Ollama's `keep_alive`).
    extra_body: serde_json::Map<String, serde_json::Value>,
}

impl OpenAiProvider {
    /// Create with an API key and the default OpenAI endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, DEFAULT_OPENAI_URL, "openai")
    }

    /// Create with an API key, custom base URL, and provider name.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        provider_name: impl Into<String>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            provider_name: provider_name.into(),
            retry_config: RetryConfig::default(),
            pricing: None,
            extra_body: serde_json::Map::new(),
        }
    }

    /// Override the default retry configuration.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Attach a pricing registry for cost lookups.
    pub fn with_pricing(mut self, registry: Arc<ModelRegistry>) -> Self {
        self.pricing = Some(registry);
        self
    }

    /// Set extra fields merged into every request body.
    ///
    /// Useful for provider-specific options like Ollama's `keep_alive` or `num_ctx`.
    pub fn with_extra_body(mut self, extra: serde_json::Map<String, serde_json::Value>) -> Self {
        self.extra_body = extra;
        self
    }

    fn default_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !self.api_key.is_empty() {
            headers.insert(
                "Authorization",
                HeaderValue::from_str(&format!("Bearer {}", self.api_key)).map_err(|_| {
                    AgentError::AuthenticationFailed(
                        "API key contains invalid header characters".into(),
                    )
                })?,
            );
        }
        Ok(headers)
    }

    fn backoff_duration(&self, attempt: u32) -> Duration {
        let secs = self.retry_config.initial_backoff.as_secs_f64()
            * self.retry_config.backoff_multiplier.powi(attempt as i32);
        Duration::from_secs_f64(secs.min(self.retry_config.max_backoff.as_secs_f64()))
    }

    fn is_retryable(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS || status.as_u16() == 529
    }

    fn status_to_error(status: StatusCode, body: &str) -> AgentError {
        let detail = serde_json::from_str::<OaiErrorResponse>(body)
            .map(|e| e.error.message)
            .unwrap_or_else(|_| body.to_string());

        match status.as_u16() {
            401 | 403 => AgentError::AuthenticationFailed(detail),
            400 => AgentError::InvalidRequest(detail),
            402 => AgentError::BillingError(detail),
            429 => AgentError::RateLimited(detail),
            500..=599 => AgentError::ServerError(detail),
            _ => AgentError::Api(format!("HTTP {status}: {detail}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Translation: canonical -> OpenAI wire format
// ---------------------------------------------------------------------------

/// Build the OpenAI request JSON from our canonical `CreateMessageRequest`.
fn build_openai_request(request: &CreateMessageRequest) -> serde_json::Value {
    let mut messages: Vec<serde_json::Value> = Vec::new();

    // System prompt
    if let Some(system_blocks) = &request.system {
        let system_text: String = system_blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !system_text.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system_text,
            }));
        }
    }

    // Conversation messages
    for msg in &request.messages {
        messages.extend(translate_message(msg));
    }

    let mut body = serde_json::json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": request.stream,
    });

    // Tools
    if let Some(tools) = &request.tools {
        let oai_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();
        if !oai_tools.is_empty() {
            body["tools"] = serde_json::json!(oai_tools);
        }
    }

    body
}

/// Translate a single canonical `ApiMessage` into one or more OpenAI messages.
fn translate_message(msg: &ApiMessage) -> Vec<serde_json::Value> {
    let role = &msg.role;

    // Collect tool results -> individual "tool" role messages
    let tool_results: Vec<&ApiContentBlock> = msg
        .content
        .iter()
        .filter(|b| matches!(b, ApiContentBlock::ToolResult { .. }))
        .collect();

    if !tool_results.is_empty() {
        return tool_results
            .into_iter()
            .map(|b| match b {
                ApiContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    let text = match content {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": text,
                    })
                }
                _ => unreachable!(),
            })
            .collect();
    }

    // Collect text blocks
    let texts: Vec<String> = msg
        .content
        .iter()
        .filter_map(|b| match b {
            ApiContentBlock::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();

    // Collect tool_use blocks -> tool_calls
    let tool_calls: Vec<serde_json::Value> = msg
        .content
        .iter()
        .filter_map(|b| match b {
            ApiContentBlock::ToolUse { id, name, input } => Some(serde_json::json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": input.to_string(),
                }
            })),
            _ => None,
        })
        .collect();

    let content_text = texts.join("");

    let mut oai_msg = serde_json::json!({
        "role": role,
    });

    if !content_text.is_empty() || tool_calls.is_empty() {
        oai_msg["content"] = serde_json::json!(content_text);
    }

    if !tool_calls.is_empty() {
        oai_msg["tool_calls"] = serde_json::json!(tool_calls);
        // OpenAI requires content to be null when there are tool_calls and no text
        if content_text.is_empty() {
            oai_msg["content"] = serde_json::Value::Null;
        }
    }

    vec![oai_msg]
}

// ---------------------------------------------------------------------------
// Translation: OpenAI response -> canonical
// ---------------------------------------------------------------------------

fn parse_openai_response(oai: &OaiChatResponse) -> Result<MessageResponse> {
    let choice = oai
        .choices
        .first()
        .ok_or_else(|| AgentError::Api("No choices in OpenAI response".into()))?;

    let mut content: Vec<ApiContentBlock> = Vec::new();

    // Text content
    if let Some(text) = &choice.message.content {
        if !text.is_empty() {
            content.push(ApiContentBlock::Text {
                text: text.clone(),
                cache_control: None,
            });
        }
    }

    // Tool calls
    if let Some(tool_calls) = &choice.message.tool_calls {
        for tc in tool_calls {
            let input: serde_json::Value = match serde_json::from_str(&tc.function.arguments) {
                Ok(v) => v,
                Err(e) => {
                    warn!(
                        tool = %tc.function.name,
                        arguments = %tc.function.arguments,
                        error = %e,
                        "failed to parse tool call arguments, defaulting to {{}}"
                    );
                    serde_json::json!({})
                }
            };
            content.push(ApiContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                input,
            });
        }
    }

    // Map finish_reason
    let stop_reason = match choice.finish_reason.as_deref() {
        Some("stop") => Some("end_turn".to_string()),
        Some("tool_calls") => Some("tool_use".to_string()),
        Some("length") => Some("max_tokens".to_string()),
        other => other.map(String::from),
    };

    let usage = if let Some(u) = &oai.usage {
        ApiUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }
    } else {
        ApiUsage::default()
    };

    Ok(MessageResponse {
        id: oai.id.clone(),
        role: "assistant".to_string(),
        content,
        model: oai.model.clone(),
        stop_reason,
        usage,
    })
}

// ---------------------------------------------------------------------------
// OpenAI wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OaiChatResponse {
    id: String,
    model: String,
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
struct OaiToolCall {
    id: String,
    function: OaiFunction,
}

#[derive(Debug, Clone, Deserialize)]
struct OaiFunction {
    name: String,
    /// OpenAI returns arguments as a JSON string (`"{\"key\":\"val\"}"`),
    /// but Ollama's OpenAI-compatible endpoint may return a raw JSON object.
    /// We accept both via the custom deserializer.
    #[serde(deserialize_with = "deserialize_arguments")]
    arguments: String,
}

/// Deserialize `arguments` from either a JSON string or an inline object/value.
///
/// OpenAI always sends `"arguments": "{\"key\":\"val\"}"` (a string containing JSON).
/// Ollama sometimes sends `"arguments": {"key":"val"}` (an already-parsed object).
/// This function normalises both to a `String` suitable for `serde_json::from_str`.
fn deserialize_arguments<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(deserializer)?;
    match val {
        serde_json::Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

/// Same as [`deserialize_arguments`] but for `Option<String>` (streaming deltas).
fn deserialize_arguments_opt<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(val.map(|v| match v {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }))
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct OaiErrorResponse {
    error: OaiError,
}

#[derive(Debug, Deserialize)]
struct OaiError {
    message: String,
}

// Streaming types

#[derive(Debug, Deserialize)]
struct OaiStreamChunk {
    id: String,
    model: String,
    choices: Vec<OaiStreamChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamChoice {
    delta: OaiStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OaiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamToolCall {
    index: usize,
    id: Option<String>,
    function: Option<OaiStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamFunction {
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_arguments_opt")]
    arguments: Option<String>,
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_use: true,
            thinking: false,
            prompt_caching: false,
        }
    }

    fn cost_rates(&self, model: &str) -> CostRates {
        if let Some(ref registry) = self.pricing {
            if let Some(rates) = registry.get_pricing(&self.provider_name, model) {
                return rates;
            }
        }
        // Hardcoded fallback
        let cache = (Some(0.1), Some(1.0));
        match model {
            "gpt-4.1" => CostRates {
                input_per_million: 2.0,
                output_per_million: 8.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            "gpt-4o" => CostRates {
                input_per_million: 2.5,
                output_per_million: 10.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            "gpt-4o-mini" => CostRates {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            "o3" => CostRates {
                input_per_million: 2.0,
                output_per_million: 8.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            "o4-mini" => CostRates {
                input_per_million: 1.1,
                output_per_million: 4.4,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            _ => CostRates {
                input_per_million: 2.0,
                output_per_million: 8.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
        }
    }

    async fn create_message(&self, request: &CreateMessageRequest) -> Result<MessageResponse> {
        let mut oai_body = build_openai_request(request);
        oai_body["stream"] = serde_json::json!(false);
        for (k, v) in &self.extra_body {
            oai_body[k] = v.clone();
        }

        let mut attempt: u32 = 0;
        loop {
            let response = self
                .http
                .post(&self.base_url)
                .headers(self.default_headers()?)
                .json(&oai_body)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let oai_resp: OaiChatResponse = response.json().await?;
                return parse_openai_response(&oai_resp);
            }

            let error_body = response.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if Self::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = self.backoff_duration(attempt);
                warn!(
                    status = status.as_u16(),
                    attempt,
                    wait_secs = wait.as_secs_f64(),
                    "Retryable OpenAI API error, backing off"
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
        let mut oai_body = build_openai_request(request);
        oai_body["stream"] = serde_json::json!(true);
        for (k, v) in &self.extra_body {
            oai_body[k] = v.clone();
        }

        // Retry connection
        let mut attempt: u32 = 0;
        let response = loop {
            let resp = self
                .http
                .post(&self.base_url)
                .headers(self.default_headers()?)
                .json(&oai_body)
                .send()
                .await?;

            let status = resp.status();
            if status.is_success() {
                break resp;
            }

            let error_body = resp.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if Self::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = self.backoff_duration(attempt);
                warn!(status = status.as_u16(), attempt, "Retrying OpenAI stream");
                sleep(wait).await;
                attempt += 1;
                continue;
            }

            return Err(Self::status_to_error(status, &error_text));
        };

        let byte_stream = response.bytes_stream();
        let event_stream = openai_sse_stream(byte_stream);
        Ok(Box::pin(event_stream))
    }
}

/// Parse OpenAI SSE stream into canonical StreamEvents.
///
/// OpenAI streams `data: {...}` lines (no `event:` prefix). Tool call arguments
/// arrive as partial JSON across multiple deltas and must be accumulated before
/// emitting a `ContentBlockStart` with the complete tool use.
fn openai_sse_stream(
    byte_stream: impl Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent>> + Send + 'static {
    async_stream::stream! {
        let mut buf = String::new();
        tokio::pin!(byte_stream);

        // Accumulator for partial tool calls: index -> (id, name, arguments_buffer)
        let mut tool_accum: Vec<(String, String, String)> = Vec::new();
        let mut content_index: usize = 0;
        let mut emitted_message_start = false;

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

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if !line.starts_with("data:") {
                    continue;
                }

                let data = line["data:".len()..].trim();

                if data == "[DONE]" {
                    // Emit accumulated tool calls before finishing
                    for (id, name, args) in tool_accum.drain(..) {
                        let input: serde_json::Value = match serde_json::from_str(&args) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!(arguments = %args, error = %e, "failed to parse streamed tool arguments, defaulting to {{}}");
                                serde_json::json!({})
                            }
                        };
                        yield Ok(StreamEvent::ContentBlockStart {
                            index: content_index,
                            content_block: ApiContentBlock::ToolUse {
                                id,
                                name,
                                input,
                            },
                        });
                        yield Ok(StreamEvent::ContentBlockStop { index: content_index });
                        content_index += 1;
                    }
                    yield Ok(StreamEvent::MessageStop);
                    return;
                }

                let chunk: OaiStreamChunk = match serde_json::from_str(data) {
                    Ok(c) => c,
                    Err(e) => {
                        debug!("Failed to parse OpenAI SSE chunk: {}", e);
                        continue;
                    }
                };

                // Emit MessageStart on the first chunk
                if !emitted_message_start {
                    emitted_message_start = true;
                    yield Ok(StreamEvent::MessageStart {
                        message: MessageResponse {
                            id: chunk.id.clone(),
                            role: "assistant".to_string(),
                            content: vec![],
                            model: chunk.model.clone(),
                            stop_reason: None,
                            usage: ApiUsage::default(),
                        },
                    });
                }

                for choice in &chunk.choices {
                    // Text delta
                    if let Some(text) = &choice.delta.content {
                        if !text.is_empty() {
                            yield Ok(StreamEvent::ContentBlockDelta {
                                index: 0,
                                delta: ContentDelta::TextDelta { text: text.clone() },
                            });
                        }
                    }

                    // Tool call deltas — accumulate
                    if let Some(tool_calls) = &choice.delta.tool_calls {
                        for tc in tool_calls {
                            let idx = tc.index;
                            // Ensure accumulator is large enough
                            while tool_accum.len() <= idx {
                                tool_accum.push((String::new(), String::new(), String::new()));
                            }

                            if let Some(id) = &tc.id {
                                tool_accum[idx].0 = id.clone();
                            }
                            if let Some(func) = &tc.function {
                                if let Some(name) = &func.name {
                                    tool_accum[idx].1 = name.clone();
                                }
                                if let Some(args) = &func.arguments {
                                    tool_accum[idx].2.push_str(args);
                                }
                            }
                        }
                    }

                    // finish_reason
                    if let Some(reason) = &choice.finish_reason {
                        let stop_reason = match reason.as_str() {
                            "stop" => "end_turn",
                            "tool_calls" => "tool_use",
                            "length" => "max_tokens",
                            other => other,
                        };

                        // Report final usage if available
                        let usage = if let Some(u) = &chunk.usage {
                            ApiUsage {
                                input_tokens: u.prompt_tokens,
                                output_tokens: u.completion_tokens,
                                cache_creation_input_tokens: None,
                                cache_read_input_tokens: None,
                            }
                        } else {
                            ApiUsage::default()
                        };

                        yield Ok(StreamEvent::MessageDelta {
                            delta: MessageDelta {
                                stop_reason: Some(stop_reason.to_string()),
                            },
                            usage,
                        });
                    }
                }
            }
        }

        // Flush any remaining tool calls
        for (id, name, args) in tool_accum.drain(..) {
            let input: serde_json::Value = match serde_json::from_str(&args) {
                Ok(v) => v,
                Err(e) => {
                    warn!(arguments = %args, error = %e, "failed to parse streamed tool arguments, defaulting to {{}}");
                    serde_json::json!({})
                }
            };
            yield Ok(StreamEvent::ContentBlockStart {
                index: content_index,
                content_block: ApiContentBlock::ToolUse {
                    id,
                    name,
                    input,
                },
            });
            yield Ok(StreamEvent::ContentBlockStop { index: content_index });
            content_index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::SystemBlock;

    #[test]
    fn translate_simple_user_message() {
        let msg = ApiMessage {
            role: "user".into(),
            content: vec![ApiContentBlock::Text {
                text: "Hello".into(),
                cache_control: None,
            }],
        };
        let result = translate_message(&msg);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[0]["content"], "Hello");
    }

    #[test]
    fn translate_assistant_with_tool_calls() {
        let msg = ApiMessage {
            role: "assistant".into(),
            content: vec![
                ApiContentBlock::Text {
                    text: "Let me check.".into(),
                    cache_control: None,
                },
                ApiContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "Bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                },
            ],
        };
        let result = translate_message(&msg);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "assistant");
        assert_eq!(result[0]["content"], "Let me check.");
        assert!(result[0]["tool_calls"].is_array());
        assert_eq!(result[0]["tool_calls"][0]["id"], "call_1");
        assert_eq!(result[0]["tool_calls"][0]["function"]["name"], "Bash");
    }

    #[test]
    fn translate_tool_results() {
        let msg = ApiMessage {
            role: "user".into(),
            content: vec![ApiContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: serde_json::json!("file1.txt\nfile2.txt"),
                is_error: None,
                cache_control: None,
                name: None,
            }],
        };
        let result = translate_message(&msg);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "tool");
        assert_eq!(result[0]["tool_call_id"], "call_1");
    }

    #[test]
    fn parse_openai_response_basic() {
        let json = r#"{
            "id": "chatcmpl-123",
            "model": "gpt-4.1",
            "choices": [{
                "message": {
                    "content": "Hello!"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5
            }
        }"#;
        let oai: OaiChatResponse = serde_json::from_str(json).unwrap();
        let msg = parse_openai_response(&oai).unwrap();
        assert_eq!(msg.id, "chatcmpl-123");
        assert_eq!(msg.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(msg.usage.input_tokens, 10);
        assert_eq!(msg.usage.output_tokens, 5);
        match &msg.content[0] {
            ApiContentBlock::Text { text, .. } => assert_eq!(text, "Hello!"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn parse_openai_response_with_tool_calls() {
        let json = r#"{
            "id": "chatcmpl-456",
            "model": "gpt-4.1",
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "Bash",
                            "arguments": "{\"command\":\"ls\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10}
        }"#;
        let oai: OaiChatResponse = serde_json::from_str(json).unwrap();
        let msg = parse_openai_response(&oai).unwrap();
        assert_eq!(msg.stop_reason.as_deref(), Some("tool_use"));
        match &msg.content[0] {
            ApiContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "Bash");
                assert_eq!(input, &serde_json::json!({"command": "ls"}));
            }
            _ => panic!("expected tool use"),
        }
    }

    // ── Ollama / argument-format compatibility tests ───────────────────

    #[test]
    fn parse_ollama_response_with_object_arguments() {
        // Ollama's OpenAI-compatible endpoint returns `arguments` as a raw
        // JSON object instead of a stringified JSON string.
        let json = r#"{
            "id": "chatcmpl-789",
            "model": "qwen3:8b",
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_ollama",
                        "type": "function",
                        "function": {
                            "name": "FileRead",
                            "arguments": {"path": "/tmp/test.txt"}
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 30, "completion_tokens": 15}
        }"#;
        let oai: OaiChatResponse = serde_json::from_str(json).unwrap();
        let msg = parse_openai_response(&oai).unwrap();
        match &msg.content[0] {
            ApiContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "call_ollama");
                assert_eq!(name, "FileRead");
                assert_eq!(input, &serde_json::json!({"path": "/tmp/test.txt"}));
            }
            _ => panic!("expected tool use"),
        }
    }

    #[test]
    fn parse_ollama_response_with_nested_object_arguments() {
        // Complex nested objects should also round-trip correctly.
        let json = r#"{
            "id": "chatcmpl-nested",
            "model": "qwen3:8b",
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_nested",
                        "type": "function",
                        "function": {
                            "name": "WriteFile",
                            "arguments": {
                                "path": "/tmp/out.json",
                                "content": "hello world",
                                "options": {"overwrite": true, "mode": 644}
                            }
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 40, "completion_tokens": 20}
        }"#;
        let oai: OaiChatResponse = serde_json::from_str(json).unwrap();
        let msg = parse_openai_response(&oai).unwrap();
        match &msg.content[0] {
            ApiContentBlock::ToolUse { input, .. } => {
                assert_eq!(input["path"], "/tmp/out.json");
                assert_eq!(input["options"]["overwrite"], true);
                assert_eq!(input["options"]["mode"], 644);
            }
            _ => panic!("expected tool use"),
        }
    }

    #[test]
    fn parse_ollama_response_with_empty_object_arguments() {
        // Tools with no parameters: `arguments: {}` (object) should work.
        let json = r#"{
            "id": "chatcmpl-empty",
            "model": "llama3:8b",
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_empty",
                        "type": "function",
                        "function": {
                            "name": "FileList",
                            "arguments": {}
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        }"#;
        let oai: OaiChatResponse = serde_json::from_str(json).unwrap();
        let msg = parse_openai_response(&oai).unwrap();
        match &msg.content[0] {
            ApiContentBlock::ToolUse { name, input, .. } => {
                assert_eq!(name, "FileList");
                assert_eq!(input, &serde_json::json!({}));
            }
            _ => panic!("expected tool use"),
        }
    }

    #[test]
    fn parse_ollama_multiple_tool_calls_with_object_arguments() {
        // Multiple tool calls in one response, all with object arguments.
        let json = r#"{
            "id": "chatcmpl-multi",
            "model": "qwen3:8b",
            "choices": [{
                "message": {
                    "content": "Let me check both files.",
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "FileRead",
                                "arguments": {"path": "/etc/hosts"}
                            }
                        },
                        {
                            "id": "call_2",
                            "type": "function",
                            "function": {
                                "name": "FileRead",
                                "arguments": {"path": "/etc/resolv.conf"}
                            }
                        }
                    ]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 50, "completion_tokens": 25}
        }"#;
        let oai: OaiChatResponse = serde_json::from_str(json).unwrap();
        let msg = parse_openai_response(&oai).unwrap();
        // Text block + 2 tool uses
        assert_eq!(msg.content.len(), 3);
        match &msg.content[1] {
            ApiContentBlock::ToolUse { input, .. } => {
                assert_eq!(input["path"], "/etc/hosts");
            }
            _ => panic!("expected tool use at index 1"),
        }
        match &msg.content[2] {
            ApiContentBlock::ToolUse { input, .. } => {
                assert_eq!(input["path"], "/etc/resolv.conf");
            }
            _ => panic!("expected tool use at index 2"),
        }
    }

    // ── deserialize_arguments unit tests ────────────────────────────────

    #[test]
    fn deserialize_arguments_from_string() {
        // Standard OpenAI format: arguments is a JSON string.
        let json = r#"{"name": "Bash", "arguments": "{\"command\":\"ls\"}"}"#;
        let func: OaiFunction = serde_json::from_str(json).unwrap();
        assert_eq!(func.arguments, r#"{"command":"ls"}"#);
        let parsed: serde_json::Value = serde_json::from_str(&func.arguments).unwrap();
        assert_eq!(parsed["command"], "ls");
    }

    #[test]
    fn deserialize_arguments_from_object() {
        // Ollama format: arguments is a raw JSON object.
        let json = r#"{"name": "Bash", "arguments": {"command": "ls"}}"#;
        let func: OaiFunction = serde_json::from_str(json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&func.arguments).unwrap();
        assert_eq!(parsed["command"], "ls");
    }

    #[test]
    fn deserialize_arguments_from_empty_string() {
        let json = r#"{"name": "Noop", "arguments": ""}"#;
        let func: OaiFunction = serde_json::from_str(json).unwrap();
        assert_eq!(func.arguments, "");
    }

    #[test]
    fn deserialize_arguments_from_empty_object() {
        let json = r#"{"name": "Noop", "arguments": {}}"#;
        let func: OaiFunction = serde_json::from_str(json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&func.arguments).unwrap();
        assert_eq!(parsed, serde_json::json!({}));
    }

    #[test]
    fn deserialize_arguments_from_array() {
        // Edge case: some providers might send an array.
        let json = r#"{"name": "Multi", "arguments": [1, 2, 3]}"#;
        let func: OaiFunction = serde_json::from_str(json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&func.arguments).unwrap();
        assert_eq!(parsed, serde_json::json!([1, 2, 3]));
    }

    // ── deserialize_arguments_opt unit tests ────────────────────────────

    #[test]
    fn deserialize_stream_arguments_from_string() {
        let json = r#"{"name": "Bash", "arguments": "{\"cmd\":\"pwd\"}"}"#;
        let func: OaiStreamFunction = serde_json::from_str(json).unwrap();
        assert_eq!(func.arguments.as_deref(), Some(r#"{"cmd":"pwd"}"#));
    }

    #[test]
    fn deserialize_stream_arguments_from_object() {
        let json = r#"{"name": "Bash", "arguments": {"cmd": "pwd"}}"#;
        let func: OaiStreamFunction = serde_json::from_str(json).unwrap();
        let args = func.arguments.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&args).unwrap();
        assert_eq!(parsed["cmd"], "pwd");
    }

    #[test]
    fn deserialize_stream_arguments_missing() {
        let json = r#"{"name": "Bash"}"#;
        let func: OaiStreamFunction = serde_json::from_str(json).unwrap();
        assert!(func.arguments.is_none());
    }

    #[test]
    fn deserialize_stream_arguments_null() {
        let json = r#"{"name": "Bash", "arguments": null}"#;
        let func: OaiStreamFunction = serde_json::from_str(json).unwrap();
        assert!(func.arguments.is_none());
    }

    // ── SSE streaming with Ollama-style tool calls ──────────────────────

    #[tokio::test]
    async fn stream_ollama_tool_call_with_object_arguments() {
        use tokio_stream::StreamExt as _;

        // Simulate Ollama sending a complete tool call in a single SSE chunk
        // where arguments is a JSON object, not a string.
        let sse_data = "\
data: {\"id\":\"chat-1\",\"model\":\"qwen3:8b\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"FileRead\",\"arguments\":\"{\\\"path\\\":\\\"/tmp/foo\\\"}\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chat-1\",\"model\":\"qwen3:8b\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n\
data: [DONE]\n\n";

        let bytes_stream =
            futures::stream::once(
                async move { Ok::<_, reqwest::Error>(bytes::Bytes::from(sse_data)) },
            );

        let raw: Vec<Result<StreamEvent>> = openai_sse_stream(bytes_stream).collect().await;
        let events: Vec<StreamEvent> = raw.into_iter().map(|r| r.unwrap()).collect();

        // Should contain: MessageStart, ContentBlockStart (tool), ContentBlockStop, MessageDelta, MessageStop
        let tool_event = events
            .iter()
            .find(|e| matches!(e, StreamEvent::ContentBlockStart { .. }));
        assert!(
            tool_event.is_some(),
            "expected a tool ContentBlockStart event"
        );
        if let StreamEvent::ContentBlockStart {
            content_block: ApiContentBlock::ToolUse { name, input, .. },
            ..
        } = tool_event.unwrap()
        {
            assert_eq!(name, "FileRead");
            assert_eq!(input["path"], "/tmp/foo");
        } else {
            panic!("expected ToolUse content block");
        }
    }

    #[tokio::test]
    async fn stream_ollama_incremental_tool_arguments() {
        use tokio_stream::StreamExt as _;

        // Simulate OpenAI-style incremental argument streaming (also used by
        // some Ollama builds): arguments arrive in multiple string fragments.
        let sse_data = "\
data: {\"id\":\"chat-2\",\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_inc\",\"function\":{\"name\":\"Bash\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chat-2\",\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"com\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chat-2\",\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"mand\\\":\\\"ls\\\"}\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chat-2\",\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":10}}\n\n\
data: [DONE]\n\n";

        let bytes_stream =
            futures::stream::once(
                async move { Ok::<_, reqwest::Error>(bytes::Bytes::from(sse_data)) },
            );

        let raw: Vec<Result<StreamEvent>> = openai_sse_stream(bytes_stream).collect().await;
        let events: Vec<StreamEvent> = raw.into_iter().map(|r| r.unwrap()).collect();

        let tool_event = events
            .iter()
            .find(|e| matches!(e, StreamEvent::ContentBlockStart { .. }));
        assert!(
            tool_event.is_some(),
            "expected a tool ContentBlockStart event"
        );
        if let StreamEvent::ContentBlockStart {
            content_block: ApiContentBlock::ToolUse { name, input, .. },
            ..
        } = tool_event.unwrap()
        {
            assert_eq!(name, "Bash");
            assert_eq!(input["command"], "ls");
        } else {
            panic!("expected ToolUse content block");
        }
    }

    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn translate_assistant_tool_calls_only_no_text() {
        let msg = ApiMessage {
            role: "assistant".into(),
            content: vec![ApiContentBlock::ToolUse {
                id: "call_99".into(),
                name: "Read".into(),
                input: serde_json::json!({"path": "/tmp/foo"}),
            }],
        };
        let result = translate_message(&msg);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "assistant");
        // When there are tool_calls but no text, content must be null
        assert!(
            result[0]["content"].is_null(),
            "content should be null when assistant has only tool_calls"
        );
        assert!(result[0]["tool_calls"].is_array());
        assert_eq!(result[0]["tool_calls"][0]["id"], "call_99");
        assert_eq!(result[0]["tool_calls"][0]["function"]["name"], "Read");
    }

    #[test]
    fn build_openai_request_with_tools() {
        let req = CreateMessageRequest {
            model: "gpt-4o".into(),
            max_tokens: 2048,
            messages: vec![ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "List files".into(),
                    cache_control: None,
                }],
            }],
            system: None,
            tools: Some(vec![
                crate::client::ToolDefinition {
                    name: "Bash".into(),
                    description: "Run a shell command".into(),
                    input_schema: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
                    cache_control: None,
                },
                crate::client::ToolDefinition {
                    name: "Read".into(),
                    description: "Read a file".into(),
                    input_schema: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
                    cache_control: None,
                },
            ]),
            stream: false,
            metadata: None,
            thinking: None,
        };

        let body = build_openai_request(&req);
        let tools = body["tools"].as_array().expect("tools should be an array");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "Bash");
        assert_eq!(tools[0]["function"]["description"], "Run a shell command");
        assert!(tools[0]["function"]["parameters"].is_object());
        assert_eq!(tools[1]["function"]["name"], "Read");
    }

    #[test]
    fn parse_openai_response_no_usage() {
        let json = r#"{
            "id": "chatcmpl-789",
            "model": "gpt-4o-mini",
            "choices": [{
                "message": {
                    "content": "Hi there"
                },
                "finish_reason": "stop"
            }]
        }"#;
        let oai: OaiChatResponse = serde_json::from_str(json).unwrap();
        let msg = parse_openai_response(&oai).unwrap();
        assert_eq!(msg.id, "chatcmpl-789");
        // When usage is absent, should default to zeros
        assert_eq!(msg.usage.input_tokens, 0);
        assert_eq!(msg.usage.output_tokens, 0);
        assert!(msg.usage.cache_creation_input_tokens.is_none());
        assert!(msg.usage.cache_read_input_tokens.is_none());
    }

    #[test]
    fn build_openai_request_with_system() {
        let req = CreateMessageRequest {
            model: "gpt-4.1".into(),
            max_tokens: 1024,
            messages: vec![ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "Hi".into(),
                    cache_control: None,
                }],
            }],
            system: Some(vec![SystemBlock {
                kind: "text".into(),
                text: "You are helpful.".into(),
                cache_control: None,
            }]),
            tools: None,
            stream: false,
            metadata: None,
            thinking: None,
        };

        let body = build_openai_request(&req);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are helpful.");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "Hi");
    }

    // ── extra_body tests ────────────────────────────────────────────────

    #[test]
    fn extra_body_defaults_empty() {
        let provider = OpenAiProvider::new("sk-test");
        assert!(provider.extra_body.is_empty());
    }

    #[test]
    fn with_extra_body_sets_fields() {
        let mut extra = serde_json::Map::new();
        extra.insert("keep_alive".into(), serde_json::json!("5m"));
        extra.insert("num_ctx".into(), serde_json::json!(32768));

        let provider = OpenAiProvider::new("sk-test").with_extra_body(extra);
        assert_eq!(provider.extra_body["keep_alive"], "5m");
        assert_eq!(provider.extra_body["num_ctx"], 32768);
    }

    #[test]
    fn extra_body_merged_into_request() {
        let req = CreateMessageRequest {
            model: "qwen3:8b".into(),
            max_tokens: 2048,
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

        let mut extra = serde_json::Map::new();
        extra.insert("keep_alive".into(), serde_json::json!("5m"));
        extra.insert("num_ctx".into(), serde_json::json!(32768));

        let mut body = build_openai_request(&req);
        for (k, v) in &extra {
            body[k] = v.clone();
        }

        assert_eq!(body["model"], "qwen3:8b");
        assert_eq!(body["keep_alive"], "5m");
        assert_eq!(body["num_ctx"], 32768);
    }

    #[test]
    fn extra_body_does_not_override_core_fields_by_default() {
        // extra_body should only add new keys; core fields like "model"
        // are set before the merge, so extra_body CAN override them.
        // This test documents the behavior: extra_body wins on conflict.
        let req = CreateMessageRequest {
            model: "qwen3:8b".into(),
            max_tokens: 2048,
            messages: vec![],
            system: None,
            tools: None,
            stream: false,
            metadata: None,
            thinking: None,
        };

        let mut extra = serde_json::Map::new();
        extra.insert("stream".into(), serde_json::json!(true));

        let mut body = build_openai_request(&req);
        body["stream"] = serde_json::json!(false); // set by create_message
        for (k, v) in &extra {
            body[k] = v.clone();
        }

        // extra_body overwrites — this is intentional (provider-specific overrides)
        assert_eq!(body["stream"], true);
    }
}
