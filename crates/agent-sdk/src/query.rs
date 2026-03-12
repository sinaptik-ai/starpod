//! The query function and agent loop implementation.
//!
//! This module contains the core `query()` function that creates an async stream
//! of messages, driving Claude through the agentic loop of prompt → response →
//! tool calls → tool results → repeat.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use futures::Stream;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::client::{
    ApiClient, ApiContentBlock, ApiMessage, CacheControl, CreateMessageRequest, SystemBlock,
    ToolDefinition,
};
use crate::error::{AgentError, Result};
use crate::hooks::{HookCallbackMatcher, HookEvent, HookInput};
use crate::hooks::input::BaseHookInput;
use crate::options::{Options, PermissionMode};
use crate::permissions::{PermissionEvaluator, PermissionVerdict};
use crate::session::Session;
use crate::tools::definitions::get_tool_definitions;
use crate::tools::executor::{ToolExecutor, ToolResult};
use crate::types::messages::*;

/// Default model to use when none is specified.
const DEFAULT_MODEL: &str = "claude-haiku-4-5";
/// Default max tokens for API responses.
const DEFAULT_MAX_TOKENS: u32 = 16384;

/// A handle to a running query that streams messages.
///
/// Implements `Stream<Item = Result<Message>>` for async iteration.
pub struct Query {
    receiver: UnboundedReceiverStream<Result<Message>>,
    session_id: Option<String>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl Query {
    /// Interrupt the current query.
    pub async fn interrupt(&self) -> Result<()> {
        self.cancel_token.cancel();
        Ok(())
    }

    /// Get the session ID (available after the init message).
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Change the permission mode mid-session.
    pub async fn set_permission_mode(&self, _mode: PermissionMode) -> Result<()> {
        // TODO: Send control message to the running agent loop
        Ok(())
    }

    /// Change the model mid-session.
    pub async fn set_model(&self, _model: &str) -> Result<()> {
        // TODO: Send control message to the running agent loop
        Ok(())
    }

    /// Close the query and terminate the underlying process.
    pub fn close(&self) {
        self.cancel_token.cancel();
    }
}

impl Stream for Query {
    type Item = Result<Message>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}

/// Create a query that streams messages from Claude.
///
/// This is the primary function for interacting with the Claude Agent SDK.
/// Returns a [`Query`] stream that yields [`Message`] items as the agent loop
/// progresses.
///
/// # Arguments
///
/// * `prompt` - The input prompt string
/// * `options` - Configuration options for the query
///
/// # Example
///
/// ```rust,no_run
/// use agent_sdk::{query, Options, Message};
/// use tokio_stream::StreamExt;
///
/// # async fn example() -> anyhow::Result<()> {
/// let mut stream = query(
///     "What files are in this directory?",
///     Options::builder()
///         .allowed_tools(vec!["Bash".into(), "Glob".into()])
///         .build(),
/// );
///
/// while let Some(message) = stream.next().await {
///     let message = message?;
///     if let Message::Result(result) = &message {
///         println!("{}", result.result.as_deref().unwrap_or(""));
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub fn query(prompt: &str, options: Options) -> Query {
    let (tx, rx) = mpsc::unbounded_channel();
    let cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = cancel_token.clone();

    let prompt = prompt.to_string();

    tokio::spawn(async move {
        let result = run_agent_loop(prompt, options, tx.clone(), cancel).await;
        if let Err(e) = result {
            let _ = tx.send(Err(e));
        }
    });

    Query {
        receiver: UnboundedReceiverStream::new(rx),
        session_id: None,
        cancel_token,
    }
}

/// The main agent loop.
///
/// This implements the core cycle:
/// 1. Receive prompt
/// 2. Send to Claude
/// 3. Process response (text + tool calls)
/// 4. Execute tools
/// 5. Feed results back
/// 6. Repeat until done or limits hit
async fn run_agent_loop(
    prompt: String,
    options: Options,
    tx: mpsc::UnboundedSender<Result<Message>>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let start_time = Instant::now();
    let mut api_time_ms: u64 = 0;

    // Resolve working directory
    let cwd = options
        .cwd
        .clone()
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string()
        });

    // Create or resume session
    let session = if let Some(ref resume_id) = options.resume {
        Session::with_id(resume_id, &cwd)
    } else if options.continue_session {
        // Find most recent session
        match crate::session::find_most_recent_session(Some(&cwd)).await? {
            Some(info) => Session::with_id(&info.session_id, &cwd),
            None => Session::new(&cwd),
        }
    } else {
        match &options.session_id {
            Some(id) => Session::with_id(id, &cwd),
            None => Session::new(&cwd),
        }
    };

    let session_id = session.id.clone();
    let model = options
        .model
        .clone()
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    // Build tool definitions
    let tool_names: Vec<String> = if options.allowed_tools.is_empty() {
        // Default set of tools
        vec![
            "Read".into(), "Write".into(), "Edit".into(), "Bash".into(),
            "Glob".into(), "Grep".into(),
        ]
    } else {
        options.allowed_tools.clone()
    };

    let raw_defs: Vec<_> = get_tool_definitions(&tool_names);
    let num_tools = raw_defs.len();
    let tool_defs: Vec<ToolDefinition> = raw_defs
        .into_iter()
        .enumerate()
        .map(|(i, td)| ToolDefinition {
            name: td.name.to_string(),
            description: td.description.to_string(),
            input_schema: td.input_schema,
            // Mark the last tool with cache_control so the tools block is cached
            cache_control: if i == num_tools - 1 {
                Some(CacheControl::ephemeral())
            } else {
                None
            },
        })
        .collect();

    // Emit init system message
    let init_msg = Message::System(SystemMessage {
        subtype: SystemSubtype::Init,
        uuid: Uuid::new_v4(),
        session_id: session_id.clone(),
        agents: if options.agents.is_empty() {
            None
        } else {
            Some(options.agents.keys().cloned().collect())
        },
        claude_code_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        cwd: Some(cwd.clone()),
        tools: Some(tool_names.clone()),
        mcp_servers: if options.mcp_servers.is_empty() {
            None
        } else {
            Some(
                options
                    .mcp_servers
                    .keys()
                    .map(|name| McpServerStatus {
                        name: name.clone(),
                        status: "connected".to_string(),
                    })
                    .collect(),
            )
        },
        model: Some(model.clone()),
        permission_mode: Some(options.permission_mode.to_string()),
        compact_metadata: None,
    });

    // Persist and emit init message
    if options.persist_session {
        let _ = session.append_message(&serde_json::to_value(&init_msg).unwrap_or_default()).await;
    }
    if tx.send(Ok(init_msg)).is_err() {
        return Ok(());
    }

    // Initialize API client
    let api_client = ApiClient::new()?;

    // Initialize tool executor
    let tool_executor = ToolExecutor::new(PathBuf::from(&cwd));

    // Initialize permission evaluator
    let permission_eval = PermissionEvaluator::new(&options);

    // Build the system prompt as SystemBlock(s) with prompt caching
    let system_prompt: Option<Vec<SystemBlock>> = {
        let text = match &options.system_prompt {
            Some(crate::options::SystemPrompt::Custom(s)) => s.clone(),
            Some(crate::options::SystemPrompt::Preset { append, .. }) => {
                let base = "You are Claude, an AI assistant. You have access to tools to help accomplish tasks.";
                match append {
                    Some(extra) => format!("{}\n\n{}", base, extra),
                    None => base.to_string(),
                }
            }
            None => "You are Claude, an AI assistant. You have access to tools to help accomplish tasks.".to_string(),
        };
        Some(vec![SystemBlock {
            kind: "text".to_string(),
            text,
            cache_control: Some(CacheControl::ephemeral()),
        }])
    };

    // Build initial conversation from prompt
    let mut conversation: Vec<ApiMessage> = Vec::new();

    // Load previous messages if resuming
    if options.resume.is_some() || options.continue_session {
        let prev_messages = session.load_messages().await?;
        for msg_value in prev_messages {
            if let Some(api_msg) = value_to_api_message(&msg_value) {
                conversation.push(api_msg);
            }
        }
    }

    // Add the user prompt
    conversation.push(ApiMessage {
        role: "user".to_string(),
        content: vec![ApiContentBlock::Text {
            text: prompt.clone(),
            cache_control: None,
        }],
    });

    // Persist user message
    if options.persist_session {
        let user_msg = json!({
            "type": "user",
            "uuid": Uuid::new_v4().to_string(),
            "session_id": &session_id,
            "content": [{"type": "text", "text": &prompt}]
        });
        let _ = session.append_message(&user_msg).await;
    }

    // Agent loop
    let mut num_turns: u32 = 0;
    let mut total_usage = Usage::default();
    let mut total_cost: f64 = 0.0;
    let mut model_usage: HashMap<String, ModelUsage> = HashMap::new();
    let mut permission_denials: Vec<PermissionDenial> = Vec::new();

    loop {
        // Check cancellation
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }

        // Check turn limit
        if let Some(max_turns) = options.max_turns {
            if num_turns >= max_turns {
                let result_msg = build_result_message(
                    ResultSubtype::ErrorMaxTurns,
                    &session_id,
                    None,
                    start_time,
                    api_time_ms,
                    num_turns,
                    total_cost,
                    &total_usage,
                    &model_usage,
                    &permission_denials,
                );
                let _ = tx.send(Ok(result_msg));
                return Ok(());
            }
        }

        // Check budget limit
        if let Some(max_budget) = options.max_budget_usd {
            if total_cost >= max_budget {
                let result_msg = build_result_message(
                    ResultSubtype::ErrorMaxBudgetUsd,
                    &session_id,
                    None,
                    start_time,
                    api_time_ms,
                    num_turns,
                    total_cost,
                    &total_usage,
                    &model_usage,
                    &permission_denials,
                );
                let _ = tx.send(Ok(result_msg));
                return Ok(());
            }
        }

        // Set a cache breakpoint on the last content block of the last user
        // message. This keeps the total breakpoints at 3 (system + tools + last
        // user turn), well within the API limit of 4.
        apply_cache_breakpoint(&mut conversation);

        // Build the API request
        let request = CreateMessageRequest {
            model: model.clone(),
            max_tokens: DEFAULT_MAX_TOKENS,
            messages: conversation.clone(),
            system: system_prompt.clone(),
            tools: if tool_defs.is_empty() {
                None
            } else {
                Some(tool_defs.clone())
            },
            stream: false,
            metadata: None,
            thinking: None,
        };

        // Call Claude
        let api_start = Instant::now();
        let response = match api_client.create_message(&request).await {
            Ok(resp) => resp,
            Err(e) => {
                error!("API call failed: {}", e);
                let result_msg = build_error_result_message(
                    &session_id,
                    &format!("API error: {}", e),
                    start_time,
                    api_time_ms,
                    num_turns,
                    total_cost,
                    &total_usage,
                    &model_usage,
                    &permission_denials,
                );
                let _ = tx.send(Ok(result_msg));
                return Ok(());
            }
        };
        api_time_ms += api_start.elapsed().as_millis() as u64;

        // Update usage
        total_usage.input_tokens += response.usage.input_tokens;
        total_usage.output_tokens += response.usage.output_tokens;
        total_usage.cache_creation_input_tokens +=
            response.usage.cache_creation_input_tokens.unwrap_or(0);
        total_usage.cache_read_input_tokens +=
            response.usage.cache_read_input_tokens.unwrap_or(0);

        // Estimate cost (rough: $3/M input, $15/M output for Sonnet)
        let turn_cost = (response.usage.input_tokens as f64 * 3.0
            + response.usage.output_tokens as f64 * 15.0)
            / 1_000_000.0;
        total_cost += turn_cost;

        // Update model usage
        let model_entry = model_usage
            .entry(model.clone())
            .or_insert_with(ModelUsage::default);
        model_entry.input_tokens += response.usage.input_tokens;
        model_entry.output_tokens += response.usage.output_tokens;
        model_entry.cost_usd += turn_cost;

        // Convert response to our message types
        let content_blocks: Vec<ContentBlock> = response
            .content
            .iter()
            .map(api_block_to_content_block)
            .collect();

        // Emit assistant message
        let assistant_msg = Message::Assistant(AssistantMessage {
            uuid: Uuid::new_v4(),
            session_id: session_id.clone(),
            content: content_blocks.clone(),
            model: response.model.clone(),
            stop_reason: response.stop_reason.clone(),
            parent_tool_use_id: None,
            usage: Some(Usage {
                input_tokens: response.usage.input_tokens,
                output_tokens: response.usage.output_tokens,
                cache_creation_input_tokens: response
                    .usage
                    .cache_creation_input_tokens
                    .unwrap_or(0),
                cache_read_input_tokens: response.usage.cache_read_input_tokens.unwrap_or(0),
            }),
            error: None,
        });

        if options.persist_session {
            let _ = session
                .append_message(&serde_json::to_value(&assistant_msg).unwrap_or_default())
                .await;
        }
        if tx.send(Ok(assistant_msg)).is_err() {
            return Ok(());
        }

        // Add assistant response to conversation
        conversation.push(ApiMessage {
            role: "assistant".to_string(),
            content: response.content.clone(),
        });

        // Check if there are tool calls
        let tool_uses: Vec<_> = response
            .content
            .iter()
            .filter_map(|block| match block {
                ApiContentBlock::ToolUse { id, name, input } => {
                    Some((id.clone(), name.clone(), input.clone()))
                }
                _ => None,
            })
            .collect();

        // If no tool calls, we're done
        if tool_uses.is_empty() {
            // Extract final text
            let final_text = response
                .content
                .iter()
                .filter_map(|block| match block {
                    ApiContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            let result_msg = build_result_message(
                ResultSubtype::Success,
                &session_id,
                Some(final_text),
                start_time,
                api_time_ms,
                num_turns,
                total_cost,
                &total_usage,
                &model_usage,
                &permission_denials,
            );

            if options.persist_session {
                let _ = session
                    .append_message(&serde_json::to_value(&result_msg).unwrap_or_default())
                    .await;
            }
            let _ = tx.send(Ok(result_msg));
            return Ok(());
        }

        // Execute tool calls
        num_turns += 1;
        let mut tool_results: Vec<ApiContentBlock> = Vec::new();

        for (tool_use_id, tool_name, tool_input) in &tool_uses {
            // Check permissions
            let verdict = permission_eval
                .evaluate(tool_name, tool_input, tool_use_id, &session_id, &cwd)
                .await?;

            let actual_input = match &verdict {
                PermissionVerdict::AllowWithUpdatedInput(new_input) => new_input.clone(),
                _ => tool_input.clone(),
            };

            match verdict {
                PermissionVerdict::Allow | PermissionVerdict::AllowWithUpdatedInput(_) => {
                    // Execute the tool
                    debug!(tool = %tool_name, "Executing tool");
                    let result = tool_executor
                        .execute(tool_name, actual_input.clone())
                        .await;

                    let tool_result = match result {
                        Ok(tr) => tr,
                        Err(e) => ToolResult {
                            content: format!("Tool execution error: {}", e),
                            is_error: true,
                        },
                    };

                    // Run PostToolUse hooks
                    if let Some(matchers) = options.hooks.get(&HookEvent::PostToolUse) {
                        run_post_tool_use_hooks(
                            matchers,
                            tool_name,
                            &actual_input,
                            &serde_json::to_value(&tool_result.content).unwrap_or_default(),
                            tool_use_id,
                            &session_id,
                            &cwd,
                        )
                        .await;
                    }

                    tool_results.push(ApiContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: json!(tool_result.content),
                        is_error: if tool_result.is_error {
                            Some(true)
                        } else {
                            None
                        },
                        cache_control: None, // set below on the last result
                    });
                }
                PermissionVerdict::Deny { reason } => {
                    debug!(tool = %tool_name, reason = %reason, "Tool denied");
                    permission_denials.push(PermissionDenial {
                        tool_name: tool_name.clone(),
                        tool_use_id: tool_use_id.clone(),
                        tool_input: tool_input.clone(),
                    });

                    tool_results.push(ApiContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: json!(format!("Permission denied: {}", reason)),
                        is_error: Some(true),
                        cache_control: None, // set below on the last result
                    });
                }
            }
        }

        // Emit user message with tool results
        let user_result_blocks: Vec<ContentBlock> = tool_results
            .iter()
            .map(api_block_to_content_block)
            .collect();

        let user_msg = Message::User(UserMessage {
            uuid: Some(Uuid::new_v4()),
            session_id: session_id.clone(),
            content: user_result_blocks,
            parent_tool_use_id: None,
            is_synthetic: true,
            tool_use_result: None,
        });

        if options.persist_session {
            let _ = session
                .append_message(&serde_json::to_value(&user_msg).unwrap_or_default())
                .await;
        }
        if tx.send(Ok(user_msg)).is_err() {
            return Ok(());
        }

        // Add tool results to conversation
        conversation.push(ApiMessage {
            role: "user".to_string(),
            content: tool_results,
        });
    }
}

/// Run PostToolUse hooks (fire-and-forget for async hooks).
async fn run_post_tool_use_hooks(
    matchers: &[HookCallbackMatcher],
    tool_name: &str,
    tool_input: &serde_json::Value,
    tool_response: &serde_json::Value,
    tool_use_id: &str,
    session_id: &str,
    cwd: &str,
) {
    for matcher in matchers {
        if !matcher.matches(tool_name).unwrap_or(false) {
            continue;
        }

        let input = HookInput::PostToolUse {
            base: BaseHookInput {
                session_id: session_id.to_string(),
                transcript_path: String::new(),
                cwd: cwd.to_string(),
                permission_mode: None,
                agent_id: None,
                agent_type: None,
            },
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
            tool_response: tool_response.clone(),
            tool_use_id: tool_use_id.to_string(),
        };

        let cancel = tokio_util::sync::CancellationToken::new();
        for hook in &matcher.hooks {
            if let Err(e) = hook(input.clone(), Some(tool_use_id.to_string()), cancel.clone()).await
            {
                warn!("PostToolUse hook error: {}", e);
            }
        }
    }
}

/// Apply a single cache breakpoint to the last content block of the last user
/// message in the conversation. Clears any previous breakpoints from messages
/// so we stay within the API limit of 4 cache_control blocks (system + tools +
/// this one = 3 total).
fn apply_cache_breakpoint(conversation: &mut [ApiMessage]) {
    // First, clear all existing cache_control from messages
    for msg in conversation.iter_mut() {
        for block in msg.content.iter_mut() {
            match block {
                ApiContentBlock::Text { cache_control, .. }
                | ApiContentBlock::ToolResult { cache_control, .. } => {
                    *cache_control = None;
                }
                _ => {}
            }
        }
    }

    // Set cache_control on the last content block of the last user message
    if let Some(last_user) = conversation.iter_mut().rev().find(|m| m.role == "user") {
        if let Some(last_block) = last_user.content.last_mut() {
            match last_block {
                ApiContentBlock::Text { cache_control, .. }
                | ApiContentBlock::ToolResult { cache_control, .. } => {
                    *cache_control = Some(CacheControl::ephemeral());
                }
                _ => {}
            }
        }
    }
}

/// Convert an API content block to our ContentBlock type.
fn api_block_to_content_block(block: &ApiContentBlock) -> ContentBlock {
    match block {
        ApiContentBlock::Text { text, .. } => ContentBlock::Text {
            text: text.clone(),
        },
        ApiContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ApiContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            ..
        } => ContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        ApiContentBlock::Thinking { thinking } => ContentBlock::Thinking {
            thinking: thinking.clone(),
        },
    }
}

/// Try to convert a stored JSON value to an API message.
fn value_to_api_message(value: &serde_json::Value) -> Option<ApiMessage> {
    let msg_type = value.get("type")?.as_str()?;

    match msg_type {
        "assistant" => {
            let content = value.get("content")?;
            let blocks = parse_content_blocks(content)?;
            Some(ApiMessage {
                role: "assistant".to_string(),
                content: blocks,
            })
        }
        "user" => {
            let content = value.get("content")?;
            let blocks = parse_content_blocks(content)?;
            Some(ApiMessage {
                role: "user".to_string(),
                content: blocks,
            })
        }
        _ => None,
    }
}

/// Parse content blocks from a JSON value.
fn parse_content_blocks(content: &serde_json::Value) -> Option<Vec<ApiContentBlock>> {
    if let Some(text) = content.as_str() {
        return Some(vec![ApiContentBlock::Text {
            text: text.to_string(),
            cache_control: None,
        }]);
    }

    if let Some(blocks) = content.as_array() {
        let parsed: Vec<ApiContentBlock> = blocks
            .iter()
            .filter_map(|b| serde_json::from_value(b.clone()).ok())
            .collect();
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }

    None
}

/// Build a ResultMessage.
fn build_result_message(
    subtype: ResultSubtype,
    session_id: &str,
    result_text: Option<String>,
    start_time: Instant,
    api_time_ms: u64,
    num_turns: u32,
    total_cost: f64,
    usage: &Usage,
    model_usage: &HashMap<String, ModelUsage>,
    permission_denials: &[PermissionDenial],
) -> Message {
    Message::Result(ResultMessage {
        subtype,
        uuid: Uuid::new_v4(),
        session_id: session_id.to_string(),
        duration_ms: start_time.elapsed().as_millis() as u64,
        duration_api_ms: api_time_ms,
        is_error: result_text.is_none(),
        num_turns,
        result: result_text,
        stop_reason: Some("end_turn".to_string()),
        total_cost_usd: total_cost,
        usage: Some(usage.clone()),
        model_usage: model_usage.clone(),
        permission_denials: permission_denials.to_vec(),
        structured_output: None,
        errors: Vec::new(),
    })
}

/// Build an error ResultMessage.
fn build_error_result_message(
    session_id: &str,
    error_msg: &str,
    start_time: Instant,
    api_time_ms: u64,
    num_turns: u32,
    total_cost: f64,
    usage: &Usage,
    model_usage: &HashMap<String, ModelUsage>,
    permission_denials: &[PermissionDenial],
) -> Message {
    Message::Result(ResultMessage {
        subtype: ResultSubtype::ErrorDuringExecution,
        uuid: Uuid::new_v4(),
        session_id: session_id.to_string(),
        duration_ms: start_time.elapsed().as_millis() as u64,
        duration_api_ms: api_time_ms,
        is_error: true,
        num_turns,
        result: None,
        stop_reason: None,
        total_cost_usd: total_cost,
        usage: Some(usage.clone()),
        model_usage: model_usage.clone(),
        permission_denials: permission_denials.to_vec(),
        structured_output: None,
        errors: vec![error_msg.to_string()],
    })
}
