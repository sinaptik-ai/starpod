//! Conversation compaction — summarize older messages when approaching context limits.
//!
//! When `input_tokens` from the API response exceeds the configured `context_budget`,
//! older messages are summarized via a cheaper model call and replaced with a compact
//! summary, preserving the system prompt and recent turns.

use tracing::{debug, warn};

use crate::client::{ApiContentBlock, ApiMessage, CreateMessageRequest};
use crate::error::Result;
use crate::provider::LlmProvider;

/// Default model for compaction summaries.
pub const DEFAULT_COMPACTION_MODEL: &str = "claude-haiku-4-5";

/// Default minimum number of messages to keep at the end (never compact below this).
pub const DEFAULT_MIN_KEEP_MESSAGES: usize = 4;

/// Default max tokens for the summarization response.
pub const DEFAULT_SUMMARY_MAX_TOKENS: u32 = 4096;

/// Check whether compaction should trigger.
pub fn should_compact(input_tokens: u64, context_budget: u64) -> bool {
    input_tokens > context_budget
}

/// Find the split point — index where old messages end and recent messages begin.
///
/// Rules:
/// - Keep at least `min_keep` messages at the end (falls back to `DEFAULT_MIN_KEEP_MESSAGES`).
/// - Never split inside a tool-use cycle (assistant with tool_use followed by
///   user with tool_result must stay together).
/// - Returns 0 if the conversation is too short to compact.
pub fn find_split_point(conversation: &[ApiMessage], min_keep: usize) -> usize {
    if conversation.len() <= min_keep {
        return 0;
    }

    // Start candidate: keep min_keep from the end
    let mut split = conversation.len() - min_keep;

    // Walk backwards to find a clean boundary (not inside a tool cycle).
    // A tool cycle is: assistant message with ToolUse blocks followed by a user
    // message with ToolResult blocks. We must not split between them.
    while split > 0 {
        // Check if the message at `split` is a user message with tool results
        // and the message before it is an assistant with tool uses — if so, the
        // split would break a tool cycle, so move split backwards to before the
        // assistant message.
        if split < conversation.len() {
            let msg = &conversation[split];
            if msg.role == "user" && has_tool_results(&msg.content) {
                // This user message has tool results — check if prior is assistant with tool_use
                if split > 0 {
                    let prev = &conversation[split - 1];
                    if prev.role == "assistant" && has_tool_uses(&prev.content) {
                        // Can't split here — move before the assistant message
                        split -= 1;
                        continue;
                    }
                }
            }
        }
        break;
    }

    split
}

/// Build the summarization prompt from old messages.
pub fn build_summary_prompt(old_messages: &[ApiMessage]) -> String {
    let mut rendered = String::new();

    for msg in old_messages {
        rendered.push_str(&format!("[{}]\n", msg.role));
        for block in &msg.content {
            match block {
                ApiContentBlock::Text { text, .. } => {
                    rendered.push_str(text);
                    rendered.push('\n');
                }
                ApiContentBlock::ToolUse { name, input, .. } => {
                    rendered.push_str(&format!("Tool call: {} input: {}\n", name, input));
                }
                ApiContentBlock::ToolResult { content, is_error, .. } => {
                    let label = if *is_error == Some(true) { "error" } else { "result" };
                    // Truncate long tool results
                    let content_str = content.to_string();
                    if content_str.len() > 500 {
                        let mut end = 500;
                        while end > 0 && !content_str.is_char_boundary(end) { end -= 1; }
                        rendered.push_str(&format!("Tool {}: {}...\n", label, &content_str[..end]));
                    } else {
                        rendered.push_str(&format!("Tool {}: {}\n", label, content_str));
                    }
                }
                ApiContentBlock::Thinking { thinking } => {
                    // Skip thinking blocks in summary — they're internal reasoning
                    if thinking.len() <= 200 {
                        rendered.push_str(&format!("(thinking: {})\n", thinking));
                    }
                }
                ApiContentBlock::Image { .. } => {
                    rendered.push_str("[image]\n");
                }
            }
        }
        rendered.push('\n');
    }

    format!(
        "Summarize the following conversation segment concisely. Preserve:\n\
         - Key decisions made\n\
         - Important facts and context established\n\
         - File paths and code references mentioned\n\
         - Tool results and their outcomes\n\
         - Any commitments or action items\n\n\
         Format as a structured summary with sections.\n\n\
         <conversation>\n{rendered}</conversation>"
    )
}

/// Call the summarizer model via an LLM provider. Falls back to `fallback_model` on failure.
///
/// If a separate `fallback_provider` is given it is used for the retry; otherwise
/// the same `provider` is reused with `fallback_model`.
pub async fn call_summarizer(
    provider: &dyn LlmProvider,
    summary_prompt: &str,
    compaction_model: &str,
    fallback_provider: Option<&dyn LlmProvider>,
    fallback_model: &str,
    summary_max_tokens: u32,
) -> Result<String> {
    let request = CreateMessageRequest {
        model: compaction_model.to_string(),
        max_tokens: summary_max_tokens,
        messages: vec![ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: summary_prompt.to_string(),
                cache_control: None,
            }],
        }],
        system: None,
        tools: None,
        stream: false,
        metadata: None,
        thinking: None,
    };

    match provider.create_message(&request).await {
        Ok(resp) => extract_text(&resp.content),
        Err(e) => {
            warn!(
                model = compaction_model,
                error = %e,
                "Compaction model failed, falling back to primary model"
            );
            // Retry with the fallback (primary) model/provider
            let mut fallback_req = request;
            fallback_req.model = fallback_model.to_string();
            let fb = fallback_provider.unwrap_or(provider);
            let resp = fb.create_message(&fallback_req).await?;
            extract_text(&resp.content)
        }
    }
}

/// Replace old messages with a summary message.
pub fn splice_conversation(
    conversation: &mut Vec<ApiMessage>,
    split_point: usize,
    summary: &str,
) {
    // Remove old messages
    conversation.drain(..split_point);

    // Insert summary as the first message
    conversation.insert(
        0,
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: format!(
                    "[Previous conversation summary]\n{summary}\n[End of summary — conversation continues below]"
                ),
                cache_control: None,
            }],
        },
    );
}

/// Result of a compaction operation.
#[derive(Debug)]
pub struct CompactResult {
    pub pre_tokens: u64,
    pub summary: String,
    pub messages_compacted: usize,
}

// ── helpers ──────────────────────────────────────────────────────────────

fn has_tool_uses(blocks: &[ApiContentBlock]) -> bool {
    blocks.iter().any(|b| matches!(b, ApiContentBlock::ToolUse { .. }))
}

fn has_tool_results(blocks: &[ApiContentBlock]) -> bool {
    blocks.iter().any(|b| matches!(b, ApiContentBlock::ToolResult { .. }))
}

fn extract_text(content: &[ApiContentBlock]) -> Result<String> {
    let text: String = content
        .iter()
        .filter_map(|b| match b {
            ApiContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    if text.is_empty() {
        Err(crate::error::AgentError::Api(
            "Compaction response contained no text".into(),
        ))
    } else {
        debug!(summary_len = text.len(), "Generated compaction summary");
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_msg(role: &str, text: &str) -> ApiMessage {
        ApiMessage {
            role: role.to_string(),
            content: vec![ApiContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
        }
    }

    fn tool_use_msg() -> ApiMessage {
        ApiMessage {
            role: "assistant".to_string(),
            content: vec![
                ApiContentBlock::Text {
                    text: "Let me check.".to_string(),
                    cache_control: None,
                },
                ApiContentBlock::ToolUse {
                    id: "tu_1".to_string(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({"command": "ls"}),
                },
            ],
        }
    }

    fn tool_result_msg() -> ApiMessage {
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::ToolResult {
                tool_use_id: "tu_1".to_string(),
                content: serde_json::json!("file1.rs\nfile2.rs"),
                is_error: None,
                cache_control: None,
                name: None,
            }],
        }
    }

    #[test]
    fn test_should_compact_threshold() {
        assert!(!should_compact(150_000, 160_000));
        assert!(should_compact(170_000, 160_000));
        assert!(should_compact(160_001, 160_000));
        assert!(!should_compact(160_000, 160_000));
    }

    #[test]
    fn test_find_split_point_preserves_recent() {
        let conv: Vec<ApiMessage> = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                text_msg(role, &format!("message {i}"))
            })
            .collect();

        let split = find_split_point(&conv, DEFAULT_MIN_KEEP_MESSAGES);
        // Should keep at least DEFAULT_MIN_KEEP_MESSAGES (4) at the end
        assert!(conv.len() - split >= DEFAULT_MIN_KEEP_MESSAGES);
        assert_eq!(split, 6); // 10 - 4 = 6
    }

    #[test]
    fn test_find_split_point_respects_tool_boundaries() {
        // Conversation: user, assistant, user, assistant(tool_use), user(tool_result), assistant, user
        // 7 messages total, MIN_KEEP = 4, so split candidate = 3
        // Index 3 = assistant(tool_use), index 4 = user(tool_result)
        // Split at 3 means keeping [3..7] which includes the tool pair — that's fine
        // But what if split would land at index 4 (user with tool_result)?
        let conv = vec![
            text_msg("user", "hello"),           // 0
            text_msg("assistant", "hi"),          // 1
            text_msg("user", "do something"),     // 2
            tool_use_msg(),                       // 3 - assistant with tool_use
            tool_result_msg(),                    // 4 - user with tool_result
            text_msg("assistant", "done"),        // 5
            text_msg("user", "thanks"),           // 6
        ];

        let split = find_split_point(&conv, DEFAULT_MIN_KEEP_MESSAGES);
        // split candidate = 7 - 4 = 3
        // index 3 is assistant(tool_use), index 4 is user(tool_result)
        // The kept portion [3..] includes both, so split=3 is clean
        assert_eq!(split, 3);
    }

    #[test]
    fn test_find_split_point_moves_back_when_splitting_tool_cycle() {
        // Force the split candidate to land ON a tool_result message
        // 5 messages: user, tool_use, tool_result, assistant, user
        // MIN_KEEP = 4, candidate split = 5 - 4 = 1
        // Index 1 is tool_use(assistant), which is fine — kept portion starts with it
        let conv = vec![
            text_msg("user", "start"),    // 0
            tool_use_msg(),               // 1
            tool_result_msg(),            // 2
            text_msg("assistant", "ok"),  // 3
            text_msg("user", "next"),     // 4
        ];
        let split = find_split_point(&conv, DEFAULT_MIN_KEEP_MESSAGES);
        assert_eq!(split, 1);

        // Now: 6 messages where split=2 lands on tool_result
        let conv2 = vec![
            text_msg("user", "start"),     // 0
            text_msg("assistant", "ack"),   // 1
            tool_result_msg(),             // 2 - user with tool_result (split candidate)
            text_msg("assistant", "done"), // 3
            text_msg("user", "q1"),        // 4
            text_msg("assistant", "a1"),   // 5
        ];
        let split2 = find_split_point(&conv2, DEFAULT_MIN_KEEP_MESSAGES);
        // candidate = 6 - 4 = 2, which is a tool_result user msg
        // prev (index 1) is assistant but no tool_use → no cycle, so split stays at 2
        assert_eq!(split2, 2);
    }

    #[test]
    fn test_find_split_point_too_short() {
        let conv = vec![
            text_msg("user", "hi"),
            text_msg("assistant", "hello"),
            text_msg("user", "bye"),
        ];
        assert_eq!(find_split_point(&conv, DEFAULT_MIN_KEEP_MESSAGES), 0);
    }

    #[test]
    fn test_splice_conversation() {
        let mut conv: Vec<ApiMessage> = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                text_msg(role, &format!("msg {i}"))
            })
            .collect();

        splice_conversation(&mut conv, 6, "Summary of messages 0-5");

        // 1 summary + 4 kept = 5
        assert_eq!(conv.len(), 5);
        // First message should be the summary
        match &conv[0].content[0] {
            ApiContentBlock::Text { text, .. } => {
                assert!(text.contains("Summary of messages 0-5"));
                assert!(text.contains("[Previous conversation summary]"));
            }
            _ => panic!("Expected text block"),
        }
        // Second message should be the old index 6 (msg 6)
        match &conv[1].content[0] {
            ApiContentBlock::Text { text, .. } => assert_eq!(text, "msg 6"),
            _ => panic!("Expected text block"),
        }
    }

    #[test]
    fn test_find_split_point_custom_min_keep() {
        let conv: Vec<ApiMessage> = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                text_msg(role, &format!("message {i}"))
            })
            .collect();

        // min_keep=2 → split at 8 (keeps last 2)
        assert_eq!(find_split_point(&conv, 2), 8);

        // min_keep=6 → split at 4 (keeps last 6)
        assert_eq!(find_split_point(&conv, 6), 4);

        // min_keep=1 → split at 9 (keeps last 1)
        assert_eq!(find_split_point(&conv, 1), 9);
    }

    #[test]
    fn test_build_summary_prompt_format() {
        let msgs = vec![
            text_msg("user", "Tell me about Rust"),
            text_msg("assistant", "Rust is a systems language."),
        ];

        let prompt = build_summary_prompt(&msgs);
        assert!(prompt.contains("Summarize the following"));
        assert!(prompt.contains("[user]"));
        assert!(prompt.contains("Tell me about Rust"));
        assert!(prompt.contains("[assistant]"));
        assert!(prompt.contains("Rust is a systems language."));
        assert!(prompt.contains("<conversation>"));
    }
}
