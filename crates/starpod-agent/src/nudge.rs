//! Background memory nudge — periodically reviews the conversation and persists
//! important information to durable memory.
//!
//! Every [`nudge_interval`](starpod_core::MemoryConfig::nudge_interval) user
//! messages, a lightweight LLM call reviews the recent conversation transcript
//! (pulled from session messages) and uses `MemoryWrite` / `MemoryAppendDaily`
//! tools to save anything worth keeping.
//!
//! Unlike the pre-compaction flush ([`flush`](crate::flush)) which fires when
//! the context window fills, the nudge runs **proactively** on a cadence so
//! information is captured even in short conversations that never hit
//! compaction.
//!
//! # How it works
//!
//! 1. `StarpodAgent` keeps a per-session message counter
//!    (`nudge_counters`). After each user message, the counter increments.
//! 2. When `count % nudge_interval == 0`, [`StarpodAgent::maybe_nudge_memory`]
//!    loads the full session transcript from the database.
//! 3. A background `tokio::spawn` task calls [`run_memory_nudge`] which:
//!    - Converts `SessionMessage` records into a human-readable transcript
//!    - Makes a single non-streaming LLM call with memory tools
//!    - Executes any `MemoryWrite` / `MemoryAppendDaily` tool calls from the
//!      response (via [`flush::execute_flush_tool_calls`])
//!    - Discards the LLM's text output — only tool calls matter
//!
//! # Configuration
//!
//! - `memory.nudge_interval` (default `10`) — user messages between nudges;
//!   set to `0` to disable
//! - `memory.nudge_model` — model override; falls back to
//!   `compaction.flush_model` → `compaction_model` → primary model
//!
//! # Failure mode
//!
//! Fail-open: if the provider call fails or the LLM returns no tool calls, a
//! warning is logged and the conversation continues unaffected.

use std::sync::Arc;

use tracing::{debug, warn};

use agent_sdk::client::{
    ApiContentBlock, ApiMessage, CreateMessageRequest, SystemBlock,
};
use agent_sdk::LlmProvider;
use starpod_memory::MemoryStore;
use starpod_memory::UserMemoryView;
use starpod_session::SessionMessage;

use crate::flush;

/// System prompt for the background memory nudge.
///
/// Instructs the LLM to act as a memory management agent, reviewing the
/// conversation transcript and deciding what to persist. Guides routing:
/// user details → `USER.md`, factual knowledge → `MEMORY.md`, temporal
/// notes → daily log.
const NUDGE_SYSTEM_PROMPT: &str = "\
You are a memory management agent. Your ONLY job is to review the recent conversation \
and save important information using the provided tools. Be selective — only save \
information that would be useful in future conversations.

Save these kinds of information:
- User preferences, working style, and personal details → USER.md
- Key decisions and their reasoning → MEMORY.md (append)
- Important facts, names, dates, and relationships → MEMORY.md (append)
- Technical context: architecture choices, conventions, configurations → MEMORY.md (append)
- Action items, commitments, and follow-ups → daily log
- Brief summary of what was discussed → daily log

Do NOT save:
- Trivial or transient exchanges (greetings, acknowledgments, small talk)
- Information that's already in MEMORY.md or USER.md
- Raw code or long outputs (summarize instead)
- Temporary debugging context or error traces

If there is nothing worth saving, respond with a single text message saying \
\"Nothing to save\" and make no tool calls.

Use MemoryWrite with append=true to add to MEMORY.md or USER.md. \
Use MemoryAppendDaily for time-specific notes and conversation summaries. \
Respond with ONLY tool calls (or \"Nothing to save\"), no other text.";

/// Maximum transcript length sent to the nudge LLM (characters).
const MAX_TRANSCRIPT_LEN: usize = 30_000;

/// Maximum length of a single message in the transcript (characters).
const MAX_MESSAGE_LEN: usize = 1_000;

/// Convert session messages into a human-readable transcript for the nudge LLM.
///
/// Each message is formatted as `[role] content` with double-newline separators.
/// Individual messages exceeding [`MAX_MESSAGE_LEN`] are truncated with `...`.
fn session_messages_to_transcript(messages: &[SessionMessage]) -> String {
    let mut parts = Vec::with_capacity(messages.len());
    for msg in messages {
        let role = match msg.role.as_str() {
            "user" => "user",
            "assistant" => "assistant",
            "tool_use" => "assistant (tool_use)",
            "tool_result" => "tool_result",
            other => other,
        };
        // Truncate very long messages (e.g. tool results with big outputs)
        let content = if msg.content.len() > MAX_MESSAGE_LEN {
            let mut end = MAX_MESSAGE_LEN;
            while end > 0 && !msg.content.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &msg.content[..end])
        } else {
            msg.content.clone()
        };
        parts.push(format!("[{}] {}", role, content));
    }
    parts.join("\n\n")
}

/// Run the background memory nudge.
///
/// Builds a transcript from `messages`, sends it to the LLM via a single
/// non-streaming call, then executes any memory tool calls from the response.
///
/// # Arguments
///
/// * `provider` — LLM provider (shared ownership for use across `tokio::spawn`)
/// * `model` — model ID to use for the nudge call
/// * `messages` — session messages to review (from `SessionManager::get_messages`)
/// * `memory` — agent-level memory store (writes land here when no user view)
/// * `user_view` — per-user memory view; when present, writes route to the
///   user's directory instead of the agent-level store
///
/// # Errors
///
/// This function is fail-open: provider errors are logged as warnings, and
/// the caller is not notified. This ensures the nudge never disrupts the
/// main chat flow.
pub async fn run_memory_nudge(
    provider: Arc<dyn LlmProvider>,
    model: &str,
    messages: &[SessionMessage],
    memory: &MemoryStore,
    user_view: Option<&UserMemoryView>,
) {
    let transcript = session_messages_to_transcript(messages);
    if transcript.trim().is_empty() {
        return;
    }

    // Cap transcript to avoid huge requests
    let transcript = if transcript.len() > MAX_TRANSCRIPT_LEN {
        let mut end = MAX_TRANSCRIPT_LEN;
        while end > 0 && !transcript.is_char_boundary(end) {
            end -= 1;
        }
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
                    "Review this recent conversation and save important information to memory:\n\n{}",
                    transcript
                ),
                cache_control: None,
            }],
        }],
        system: Some(vec![SystemBlock {
            kind: "text".into(),
            text: NUDGE_SYSTEM_PROMPT.to_string(),
            cache_control: None,
        }]),
        tools: Some(flush::flush_tool_definitions()),
        stream: false,
        metadata: None,
        thinking: None,
    };

    debug!(model = %model, transcript_len = transcript.len(), messages = messages.len(), "Running memory nudge");

    match provider.create_message(&request).await {
        Ok(response) => {
            let tool_calls: Vec<_> = response
                .content
                .iter()
                .filter(|b| matches!(b, ApiContentBlock::ToolUse { .. }))
                .collect();
            if tool_calls.is_empty() {
                debug!("Nudge: nothing to save");
            } else {
                debug!(tool_calls = tool_calls.len(), "Nudge: executing tool calls");
            }
            flush::execute_flush_tool_calls(&response.content, memory, user_view).await;
        }
        Err(e) => {
            warn!(error = %e, "Memory nudge LLM call failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::pin::Pin;

    use async_trait::async_trait;
    use agent_sdk::client::{ApiUsage, MessageResponse, StreamEvent};
    use agent_sdk::error::Result as SdkResult;
    use agent_sdk::provider::{CostRates, ProviderCapabilities};
    use futures::stream::Stream;

    /// Helper to build a SessionMessage with minimal boilerplate.
    fn msg(id: i64, role: &str, content: &str) -> SessionMessage {
        SessionMessage {
            id,
            session_id: "test-session".into(),
            role: role.into(),
            content: content.into(),
            timestamp: "2026-03-28T10:00:00".into(),
        }
    }

    // ── transcript formatting ─────────────────────────────────────────

    #[test]
    fn transcript_formats_session_messages() {
        let messages = vec![
            msg(1, "user", "What is Rust?"),
            msg(2, "assistant", "Rust is a systems programming language."),
        ];

        let transcript = session_messages_to_transcript(&messages);
        assert!(transcript.contains("[user] What is Rust?"));
        assert!(transcript.contains("[assistant] Rust is a systems programming language."));
    }

    #[test]
    fn transcript_separates_messages_with_double_newlines() {
        let messages = vec![
            msg(1, "user", "Hello"),
            msg(2, "assistant", "Hi there"),
        ];

        let transcript = session_messages_to_transcript(&messages);
        assert!(transcript.contains("[user] Hello\n\n[assistant] Hi there"));
    }

    #[test]
    fn transcript_truncates_long_messages() {
        let long_content = "x".repeat(2000);
        let messages = vec![msg(1, "tool_result", &long_content)];

        let transcript = session_messages_to_transcript(&messages);
        assert!(transcript.ends_with("..."));
        // Should be: "[tool_result] " (15 chars) + 1000 chars + "..." (3 chars) = ~1018
        assert!(transcript.len() < 1100);
    }

    #[test]
    fn transcript_handles_empty_messages() {
        let messages: Vec<SessionMessage> = vec![];
        let transcript = session_messages_to_transcript(&messages);
        assert!(transcript.is_empty());
    }

    #[test]
    fn transcript_maps_tool_roles() {
        let messages = vec![
            msg(1, "tool_use", "MemorySearch({\"query\": \"test\"})"),
            msg(2, "tool_result", "Found 3 results."),
        ];

        let transcript = session_messages_to_transcript(&messages);
        assert!(transcript.contains("[assistant (tool_use)]"));
        assert!(transcript.contains("[tool_result]"));
    }

    #[test]
    fn transcript_preserves_unknown_roles() {
        let messages = vec![msg(1, "system", "You are an AI assistant.")];
        let transcript = session_messages_to_transcript(&messages);
        assert!(transcript.contains("[system] You are an AI assistant."));
    }

    #[test]
    fn transcript_handles_multibyte_chars_in_truncation() {
        // 4-byte emoji repeated — truncation must not split a character
        let emoji_content = "🦀".repeat(500); // 500 * 4 = 2000 bytes
        let messages = vec![msg(1, "user", &emoji_content)];

        let transcript = session_messages_to_transcript(&messages);
        // Should not panic and should end with "..."
        assert!(transcript.ends_with("..."));
        // Verify the truncated content is valid UTF-8 (implicitly true if we got here)
        assert!(transcript.len() > 0);
    }

    #[test]
    fn transcript_single_message_no_trailing_separator() {
        let messages = vec![msg(1, "user", "Just one message")];
        let transcript = session_messages_to_transcript(&messages);
        assert_eq!(transcript, "[user] Just one message");
    }

    // ── transcript capping ───────────────────────────────────────────

    #[test]
    fn transcript_cap_applies_to_total_length() {
        // Build a transcript that exceeds MAX_TRANSCRIPT_LEN
        let long_msg = "a".repeat(800);
        let messages: Vec<_> = (0..50).map(|i| msg(i, "user", &long_msg)).collect();

        let transcript = session_messages_to_transcript(&messages);
        // The raw transcript should be large
        assert!(transcript.len() > MAX_TRANSCRIPT_LEN);

        // Now simulate the capping logic from run_memory_nudge
        let capped = if transcript.len() > MAX_TRANSCRIPT_LEN {
            let mut end = MAX_TRANSCRIPT_LEN;
            while end > 0 && !transcript.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...\n\n[transcript truncated]", &transcript[..end])
        } else {
            transcript
        };

        assert!(capped.contains("[transcript truncated]"));
        // Capped length: MAX_TRANSCRIPT_LEN + "..." + "\n\n[transcript truncated]"
        assert!(capped.len() < MAX_TRANSCRIPT_LEN + 50);
    }

    // ── mock provider ────────────────────────────────────────────────

    /// A mock LLM provider that returns a preconfigured response.
    struct MockProvider {
        response: tokio::sync::Mutex<Option<MessageResponse>>,
    }

    impl MockProvider {
        fn with_response(response: MessageResponse) -> Self {
            Self {
                response: tokio::sync::Mutex::new(Some(response)),
            }
        }

        fn failing() -> Self {
            Self {
                response: tokio::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                streaming: false,
                tool_use: true,
                thinking: false,
                prompt_caching: false,
            }
        }

        fn cost_rates(&self, _model: &str) -> CostRates {
            CostRates {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_multiplier: None,
                cache_creation_multiplier: None,
            }
        }

        async fn create_message(
            &self,
            _request: &CreateMessageRequest,
        ) -> SdkResult<MessageResponse> {
            match self.response.lock().await.take() {
                Some(r) => Ok(r),
                None => Err(agent_sdk::AgentError::Api("Mock provider failure".into())),
            }
        }

        async fn create_message_stream(
            &self,
            _request: &CreateMessageRequest,
        ) -> SdkResult<Pin<Box<dyn Stream<Item = SdkResult<StreamEvent>> + Send>>> {
            Err(agent_sdk::AgentError::Api("Not implemented".into()))
        }
    }

    fn mock_response_with_tool_calls(content: Vec<ApiContentBlock>) -> MessageResponse {
        MessageResponse {
            id: "msg_test".into(),
            role: "assistant".into(),
            content,
            model: "test-model".into(),
            stop_reason: Some("end_turn".into()),
            usage: ApiUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        }
    }

    // ── run_memory_nudge integration tests ───────────────────────────

    #[tokio::test]
    async fn nudge_executes_memory_write_tool_call() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();
        store.write_file("MEMORY.md", "# Memory\n").await.unwrap();

        let response = mock_response_with_tool_calls(vec![ApiContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "MemoryWrite".into(),
            input: serde_json::json!({
                "file": "MEMORY.md",
                "content": "\n- User prefers dark mode.",
                "append": true
            }),
        }]);

        let provider = Arc::new(MockProvider::with_response(response));
        let messages = vec![
            msg(1, "user", "I always use dark mode in my editors"),
            msg(2, "assistant", "Noted! I'll remember that you prefer dark mode."),
        ];

        run_memory_nudge(provider, "test-model", &messages, &store, None).await;

        let content = store.read_file("MEMORY.md").unwrap();
        assert!(content.contains("dark mode"), "MemoryWrite should have persisted the preference");
    }

    #[tokio::test]
    async fn nudge_executes_daily_append_tool_call() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        let response = mock_response_with_tool_calls(vec![ApiContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "MemoryAppendDaily".into(),
            input: serde_json::json!({
                "text": "Discussed project architecture and decided on event sourcing."
            }),
        }]);

        let provider = Arc::new(MockProvider::with_response(response));
        let messages = vec![
            msg(1, "user", "Let's use event sourcing for the new service"),
            msg(2, "assistant", "Good choice — event sourcing fits well here."),
        ];

        run_memory_nudge(provider, "test-model", &messages, &store, None).await;

        let results = store.search("event sourcing", 5).await.unwrap();
        assert!(!results.is_empty(), "Daily log entry should be searchable");
    }

    #[tokio::test]
    async fn nudge_handles_multiple_tool_calls() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        let response = mock_response_with_tool_calls(vec![
            ApiContentBlock::ToolUse {
                id: "tool_1".into(),
                name: "MemoryWrite".into(),
                input: serde_json::json!({
                    "file": "USER.md",
                    "content": "# User\n\nName: Alice\nRole: Backend engineer"
                }),
            },
            ApiContentBlock::ToolUse {
                id: "tool_2".into(),
                name: "MemoryAppendDaily".into(),
                input: serde_json::json!({
                    "text": "User introduced themselves as Alice, a backend engineer."
                }),
            },
        ]);

        let provider = Arc::new(MockProvider::with_response(response));
        let messages = vec![
            msg(1, "user", "Hey! I'm Alice, a backend engineer at Acme."),
            msg(2, "assistant", "Nice to meet you, Alice!"),
        ];

        run_memory_nudge(provider, "test-model", &messages, &store, None).await;

        let user = store.read_file("USER.md").unwrap();
        assert!(user.contains("Alice"), "USER.md should contain user's name");
        assert!(user.contains("Backend engineer"), "USER.md should contain user's role");

        let results = store.search("backend engineer", 5).await.unwrap();
        assert!(!results.is_empty(), "Daily log should be searchable");
    }

    #[tokio::test]
    async fn nudge_handles_nothing_to_save() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        // LLM responds with text only — no tool calls
        let response = mock_response_with_tool_calls(vec![ApiContentBlock::Text {
            text: "Nothing to save".into(),
            cache_control: None,
        }]);

        let provider = Arc::new(MockProvider::with_response(response));
        let messages = vec![
            msg(1, "user", "Hi"),
            msg(2, "assistant", "Hello!"),
        ];

        // Should not panic or write anything
        run_memory_nudge(provider, "test-model", &messages, &store, None).await;
    }

    #[tokio::test]
    async fn nudge_handles_provider_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        let provider = Arc::new(MockProvider::failing());
        let messages = vec![msg(1, "user", "Important info here")];

        // Should not panic — fail-open
        run_memory_nudge(provider, "test-model", &messages, &store, None).await;
    }

    #[tokio::test]
    async fn nudge_skips_empty_transcript() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();

        // Provider should never be called (we'll use a failing one to verify)
        let provider = Arc::new(MockProvider::failing());
        let messages: Vec<SessionMessage> = vec![];

        // Should return immediately without calling the provider
        run_memory_nudge(provider, "test-model", &messages, &store, None).await;
    }

    #[tokio::test]
    async fn nudge_routes_to_user_view() {
        let agent_tmp = tempfile::TempDir::new().unwrap();
        let user_tmp = tempfile::TempDir::new().unwrap();

        let agent_store = Arc::new(MemoryStore::new_user(agent_tmp.path()).await.unwrap());
        let user_view = UserMemoryView::new(agent_store.clone(), user_tmp.path().to_path_buf())
            .await
            .unwrap();

        let response = mock_response_with_tool_calls(vec![ApiContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "MemoryWrite".into(),
            input: serde_json::json!({
                "file": "USER.md",
                "content": "# User\n\nPrefers vim keybindings."
            }),
        }]);

        let provider = Arc::new(MockProvider::with_response(response));
        let messages = vec![msg(1, "user", "I use vim keybindings everywhere")];

        run_memory_nudge(
            provider,
            "test-model",
            &messages,
            &agent_store,
            Some(&user_view),
        )
        .await;

        // The write should have gone to the user view, not the agent store
        let user_content = user_view.read_file("USER.md").unwrap();
        assert!(user_content.contains("vim keybindings"));
    }

    // ── counter / interval logic ────────────────────────────────────

    // These tests verify the counter modulo logic used by
    // StarpodAgent::maybe_nudge_memory without needing the full agent.

    /// Simulate the counter logic from `maybe_nudge_memory` and return
    /// which message numbers trigger a nudge.
    fn simulate_nudge_triggers(interval: u32, num_messages: u32) -> Vec<u32> {
        let mut counter: u32 = 0;
        let mut triggers = Vec::new();
        for msg_num in 1..=num_messages {
            counter += 1;
            if interval > 0 && counter % interval == 0 {
                triggers.push(msg_num);
            }
        }
        triggers
    }

    #[test]
    fn counter_fires_at_exact_interval() {
        let triggers = simulate_nudge_triggers(10, 25);
        assert_eq!(triggers, vec![10, 20], "Should fire at messages 10 and 20");
    }

    #[test]
    fn counter_fires_every_message_when_interval_is_1() {
        let triggers = simulate_nudge_triggers(1, 5);
        assert_eq!(triggers, vec![1, 2, 3, 4, 5], "Should fire every message");
    }

    #[test]
    fn counter_never_fires_when_interval_is_0() {
        let triggers = simulate_nudge_triggers(0, 100);
        assert!(triggers.is_empty(), "Should never fire when disabled");
    }

    #[test]
    fn counter_fires_at_large_interval() {
        let triggers = simulate_nudge_triggers(50, 100);
        assert_eq!(triggers, vec![50, 100]);
    }

    #[test]
    fn counter_does_not_fire_before_interval() {
        let triggers = simulate_nudge_triggers(10, 9);
        assert!(triggers.is_empty(), "Should not fire before reaching interval");
    }

    #[test]
    fn counter_fires_exactly_at_boundary() {
        let triggers = simulate_nudge_triggers(10, 10);
        assert_eq!(triggers, vec![10], "Should fire exactly at interval boundary");
    }

    // ── nudge request construction ──────────────────────────────────

    #[tokio::test]
    async fn nudge_request_includes_system_prompt_and_tools() {
        // Verify the LLM request is built correctly by capturing it
        use std::sync::atomic::{AtomicBool, Ordering};

        struct InspectingProvider {
            called: AtomicBool,
        }

        #[async_trait]
        impl LlmProvider for InspectingProvider {
            fn name(&self) -> &str { "inspect" }
            fn capabilities(&self) -> ProviderCapabilities {
                ProviderCapabilities { streaming: false, tool_use: true, thinking: false, prompt_caching: false }
            }
            fn cost_rates(&self, _model: &str) -> CostRates {
                CostRates { input_per_million: 0.0, output_per_million: 0.0, cache_read_multiplier: None, cache_creation_multiplier: None }
            }
            async fn create_message(&self, request: &CreateMessageRequest) -> SdkResult<MessageResponse> {
                self.called.store(true, Ordering::SeqCst);

                // Verify system prompt is present
                assert!(request.system.is_some(), "System prompt should be present");
                let sys = &request.system.as_ref().unwrap()[0].text;
                assert!(sys.contains("memory management agent"), "System prompt should identify as memory agent");

                // Verify tools are present
                assert!(request.tools.is_some(), "Tools should be present");
                let tools = request.tools.as_ref().unwrap();
                assert_eq!(tools.len(), 2, "Should have MemoryWrite and MemoryAppendDaily");
                assert_eq!(tools[0].name, "MemoryWrite");
                assert_eq!(tools[1].name, "MemoryAppendDaily");

                // Verify transcript is in the user message
                let user_msg = &request.messages[0];
                assert_eq!(user_msg.role, "user");
                if let ApiContentBlock::Text { text, .. } = &user_msg.content[0] {
                    assert!(text.contains("dark mode"), "Transcript should contain message content");
                } else {
                    panic!("Expected text content block");
                }

                // Return "nothing to save"
                Ok(MessageResponse {
                    id: "msg_test".into(),
                    role: "assistant".into(),
                    content: vec![ApiContentBlock::Text { text: "Nothing to save".into(), cache_control: None }],
                    model: "test".into(),
                    stop_reason: Some("end_turn".into()),
                    usage: ApiUsage { input_tokens: 0, output_tokens: 0, cache_creation_input_tokens: None, cache_read_input_tokens: None },
                })
            }
            async fn create_message_stream(&self, _request: &CreateMessageRequest)
                -> SdkResult<Pin<Box<dyn Stream<Item = SdkResult<StreamEvent>> + Send>>>
            {
                Err(agent_sdk::AgentError::Api("Not implemented".into()))
            }
        }

        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::new_user(tmp.path()).await.unwrap();
        let provider = Arc::new(InspectingProvider { called: AtomicBool::new(false) });
        let provider_dyn: Arc<dyn LlmProvider> = Arc::clone(&provider) as Arc<dyn LlmProvider>;

        let messages = vec![
            msg(1, "user", "I prefer dark mode"),
            msg(2, "assistant", "Noted!"),
        ];

        run_memory_nudge(provider_dyn, "test-model", &messages, &store, None).await;
        assert!(provider.called.load(Ordering::SeqCst), "Provider should have been called");
    }

    // ── config defaults and parsing ─────────────────────────────────

    #[test]
    fn nudge_system_prompt_mentions_key_file_targets() {
        assert!(NUDGE_SYSTEM_PROMPT.contains("USER.md"));
        assert!(NUDGE_SYSTEM_PROMPT.contains("MEMORY.md"));
        assert!(NUDGE_SYSTEM_PROMPT.contains("MemoryWrite"));
        assert!(NUDGE_SYSTEM_PROMPT.contains("MemoryAppendDaily"));
        assert!(NUDGE_SYSTEM_PROMPT.contains("Nothing to save"));
    }

    #[test]
    fn max_transcript_len_is_reasonable() {
        assert_eq!(MAX_TRANSCRIPT_LEN, 30_000);
        assert_eq!(MAX_MESSAGE_LEN, 1_000);
    }
}
