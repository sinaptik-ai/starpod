//! Google Gemini provider.

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

const DEFAULT_GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Google Gemini provider.
pub struct GeminiProvider {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    retry_config: RetryConfig,
    pricing: Option<Arc<ModelRegistry>>,
}

impl GeminiProvider {
    /// Create with an API key and default endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, DEFAULT_GEMINI_URL)
    }

    /// Create with an API key and custom base URL.
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            retry_config: RetryConfig::default(),
            pricing: None,
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

    /// Build the endpoint URL for a given model and method.
    fn endpoint(&self, model: &str, method: &str) -> String {
        format!("{}/models/{}:{}", self.base_url, model, method)
    }

    fn default_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-goog-api-key",
            HeaderValue::from_str(&self.api_key).map_err(|_| {
                AgentError::AuthenticationFailed(
                    "API key contains invalid header characters".into(),
                )
            })?,
        );
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
        let detail = serde_json::from_str::<GeminiErrorResponse>(body)
            .map(|e| e.error.message)
            .unwrap_or_else(|_| body.to_string());

        match status.as_u16() {
            401 | 403 => AgentError::AuthenticationFailed(detail),
            400 => AgentError::InvalidRequest(detail),
            429 => AgentError::RateLimited(detail),
            500..=599 => AgentError::ServerError(detail),
            _ => AgentError::Api(format!("HTTP {status}: {detail}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Translation: canonical -> Gemini wire format
// ---------------------------------------------------------------------------

fn build_gemini_request(request: &CreateMessageRequest) -> serde_json::Value {
    let mut body = serde_json::json!({});

    // System instruction
    if let Some(system_blocks) = &request.system {
        let system_text: String = system_blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !system_text.is_empty() {
            body["system_instruction"] = serde_json::json!({
                "parts": [{"text": system_text}]
            });
        }
    }

    // Contents (conversation messages)
    let mut contents: Vec<serde_json::Value> = Vec::new();
    for msg in &request.messages {
        if let Some(gemini_msg) = translate_message_to_gemini(msg) {
            contents.push(gemini_msg);
        }
    }
    body["contents"] = serde_json::json!(contents);

    // Generation config
    let mut gen_config = serde_json::json!({
        "maxOutputTokens": request.max_tokens,
    });

    // Thinking config
    if let Some(thinking) = &request.thinking {
        if thinking.kind == "enabled" {
            let level = match thinking.budget_tokens {
                Some(b) if b <= 4096 => "LOW",
                Some(b) if b <= 16384 => "MEDIUM",
                _ => "HIGH",
            };
            gen_config["thinkingConfig"] = serde_json::json!({
                "thinkingLevel": level,
            });
        }
    }
    body["generationConfig"] = gen_config;

    // Tools
    if let Some(tools) = &request.tools {
        let func_decls: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                })
            })
            .collect();
        if !func_decls.is_empty() {
            body["tools"] = serde_json::json!([{
                "functionDeclarations": func_decls,
            }]);
        }
    }

    body
}

fn translate_message_to_gemini(msg: &ApiMessage) -> Option<serde_json::Value> {
    let role = match msg.role.as_str() {
        "user" => "user",
        "assistant" => "model",
        _ => return None,
    };

    let mut parts: Vec<serde_json::Value> = Vec::new();

    for block in &msg.content {
        match block {
            ApiContentBlock::Text { text, .. } => {
                parts.push(serde_json::json!({"text": text}));
            }
            ApiContentBlock::ToolUse { id: _, name, input } => {
                parts.push(serde_json::json!({
                    "functionCall": {
                        "name": name,
                        "args": input,
                    }
                }));
            }
            ApiContentBlock::ToolResult {
                tool_use_id,
                content,
                name,
                ..
            } => {
                // Gemini uses functionResponse keyed by function name.
                // Use the stored tool name; fall back to tool_use_id for
                // backward compatibility with older serialized messages.
                let fn_name = name.as_deref().unwrap_or(tool_use_id);
                let text = match content {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                parts.push(serde_json::json!({
                    "functionResponse": {
                        "name": fn_name,
                        "response": {"result": text},
                    }
                }));
            }
            ApiContentBlock::Thinking { thinking } => {
                parts.push(serde_json::json!({"text": format!("<thinking>{thinking}</thinking>")}));
            }
            ApiContentBlock::Image { source } => {
                // Gemini supports inline images via inlineData
                parts.push(serde_json::json!({
                    "inlineData": {
                        "mimeType": source.media_type,
                        "data": source.data,
                    }
                }));
            }
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(serde_json::json!({
        "role": role,
        "parts": parts,
    }))
}

// ---------------------------------------------------------------------------
// Translation: Gemini response -> canonical
// ---------------------------------------------------------------------------

fn parse_gemini_response(resp: &GeminiResponse, model: &str) -> Result<MessageResponse> {
    let candidate = resp
        .candidates
        .as_ref()
        .and_then(|c| c.first())
        .ok_or_else(|| AgentError::Api("No candidates in Gemini response".into()))?;

    let mut content: Vec<ApiContentBlock> = Vec::new();
    let mut tool_call_counter = 0;

    if let Some(parts) = &candidate.content.parts {
        for part in parts {
            if let Some(text) = &part.text {
                content.push(ApiContentBlock::Text {
                    text: text.clone(),
                    cache_control: None,
                });
            }
            if let Some(fc) = &part.function_call {
                let id = format!("gemini_call_{}", tool_call_counter);
                tool_call_counter += 1;
                content.push(ApiContentBlock::ToolUse {
                    id,
                    name: fc.name.clone(),
                    input: fc.args.clone().unwrap_or(serde_json::json!({})),
                });
            }
        }
    }

    let has_tool_use = content.iter().any(|b| matches!(b, ApiContentBlock::ToolUse { .. }));
    let stop_reason = if has_tool_use {
        Some("tool_use".to_string())
    } else {
        match candidate.finish_reason.as_deref() {
            Some("STOP") => Some("end_turn".to_string()),
            Some("MAX_TOKENS") => Some("max_tokens".to_string()),
            other => other.map(String::from),
        }
    };

    let usage = if let Some(u) = &resp.usage_metadata {
        ApiUsage {
            input_tokens: u.prompt_token_count.unwrap_or(0),
            output_tokens: u.candidates_token_count.unwrap_or(0),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: u.cached_content_token_count,
        }
    } else {
        ApiUsage::default()
    };

    Ok(MessageResponse {
        id: format!("gemini-{}", uuid::Uuid::new_v4()),
        role: "assistant".to_string(),
        content,
        model: model.to_string(),
        stop_reason,
        usage,
    })
}

// ---------------------------------------------------------------------------
// Gemini wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    content: GeminiContent,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContent {
    #[allow(dead_code)]
    role: Option<String>,
    parts: Option<Vec<GeminiPart>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    text: Option<String>,
    function_call: Option<GeminiFunctionCall>,
    #[allow(dead_code)]
    function_response: Option<GeminiFunctionResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionCall {
    name: String,
    args: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GeminiFunctionResponse {
    name: String,
    response: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    prompt_token_count: Option<u64>,
    candidates_token_count: Option<u64>,
    cached_content_token_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorResponse {
    error: GeminiError,
}

#[derive(Debug, Deserialize)]
struct GeminiError {
    message: String,
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_use: true,
            thinking: true,
            prompt_caching: false,
        }
    }

    fn cost_rates(&self, model: &str) -> CostRates {
        if let Some(ref registry) = self.pricing {
            if let Some(rates) = registry.get_pricing("gemini", model) {
                return rates;
            }
        }
        // Hardcoded fallback
        let cache = (Some(0.1), Some(1.0));
        match model {
            m if m.contains("flash") => CostRates {
                input_per_million: 0.30,
                output_per_million: 2.50,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            m if m.contains("pro") => CostRates {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            _ => CostRates {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
        }
    }

    async fn create_message(&self, request: &CreateMessageRequest) -> Result<MessageResponse> {
        let body = build_gemini_request(request);
        let url = self.endpoint(&request.model, "generateContent");

        let mut attempt: u32 = 0;
        loop {
            let response = self
                .http
                .post(&url)
                .headers(self.default_headers()?)
                .json(&body)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let gemini_resp: GeminiResponse = response.json().await?;
                return parse_gemini_response(&gemini_resp, &request.model);
            }

            let error_body = response.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if Self::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = self.backoff_duration(attempt);
                warn!(
                    status = status.as_u16(),
                    attempt,
                    wait_secs = wait.as_secs_f64(),
                    "Retryable Gemini API error, backing off"
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
        let body = build_gemini_request(request);
        let url = format!("{}?alt=sse", self.endpoint(&request.model, "streamGenerateContent"));

        let mut attempt: u32 = 0;
        let response = loop {
            let resp = self
                .http
                .post(&url)
                .headers(self.default_headers()?)
                .json(&body)
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
                warn!(status = status.as_u16(), attempt, "Retrying Gemini stream");
                sleep(wait).await;
                attempt += 1;
                continue;
            }

            return Err(Self::status_to_error(status, &error_text));
        };

        let model = request.model.clone();
        let byte_stream = response.bytes_stream();
        let event_stream = gemini_sse_stream(byte_stream, model);
        Ok(Box::pin(event_stream))
    }
}

/// Parse Gemini SSE stream into canonical StreamEvents.
fn gemini_sse_stream(
    byte_stream: impl Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
    model: String,
) -> impl Stream<Item = Result<StreamEvent>> + Send + 'static {
    async_stream::stream! {
        let mut buf = String::new();
        tokio::pin!(byte_stream);
        let mut emitted_message_start = false;
        let mut tool_call_counter: usize = 0;

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

                let gemini_resp: GeminiResponse = match serde_json::from_str(data) {
                    Ok(r) => r,
                    Err(e) => {
                        debug!("Failed to parse Gemini SSE chunk: {}", e);
                        continue;
                    }
                };

                if !emitted_message_start {
                    emitted_message_start = true;
                    yield Ok(StreamEvent::MessageStart {
                        message: MessageResponse {
                            id: format!("gemini-stream-{}", uuid::Uuid::new_v4()),
                            role: "assistant".to_string(),
                            content: vec![],
                            model: model.clone(),
                            stop_reason: None,
                            usage: ApiUsage::default(),
                        },
                    });
                }

                if let Some(candidates) = &gemini_resp.candidates {
                    for candidate in candidates {
                        if let Some(parts) = &candidate.content.parts {
                            for part in parts {
                                if let Some(text) = &part.text {
                                    if !text.is_empty() {
                                        yield Ok(StreamEvent::ContentBlockDelta {
                                            index: 0,
                                            delta: ContentDelta::TextDelta { text: text.clone() },
                                        });
                                    }
                                }
                                if let Some(fc) = &part.function_call {
                                    let id = format!("gemini_call_{}", tool_call_counter);
                                    tool_call_counter += 1;
                                    yield Ok(StreamEvent::ContentBlockStart {
                                        index: tool_call_counter,
                                        content_block: ApiContentBlock::ToolUse {
                                            id,
                                            name: fc.name.clone(),
                                            input: fc.args.clone().unwrap_or(serde_json::json!({})),
                                        },
                                    });
                                    yield Ok(StreamEvent::ContentBlockStop {
                                        index: tool_call_counter,
                                    });
                                }
                            }
                        }

                        if let Some(reason) = &candidate.finish_reason {
                            let stop_reason = match reason.as_str() {
                                "STOP" => "end_turn",
                                "MAX_TOKENS" => "max_tokens",
                                _ => reason.as_str(),
                            };

                            let usage = if let Some(u) = &gemini_resp.usage_metadata {
                                ApiUsage {
                                    input_tokens: u.prompt_token_count.unwrap_or(0),
                                    output_tokens: u.candidates_token_count.unwrap_or(0),
                                    cache_creation_input_tokens: None,
                                    cache_read_input_tokens: u.cached_content_token_count,
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
        }

        yield Ok(StreamEvent::MessageStop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{SystemBlock, ToolDefinition};

    #[test]
    fn build_gemini_request_basic() {
        let req = CreateMessageRequest {
            model: "gemini-2.5-flash".into(),
            max_tokens: 1024,
            messages: vec![ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "Hello".into(),
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

        let body = build_gemini_request(&req);

        // System instruction
        assert_eq!(
            body["system_instruction"]["parts"][0]["text"],
            "You are helpful."
        );

        // Contents
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "Hello");
    }

    #[test]
    fn build_gemini_request_with_tools() {
        let req = CreateMessageRequest {
            model: "gemini-2.5-flash".into(),
            max_tokens: 1024,
            messages: vec![ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "List files".into(),
                    cache_control: None,
                }],
            }],
            system: None,
            tools: Some(vec![ToolDefinition {
                name: "Bash".into(),
                description: "Run a command".into(),
                input_schema: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
                cache_control: None,
            }]),
            stream: false,
            metadata: None,
            thinking: None,
        };

        let body = build_gemini_request(&req);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        let decls = tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls[0]["name"], "Bash");
    }

    #[test]
    fn parse_gemini_response_text() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "Hello there!"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 3
            }
        }"#;
        let resp: GeminiResponse = serde_json::from_str(json).unwrap();
        let msg = parse_gemini_response(&resp, "gemini-2.5-flash").unwrap();
        assert_eq!(msg.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(msg.usage.input_tokens, 5);
        assert_eq!(msg.usage.output_tokens, 3);
        match &msg.content[0] {
            ApiContentBlock::Text { text, .. } => assert_eq!(text, "Hello there!"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn parse_gemini_response_with_function_call() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "Bash",
                            "args": {"command": "ls -la"}
                        }
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 8
            }
        }"#;
        let resp: GeminiResponse = serde_json::from_str(json).unwrap();
        let msg = parse_gemini_response(&resp, "gemini-2.5-flash").unwrap();
        assert_eq!(msg.stop_reason.as_deref(), Some("tool_use"));
        match &msg.content[0] {
            ApiContentBlock::ToolUse { name, input, .. } => {
                assert_eq!(name, "Bash");
                assert_eq!(input, &serde_json::json!({"command": "ls -la"}));
            }
            _ => panic!("expected tool use"),
        }
    }

    #[test]
    fn build_gemini_request_with_thinking_config() {
        let req = CreateMessageRequest {
            model: "gemini-2.5-flash".into(),
            max_tokens: 4096,
            messages: vec![ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "Think hard about this".into(),
                    cache_control: None,
                }],
            }],
            system: None,
            tools: None,
            stream: false,
            metadata: None,
            thinking: Some(crate::client::ThinkingParam {
                kind: "enabled".into(),
                budget_tokens: Some(8192),
            }),
        };

        let body = build_gemini_request(&req);
        let gen_config = &body["generationConfig"];
        assert_eq!(gen_config["maxOutputTokens"], 4096);
        // 8192 budget => MEDIUM level (4096 < 8192 <= 16384)
        assert_eq!(gen_config["thinkingConfig"]["thinkingLevel"], "MEDIUM");
    }

    #[test]
    fn build_gemini_request_with_thinking_low() {
        let req = CreateMessageRequest {
            model: "gemini-2.5-flash".into(),
            max_tokens: 1024,
            messages: vec![ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "Quick question".into(),
                    cache_control: None,
                }],
            }],
            system: None,
            tools: None,
            stream: false,
            metadata: None,
            thinking: Some(crate::client::ThinkingParam {
                kind: "enabled".into(),
                budget_tokens: Some(2048),
            }),
        };

        let body = build_gemini_request(&req);
        // 2048 <= 4096 => LOW
        assert_eq!(body["generationConfig"]["thinkingConfig"]["thinkingLevel"], "LOW");
    }

    #[test]
    fn build_gemini_request_with_thinking_high() {
        let req = CreateMessageRequest {
            model: "gemini-2.5-flash".into(),
            max_tokens: 1024,
            messages: vec![ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "Complex problem".into(),
                    cache_control: None,
                }],
            }],
            system: None,
            tools: None,
            stream: false,
            metadata: None,
            thinking: Some(crate::client::ThinkingParam {
                kind: "enabled".into(),
                budget_tokens: Some(32768),
            }),
        };

        let body = build_gemini_request(&req);
        // 32768 > 16384 => HIGH
        assert_eq!(body["generationConfig"]["thinkingConfig"]["thinkingLevel"], "HIGH");
    }

    #[test]
    fn translate_assistant_with_tool_call_to_gemini() {
        let msg = ApiMessage {
            role: "assistant".into(),
            content: vec![
                ApiContentBlock::Text {
                    text: "Let me run that.".into(),
                    cache_control: None,
                },
                ApiContentBlock::ToolUse {
                    id: "call_42".into(),
                    name: "Bash".into(),
                    input: serde_json::json!({"command": "pwd"}),
                },
            ],
        };
        let result = translate_message_to_gemini(&msg).unwrap();
        assert_eq!(result["role"], "model");
        let parts = result["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "Let me run that.");
        assert_eq!(parts[1]["functionCall"]["name"], "Bash");
        assert_eq!(parts[1]["functionCall"]["args"]["command"], "pwd");
    }

    #[test]
    fn translate_tool_result_to_gemini() {
        let msg = ApiMessage {
            role: "user".into(),
            content: vec![ApiContentBlock::ToolResult {
                tool_use_id: "call_123".into(),
                content: serde_json::json!("file1.txt"),
                is_error: None,
                cache_control: None,
                name: Some("Bash".into()),
            }],
        };
        let result = translate_message_to_gemini(&msg).unwrap();
        assert_eq!(result["role"], "user");
        let func_resp = &result["parts"][0]["functionResponse"];
        assert!(func_resp.is_object());
        // The function name should be "Bash", NOT the tool_use_id "call_123"
        assert_eq!(func_resp["name"], "Bash");
    }

    #[test]
    fn translate_tool_result_falls_back_to_id_when_no_name() {
        let msg = ApiMessage {
            role: "user".into(),
            content: vec![ApiContentBlock::ToolResult {
                tool_use_id: "legacy_call_456".into(),
                content: serde_json::json!("output"),
                is_error: None,
                cache_control: None,
                name: None, // no name stored (backward compat)
            }],
        };
        let result = translate_message_to_gemini(&msg).unwrap();
        let func_resp = &result["parts"][0]["functionResponse"];
        // Falls back to tool_use_id when name is missing
        assert_eq!(func_resp["name"], "legacy_call_456");
    }
}
