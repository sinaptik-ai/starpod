use serde::{Deserialize, Serialize};

/// Maximum attachment size in bytes (20 MB).
pub const MAX_ATTACHMENT_SIZE: usize = 20 * 1024 * 1024;

/// A file attachment (image, PDF, etc.) carried as base64.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// Original filename (e.g. "photo.png").
    pub file_name: String,
    /// MIME type (e.g. "image/png", "application/pdf").
    pub mime_type: String,
    /// Base64-encoded file content.
    pub data: String,
}

impl Attachment {
    /// Whether this attachment is an image that Claude can process via vision.
    pub fn is_image(&self) -> bool {
        matches!(
            self.mime_type.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        )
    }

    /// Approximate raw (decoded) size in bytes.
    pub fn raw_size(&self) -> usize {
        self.data.len() * 3 / 4
    }
}

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

    /// File attachments (images, PDFs, etc.).
    #[serde(default)]
    pub attachments: Vec<Attachment>,

    /// If this message was triggered by a cron job or heartbeat, the job name.
    /// `None` for regular user messages.
    #[serde(default)]
    pub triggered_by: Option<String>,
}

/// Response from the Starpod agent.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_is_image_detects_image_types() {
        let png = Attachment {
            file_name: "photo.png".into(),
            mime_type: "image/png".into(),
            data: String::new(),
        };
        assert!(png.is_image());

        let jpeg = Attachment {
            file_name: "photo.jpg".into(),
            mime_type: "image/jpeg".into(),
            data: String::new(),
        };
        assert!(jpeg.is_image());

        let gif = Attachment {
            file_name: "anim.gif".into(),
            mime_type: "image/gif".into(),
            data: String::new(),
        };
        assert!(gif.is_image());

        let webp = Attachment {
            file_name: "pic.webp".into(),
            mime_type: "image/webp".into(),
            data: String::new(),
        };
        assert!(webp.is_image());
    }

    #[test]
    fn attachment_is_image_rejects_non_images() {
        let pdf = Attachment {
            file_name: "doc.pdf".into(),
            mime_type: "application/pdf".into(),
            data: String::new(),
        };
        assert!(!pdf.is_image());

        let text = Attachment {
            file_name: "notes.txt".into(),
            mime_type: "text/plain".into(),
            data: String::new(),
        };
        assert!(!text.is_image());
    }

    #[test]
    fn attachment_raw_size_approximation() {
        // 100 base64 chars ≈ 75 raw bytes
        let att = Attachment {
            file_name: "f".into(),
            mime_type: "image/png".into(),
            data: "x".repeat(100),
        };
        assert_eq!(att.raw_size(), 75);
    }

    #[test]
    fn max_attachment_size_is_20mb() {
        assert_eq!(MAX_ATTACHMENT_SIZE, 20 * 1024 * 1024);
    }

    #[test]
    fn chat_message_with_attachments_roundtrips() {
        let msg = ChatMessage {
            text: "Look at this".into(),
            user_id: None,
            channel_id: Some("web".into()),
            channel_session_key: None,
            attachments: vec![Attachment {
                file_name: "photo.png".into(),
                mime_type: "image/png".into(),
                data: "iVBORw0KGgo=".into(),
            }],
            triggered_by: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.attachments.len(), 1);
        assert_eq!(back.attachments[0].file_name, "photo.png");
        assert!(back.attachments[0].is_image());
    }

    #[test]
    fn chat_message_without_attachments_deserializes() {
        let json = r#"{"text": "hello"}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert!(msg.attachments.is_empty());
    }
}
