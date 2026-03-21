//! Silent agentic memory flush — runs a hidden LLM turn before context compaction
//! to persist important information to durable memory.
//!
//! When the agent's context window fills up, the conversation must be compacted
//! (summarized). Before that happens, this module runs a single non-streaming LLM
//! call that reviews the messages about to be discarded and uses `MemoryWrite` and
//! `MemoryAppendDaily` tools to save anything worth keeping.
//!
//! The LLM response text is discarded — only tool calls are executed.
//!
//! # Configuration
//!
//! - `compaction.memory_flush` (default `true`) — enable/disable the flush
//! - `compaction.flush_model` — model to use (falls back to compaction_model, then primary model)
//!
//! # Architecture
//!
//! ```text
//! [Messages to compact] → [Transcript] → [LLM call with memory tools]
//!                                              ↓
//!                                    [Execute tool_use blocks]
//!                                              ↓
//!                              [MemoryWrite / MemoryAppendDaily]
//! ```

use tracing::{debug, warn};

use agent_sdk::client::{
    ApiContentBlock, ApiMessage, CreateMessageRequest, SystemBlock, ToolDefinition,
};
use agent_sdk::LlmProvider;
use starpod_memory::MemoryStore;
use starpod_memory::UserMemoryView;

/// Default system prompt for the memory flush turn.
const FLUSH_SYSTEM_PROMPT: &str = "\
You are a memory management agent. Your ONLY job is to review the conversation below \
and save important information using the provided tools. Be selective — only save \
information that would be useful in future conversations.

Save these kinds of information:
- User preferences, working style, and personal details
- Key decisions and their reasoning
- Important facts, names, dates, and relationships
- Technical context: architecture choices, conventions, configurations
- Action items, commitments, and follow-ups

Do NOT save:
- Trivial or transient exchanges (greetings, small talk)
- Information that's already in MEMORY.md or USER.md
- Raw code or long outputs (summarize instead)
- Temporary debugging context

Use MemoryWrite with append=true to add to MEMORY.md, or MemoryAppendDaily for time-specific notes. \
Respond with ONLY tool calls, no explanatory text.";

/// Tool definitions exposed to the flush LLM turn.
fn flush_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "MemoryWrite".into(),
            description: "Write or append to a memory file.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file": {
                        "type": "string",
                        "description": "Relative file path (e.g. 'MEMORY.md', 'USER.md', 'memory/notes.md')"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write or append"
                    },
                    "append": {
                        "type": "boolean",
                        "description": "If true, append to existing file (default: false)"
                    }
                },
                "required": ["file", "content"]
            }),
            cache_control: None,
        },
        ToolDefinition {
            name: "MemoryAppendDaily".into(),
            description: "Append a timestamped entry to today's daily log.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to append"
                    }
                },
                "required": ["text"]
            }),
            cache_control: None,
        },
    ]
}

/// Serialize compacted messages into a human-readable transcript for the flush LLM.
fn messages_to_transcript(messages: &[ApiMessage]) -> String {
    let mut parts = Vec::new();
    for msg in messages {
        let role = &msg.role;
        for block in &msg.content {
            match block {
                ApiContentBlock::Text { text, .. } => {
                    parts.push(format!("[{}] {}", role, text));
                }
                ApiContentBlock::ToolUse { name, input, .. } => {
                    parts.push(format!("[{}] (tool_use: {} {})", role, name, input));
                }
                ApiContentBlock::ToolResult { content, .. } => {
                    let text = if let Some(s) = content.as_str() {
                        s.to_string()
                    } else {
                        content.to_string()
                    };
                    // Truncate long tool results
                    let truncated = if text.len() > 500 {
                        let mut end = 500;
                        while end > 0 && !text.is_char_boundary(end) { end -= 1; }
                        format!("{}...", &text[..end])
                    } else {
                        text
                    };
                    parts.push(format!("[tool_result] {}", truncated));
                }
                _ => {}
            }
        }
    }
    parts.join("\n\n")
}

/// Execute tool calls from the flush LLM response against the memory store.
async fn execute_flush_tool_calls(
    content: &[ApiContentBlock],
    memory: &MemoryStore,
    user_view: Option<&UserMemoryView>,
) {
    for block in content {
        if let ApiContentBlock::ToolUse { name, input, id } = block {
            debug!(tool = %name, id = %id, "Flush: executing tool call");
            match name.as_str() {
                "MemoryWrite" => {
                    let file = match input.get("file").and_then(|v| v.as_str()) {
                        Some(f) => f,
                        None => continue,
                    };
                    let content_str = match input.get("content").and_then(|v| v.as_str()) {
                        Some(c) => c,
                        None => continue,
                    };
                    let append = input.get("append").and_then(|v| v.as_bool()).unwrap_or(false);

                    let final_content = if append {
                        let existing = if let Some(uv) = user_view {
                            uv.read_file(file).unwrap_or_default()
                        } else {
                            memory.read_file(file).unwrap_or_default()
                        };
                        if existing.is_empty() {
                            content_str.to_string()
                        } else {
                            format!("{}\n{}", existing, content_str)
                        }
                    } else {
                        content_str.to_string()
                    };

                    let result = if let Some(uv) = user_view {
                        uv.write_file(file, &final_content).await
                    } else {
                        memory.write_file(file, &final_content).await
                    };
                    if let Err(e) = result {
                        warn!(file = %file, error = %e, "Flush: MemoryWrite failed");
                    }
                }
                "MemoryAppendDaily" => {
                    let text = match input.get("text").and_then(|v| v.as_str()) {
                        Some(t) => t,
                        None => continue,
                    };

                    let result = if let Some(uv) = user_view {
                        uv.append_daily(text).await
                    } else {
                        memory.append_daily(text).await
                    };
                    if let Err(e) = result {
                        warn!(error = %e, "Flush: MemoryAppendDaily failed");
                    }
                }
                _ => {
                    warn!(tool = %name, "Flush: unknown tool call, ignoring");
                }
            }
        }
    }
}

/// Run the silent agentic memory flush.
///
/// Makes a single non-streaming LLM call with the conversation transcript,
/// then executes any memory tool calls from the response. The LLM's text
/// output is discarded — only `MemoryWrite` and `MemoryAppendDaily` tool
/// calls are acted on.
///
/// The transcript is capped at 30,000 characters to avoid excessive cost.
/// Tool results within the transcript are truncated to 500 characters each.
///
/// If the provider call fails, the error is logged and no memories are saved
/// (fail-open — compaction still proceeds).
pub async fn run_memory_flush(
    provider: &dyn LlmProvider,
    model: &str,
    messages: &[ApiMessage],
    memory: &MemoryStore,
    user_view: Option<&UserMemoryView>,
) {
    let transcript = messages_to_transcript(messages);
    if transcript.trim().is_empty() {
        return;
    }

    // Cap transcript to avoid huge flush requests
    let transcript = if transcript.len() > 30_000 {
        let mut end = 30_000;
        while end > 0 && !transcript.is_char_boundary(end) { end -= 1; }
        format!("{}...\n\n[transcript truncated]", &transcript[..end])
    } else {
        transcript
    };

    let request = CreateMessageRequest {
        model: model.to_string(),
        max_tokens: 4096,
        messages: vec![ApiMessage {
            role: "user".into(),
            content: vec![ApiContentBlock::Text {
                text: format!(
                    "Review this conversation and save important information to memory:\n\n{}",
                    transcript
                ),
                cache_control: None,
            }],
        }],
        system: Some(vec![SystemBlock {
            kind: "text".into(),
            text: FLUSH_SYSTEM_PROMPT.to_string(),
            cache_control: None,
        }]),
        tools: Some(flush_tool_definitions()),
        stream: false,
        metadata: None,
        thinking: None,
    };

    debug!(model = %model, transcript_len = transcript.len(), "Running memory flush");

    match provider.create_message(&request).await {
        Ok(response) => {
            let tool_calls: Vec<_> = response.content.iter()
                .filter(|b| matches!(b, ApiContentBlock::ToolUse { .. }))
                .collect();
            debug!(tool_calls = tool_calls.len(), "Flush: LLM responded");
            execute_flush_tool_calls(&response.content, memory, user_view).await;
        }
        Err(e) => {
            warn!(error = %e, "Memory flush LLM call failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_formats_text_blocks() {
        let messages = vec![
            ApiMessage {
                role: "user".into(),
                content: vec![ApiContentBlock::Text {
                    text: "What is Rust?".into(),
                    cache_control: None,
                }],
            },
            ApiMessage {
                role: "assistant".into(),
                content: vec![ApiContentBlock::Text {
                    text: "Rust is a systems programming language.".into(),
                    cache_control: None,
                }],
            },
        ];

        let transcript = messages_to_transcript(&messages);
        assert!(transcript.contains("[user] What is Rust?"));
        assert!(transcript.contains("[assistant] Rust is a systems programming language."));
    }

    #[test]
    fn transcript_includes_tool_use_blocks() {
        let messages = vec![ApiMessage {
            role: "assistant".into(),
            content: vec![ApiContentBlock::ToolUse {
                id: "tool_123".into(),
                name: "MemorySearch".into(),
                input: serde_json::json!({"query": "dark mode"}),
            }],
        }];

        let transcript = messages_to_transcript(&messages);
        assert!(transcript.contains("tool_use: MemorySearch"));
        assert!(transcript.contains("dark mode"));
    }

    #[test]
    fn transcript_truncates_long_tool_results() {
        let long_result = "x".repeat(1000);
        let messages = vec![ApiMessage {
            role: "user".into(),
            content: vec![ApiContentBlock::ToolResult {
                tool_use_id: "123".into(),
                content: serde_json::json!(long_result),
                is_error: None,
                cache_control: None,
                name: None,
            }],
        }];

        let transcript = messages_to_transcript(&messages);
        assert!(transcript.contains("..."));
        assert!(transcript.len() < 1000);
    }

    #[test]
    fn transcript_handles_empty_messages() {
        let messages: Vec<ApiMessage> = vec![];
        let transcript = messages_to_transcript(&messages);
        assert!(transcript.is_empty());
    }

    #[test]
    fn flush_tool_definitions_are_valid() {
        let tools = flush_tool_definitions();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "MemoryWrite");
        assert_eq!(tools[1].name, "MemoryAppendDaily");

        // Verify schemas have required fields
        let write_schema = &tools[0].input_schema;
        assert!(write_schema["properties"]["file"].is_object());
        assert!(write_schema["properties"]["content"].is_object());
        assert!(write_schema["properties"]["append"].is_object());

        let daily_schema = &tools[1].input_schema;
        assert!(daily_schema["properties"]["text"].is_object());
    }

    #[tokio::test]
    async fn execute_flush_tool_calls_memory_write() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        // Pre-write a file
        store.write_file("MEMORY.md", "# Memory\n\nOriginal.").await.unwrap();

        // Simulate LLM response with MemoryWrite tool call (append mode)
        let content = vec![ApiContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "MemoryWrite".into(),
            input: serde_json::json!({
                "file": "MEMORY.md",
                "content": "User prefers dark mode.",
                "append": true
            }),
        }];

        execute_flush_tool_calls(&content, &store, None).await;

        let written = store.read_file("MEMORY.md").unwrap();
        assert!(written.contains("Original"), "Original content should be preserved");
        assert!(written.contains("dark mode"), "Appended content should be present");
    }

    #[tokio::test]
    async fn execute_flush_tool_calls_memory_append_daily() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        let content = vec![ApiContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "MemoryAppendDaily".into(),
            input: serde_json::json!({
                "text": "Discussed project architecture."
            }),
        }];

        execute_flush_tool_calls(&content, &store, None).await;

        // Verify daily log was written
        let results = store.search("project architecture", 5).await.unwrap();
        assert!(!results.is_empty(), "Daily log entry should be searchable");
    }

    #[tokio::test]
    async fn execute_flush_tool_calls_multiple() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        // Simulate LLM response with multiple tool calls
        let content = vec![
            ApiContentBlock::Text {
                text: "I'll save important information.".into(),
                cache_control: None,
            },
            ApiContentBlock::ToolUse {
                id: "tool_1".into(),
                name: "MemoryWrite".into(),
                input: serde_json::json!({
                    "file": "MEMORY.md",
                    "content": "# Memory\n\nUser is a backend engineer."
                }),
            },
            ApiContentBlock::ToolUse {
                id: "tool_2".into(),
                name: "MemoryAppendDaily".into(),
                input: serde_json::json!({
                    "text": "User mentioned they prefer Vim."
                }),
            },
        ];

        execute_flush_tool_calls(&content, &store, None).await;

        let memory = store.read_file("MEMORY.md").unwrap();
        assert!(memory.contains("backend engineer"));

        let results = store.search("prefer Vim", 5).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn execute_flush_tool_calls_ignores_unknown_tools() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        let content = vec![ApiContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "UnknownTool".into(),
            input: serde_json::json!({"key": "value"}),
        }];

        // Should not panic
        execute_flush_tool_calls(&content, &store, None).await;
    }

    #[tokio::test]
    async fn execute_flush_tool_calls_handles_missing_fields() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        // MemoryWrite with missing "content" field — should be skipped
        let content = vec![ApiContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "MemoryWrite".into(),
            input: serde_json::json!({"file": "MEMORY.md"}),
        }];

        // Should not panic
        execute_flush_tool_calls(&content, &store, None).await;
    }
}
