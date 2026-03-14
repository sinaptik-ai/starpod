use serde::{Deserialize, Serialize};

/// An incoming chat message from a user/channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// The text content of the message.
    pub text: String,

    /// Optional user identifier.
    #[serde(default)]
    pub user_id: Option<String>,

    /// Optional channel identifier (e.g. "telegram", "discord", "web").
    #[serde(default)]
    pub channel_id: Option<String>,

    /// Optional session key within a channel (e.g. telegram chat_id, web conversation UUID).
    #[serde(default)]
    pub channel_session_key: Option<String>,

    /// Optional file attachments (paths or URLs).
    #[serde(default)]
    pub attachments: Vec<String>,
}

/// Response from the Orion agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// The text response from Claude.
    pub text: String,

    /// The session ID used for this conversation.
    pub session_id: String,

    /// Token usage for this turn.
    #[serde(default)]
    pub usage: Option<ChatUsage>,
}

/// Token usage summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
}
