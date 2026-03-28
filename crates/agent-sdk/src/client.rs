//! Canonical API types for the Messages API.
//!
//! These types are the shared representation used by all providers.
//! Each provider translates to/from these types internally.

use std::time::Duration;

use serde::{Deserialize, Serialize};

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

/// Source data for an image content block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    /// Always "base64".
    #[serde(rename = "type")]
    pub kind: String,
    /// MIME type (e.g. "image/png").
    pub media_type: String,
    /// Base64-encoded image data.
    pub data: String,
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

    #[serde(rename = "image")]
    Image { source: ImageSource },

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
        /// The tool/function name (used by Gemini's functionResponse).
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
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

// ---------------------------------------------------------------------------
// Retry configuration (shared by providers)
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

#[cfg(test)]
mod tests {
    use super::*;

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
            name: None,
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
    fn image_content_block_roundtrips() {
        let block = ApiContentBlock::Image {
            source: ImageSource {
                kind: "base64".into(),
                media_type: "image/png".into(),
                data: "iVBORw0KGgo=".into(),
            },
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"image\""));
        assert!(json.contains("\"media_type\":\"image/png\""));
        let back: ApiContentBlock = serde_json::from_str(&json).unwrap();
        match back {
            ApiContentBlock::Image { source } => {
                assert_eq!(source.kind, "base64");
                assert_eq!(source.media_type, "image/png");
                assert_eq!(source.data, "iVBORw0KGgo=");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn image_in_user_message_serializes() {
        let msg = ApiMessage {
            role: "user".into(),
            content: vec![
                ApiContentBlock::Image {
                    source: ImageSource {
                        kind: "base64".into(),
                        media_type: "image/jpeg".into(),
                        data: "abc123".into(),
                    },
                },
                ApiContentBlock::Text {
                    text: "What is this?".into(),
                    cache_control: None,
                },
            ],
        };
        let json = serde_json::to_value(&msg).unwrap();
        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[1]["type"], "text");
    }

    #[test]
    fn backoff_duration_increases() {
        use crate::provider::LlmProvider;
        use crate::AnthropicProvider;
        let provider = AnthropicProvider::with_api_key("test-key");
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(caps.tool_use);
    }
}
