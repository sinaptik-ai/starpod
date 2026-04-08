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

use futures::stream::FuturesUnordered;
use futures::{Stream, StreamExt as FuturesStreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::client::{
    ApiContentBlock, ApiMessage, ApiUsage, CacheControl, ContentDelta, CreateMessageRequest,
    ImageSource, MessageResponse, StreamEvent as ClientStreamEvent, SystemBlock, ThinkingParam,
    ToolDefinition,
};
use crate::compact;
use crate::error::{AgentError, Result};
use crate::hooks::HookRegistry;
use crate::options::{Options, PermissionMode, ThinkingConfig};
use crate::permissions::{PermissionEvaluator, PermissionVerdict};
use crate::provider::LlmProvider;
use crate::providers::AnthropicProvider;
use crate::sanitize;
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
    mut options: Options,
    tx: mpsc::UnboundedSender<Result<Message>>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let start_time = Instant::now();
    let mut api_time_ms: u64 = 0;

    // Resolve working directory
    let cwd = options.cwd.clone().unwrap_or_else(|| {
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

    // Build tool definitions (skip tools entirely when output_format is set —
    // structured-output queries should not use tools).
    let tool_names: Vec<String> = if options.output_format.is_some() {
        Vec::new()
    } else if options.allowed_tools.is_empty() {
        // Default set of tools
        vec![
            "Read".into(),
            "Write".into(),
            "Edit".into(),
            "Bash".into(),
            "Glob".into(),
            "Grep".into(),
        ]
    } else {
        options.allowed_tools.clone()
    };

    let raw_defs: Vec<_> = get_tool_definitions(&tool_names);

    // Combine built-in + custom tool definitions
    let mut all_defs: Vec<ToolDefinition> = raw_defs
        .into_iter()
        .map(|td| ToolDefinition {
            name: td.name.to_string(),
            description: td.description.to_string(),
            input_schema: td.input_schema,
            cache_control: None,
        })
        .collect();

    // Append custom tool definitions
    for ctd in &options.custom_tool_definitions {
        all_defs.push(ToolDefinition {
            name: ctd.name.clone(),
            description: ctd.description.clone(),
            input_schema: ctd.input_schema.clone(),
            cache_control: None,
        });
    }

    // Mark the last tool with cache_control so the tools block is cached
    if let Some(last) = all_defs.last_mut() {
        last.cache_control = Some(CacheControl::ephemeral());
    }

    let tool_defs = all_defs;

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
        let _ = session
            .append_message(&serde_json::to_value(&init_msg).unwrap_or_default())
            .await;
    }
    if tx.send(Ok(init_msg)).is_err() {
        return Ok(());
    }

    // Initialize LLM provider
    let provider: Box<dyn LlmProvider> = match options.provider.take() {
        Some(p) => p,
        None => Box::new(AnthropicProvider::from_env()?),
    };

    // Initialize tool executor with optional path boundary
    let additional_dirs: Vec<PathBuf> = options
        .additional_directories
        .iter()
        .map(PathBuf::from)
        .collect();
    let env_blocklist = std::mem::take(&mut options.env_blocklist);
    let env_inject = std::mem::take(&mut options.env);
    #[cfg(unix)]
    let pre_exec_fn = options.pre_exec_fn.take();
    let mut tool_executor = if additional_dirs.is_empty() {
        ToolExecutor::new(PathBuf::from(&cwd))
    } else {
        ToolExecutor::with_allowed_dirs(PathBuf::from(&cwd), additional_dirs)
    }
    .with_env_blocklist(env_blocklist)
    .with_env_inject(env_inject);
    #[cfg(unix)]
    if let Some(f) = pre_exec_fn {
        tool_executor = tool_executor.with_pre_exec(f);
    }

    // Build hook registry from options, merging file-discovered hooks
    let mut hook_registry = HookRegistry::from_map(std::mem::take(&mut options.hooks));
    if !options.hook_dirs.is_empty() {
        let dirs: Vec<&std::path::Path> = options.hook_dirs.iter().map(|p| p.as_path()).collect();
        match crate::hooks::HookDiscovery::discover(&dirs) {
            Ok(discovered) => hook_registry.merge(discovered),
            Err(e) => tracing::warn!("Failed to discover hooks from dirs: {}", e),
        }
    }

    // Take followup_rx out of options before borrowing options immutably
    let mut followup_rx = options.followup_rx.take();

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

        // Repair orphaned tool_use blocks: if the last assistant message contains
        // tool_use blocks without matching tool_result blocks in the following user
        // message, inject synthetic error tool_results so the API doesn't reject
        // the conversation history.
        repair_orphaned_tool_uses(&mut conversation);
    }

    // Add the user prompt (with optional image attachments)
    {
        let mut content_blocks: Vec<ApiContentBlock> = Vec::new();

        // Add image attachments as Image content blocks
        for att in &options.attachments {
            let is_image = matches!(
                att.mime_type.as_str(),
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            );
            if is_image {
                content_blocks.push(ApiContentBlock::Image {
                    source: ImageSource {
                        kind: "base64".to_string(),
                        media_type: att.mime_type.clone(),
                        data: att.base64_data.clone(),
                    },
                });
            }
        }

        // Add the text prompt
        content_blocks.push(ApiContentBlock::Text {
            text: prompt.clone(),
            cache_control: None,
        });

        conversation.push(ApiMessage {
            role: "user".to_string(),
            content: content_blocks,
        });
    }

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

        // Drain any followup messages that arrived while we were processing.
        // These are batched into a single user message appended to the conversation
        // so the model sees them on the next API call.
        if let Some(ref mut followup_rx) = followup_rx {
            let mut followups: Vec<String> = Vec::new();
            while let Ok(msg) = followup_rx.try_recv() {
                followups.push(msg);
            }
            if !followups.is_empty() {
                let combined = followups.join("\n\n");
                debug!(
                    count = followups.len(),
                    "Injecting followup messages into agent loop"
                );

                conversation.push(ApiMessage {
                    role: "user".to_string(),
                    content: vec![ApiContentBlock::Text {
                        text: combined.clone(),
                        cache_control: None,
                    }],
                });

                // Emit a user message so downstream consumers know about the injection
                let followup_msg = Message::User(UserMessage {
                    uuid: Some(Uuid::new_v4()),
                    session_id: session_id.clone(),
                    content: vec![ContentBlock::Text { text: combined }],
                    parent_tool_use_id: None,
                    is_synthetic: false,
                    tool_use_result: None,
                });

                if options.persist_session {
                    let _ = session
                        .append_message(&serde_json::to_value(&followup_msg).unwrap_or_default())
                        .await;
                }
                if tx.send(Ok(followup_msg)).is_err() {
                    return Ok(());
                }
            }
        }

        // Set a cache breakpoint on the last content block of the last user
        // message. This keeps the total breakpoints at 3 (system + tools + last
        // user turn), well within the API limit of 4.
        apply_cache_breakpoint(&mut conversation);

        // Build thinking param from options
        let thinking_param = options.thinking.as_ref().map(|tc| match tc {
            ThinkingConfig::Adaptive => ThinkingParam {
                kind: "enabled".into(),
                budget_tokens: Some(10240),
            },
            ThinkingConfig::Disabled => ThinkingParam {
                kind: "disabled".into(),
                budget_tokens: None,
            },
            ThinkingConfig::Enabled { budget_tokens } => ThinkingParam {
                kind: "enabled".into(),
                budget_tokens: Some(*budget_tokens),
            },
        });

        // Increase max_tokens when thinking is enabled
        let base_max_tokens = options.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let max_tokens = if let Some(ref tp) = thinking_param {
            if let Some(budget) = tp.budget_tokens {
                base_max_tokens.max(budget as u32 + 8192)
            } else {
                base_max_tokens
            }
        } else {
            base_max_tokens
        };

        // Build the API request
        let use_streaming = options.include_partial_messages;
        let request = CreateMessageRequest {
            model: model.clone(),
            max_tokens,
            messages: conversation.clone(),
            system: system_prompt.clone(),
            tools: if tool_defs.is_empty() {
                None
            } else {
                Some(tool_defs.clone())
            },
            stream: use_streaming,
            metadata: None,
            thinking: thinking_param,
        };

        // Call LLM provider
        let api_start = Instant::now();
        let response = if use_streaming {
            // Streaming mode: consume SSE events, emit text deltas, accumulate full response
            match provider.create_message_stream(&request).await {
                Ok(mut event_stream) => {
                    match accumulate_stream(&mut event_stream, &tx, &session_id).await {
                        Ok(resp) => resp,
                        Err(e) => {
                            error!("Stream accumulation failed: {}", e);
                            let result_msg = build_error_result_message(
                                &session_id,
                                &format!("Stream error: {}", e),
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
                }
                Err(e) => {
                    error!("API stream call failed: {}", e);
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
            }
        } else {
            // Non-streaming mode: single request/response
            match provider.create_message(&request).await {
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
            }
        };
        api_time_ms += api_start.elapsed().as_millis() as u64;

        // Update usage
        total_usage.input_tokens += response.usage.input_tokens;
        total_usage.output_tokens += response.usage.output_tokens;
        total_usage.cache_creation_input_tokens +=
            response.usage.cache_creation_input_tokens.unwrap_or(0);
        total_usage.cache_read_input_tokens += response.usage.cache_read_input_tokens.unwrap_or(0);

        // Estimate cost using provider-specific rates (with cache-aware pricing)
        let rates = provider.cost_rates(&model);
        let turn_cost = rates.compute_with_cache(
            response.usage.input_tokens,
            response.usage.output_tokens,
            response.usage.cache_read_input_tokens.unwrap_or(0),
            response.usage.cache_creation_input_tokens.unwrap_or(0),
        );
        total_cost += turn_cost;

        // Update model usage
        let model_entry = model_usage.entry(model.clone()).or_default();
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

        // Phase 0: Reject hallucinated tool names immediately with a helpful error.
        // Collect known tool names from the definitions we sent to the model.
        let known_tool_names: std::collections::HashSet<&str> =
            tool_defs.iter().map(|td| td.name.as_str()).collect();

        let mut valid_tool_uses: Vec<&(String, String, serde_json::Value)> = Vec::new();
        for tu in &tool_uses {
            let (tool_use_id, tool_name, _tool_input) = tu;
            if known_tool_names.contains(tool_name.as_str()) {
                valid_tool_uses.push(tu);
            } else {
                warn!(tool = %tool_name, "model invoked unknown tool, returning error");
                let available: Vec<&str> = tool_defs.iter().map(|td| td.name.as_str()).collect();
                let error_msg = format!(
                    "Error: '{}' is not a valid tool. You MUST use one of the following tools: {}",
                    tool_name,
                    available.join(", ")
                );
                let api_block = ApiContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: json!(error_msg),
                    is_error: Some(true),
                    cache_control: None,
                    name: Some(tool_name.clone()),
                };

                // Stream the error to the frontend
                let result_msg = Message::User(UserMessage {
                    uuid: Some(Uuid::new_v4()),
                    session_id: session_id.clone(),
                    content: vec![api_block_to_content_block(&api_block)],
                    parent_tool_use_id: None,
                    is_synthetic: true,
                    tool_use_result: None,
                });
                if options.persist_session {
                    let _ = session
                        .append_message(&serde_json::to_value(&result_msg).unwrap_or_default())
                        .await;
                }
                if tx.send(Ok(result_msg)).is_err() {
                    return Ok(());
                }

                tool_results.push(api_block);
            }
        }

        // Phase 1: Evaluate permissions sequentially (may involve user interaction)
        struct PermittedTool {
            tool_use_id: String,
            tool_name: String,
            actual_input: serde_json::Value,
        }
        let mut permitted_tools: Vec<PermittedTool> = Vec::new();

        for (tool_use_id, tool_name, tool_input) in valid_tool_uses.iter().map(|t| &**t) {
            let verdict = permission_eval
                .evaluate(tool_name, tool_input, tool_use_id, &session_id, &cwd)
                .await?;

            let actual_input = match &verdict {
                PermissionVerdict::AllowWithUpdatedInput(new_input) => new_input.clone(),
                _ => tool_input.clone(),
            };

            match verdict {
                PermissionVerdict::Allow | PermissionVerdict::AllowWithUpdatedInput(_) => {
                    permitted_tools.push(PermittedTool {
                        tool_use_id: tool_use_id.clone(),
                        tool_name: tool_name.clone(),
                        actual_input,
                    });
                }
                PermissionVerdict::Deny { reason } => {
                    debug!(tool = %tool_name, reason = %reason, "Tool denied");
                    permission_denials.push(PermissionDenial {
                        tool_name: tool_name.clone(),
                        tool_use_id: tool_use_id.clone(),
                        tool_input: tool_input.clone(),
                    });

                    let api_block = ApiContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: json!(format!("Permission denied: {}", reason)),
                        is_error: Some(true),
                        cache_control: None,
                        name: Some(tool_name.clone()),
                    };

                    // Stream denial result to frontend immediately
                    let denial_msg = Message::User(UserMessage {
                        uuid: Some(Uuid::new_v4()),
                        session_id: session_id.clone(),
                        content: vec![api_block_to_content_block(&api_block)],
                        parent_tool_use_id: None,
                        is_synthetic: true,
                        tool_use_result: None,
                    });
                    if options.persist_session {
                        let _ = session
                            .append_message(&serde_json::to_value(&denial_msg).unwrap_or_default())
                            .await;
                    }
                    if tx.send(Ok(denial_msg)).is_err() {
                        return Ok(());
                    }

                    tool_results.push(api_block);
                }
            }
        }

        // Phase 2: Execute permitted tools concurrently, stream results as they complete
        let mut futs: FuturesUnordered<_> = permitted_tools
            .iter()
            .map(|pt| {
                let handler = &options.external_tool_handler;
                let executor = &tool_executor;
                let name = &pt.tool_name;
                let input = &pt.actual_input;
                let id = &pt.tool_use_id;
                async move {
                    debug!(tool = %name, "Executing tool");

                    let tool_result = if let Some(ref handler) = handler {
                        let ext_result = handler(name.clone(), input.clone()).await;
                        if let Some(tr) = ext_result {
                            tr
                        } else {
                            match executor.execute(name, input.clone()).await {
                                Ok(tr) => tr,
                                Err(e) => ToolResult {
                                    content: format!("{}", e),
                                    is_error: true,
                                    raw_content: None,
                                },
                            }
                        }
                    } else {
                        match executor.execute(name, input.clone()).await {
                            Ok(tr) => tr,
                            Err(e) => ToolResult {
                                content: format!("{}", e),
                                is_error: true,
                                raw_content: None,
                            },
                        }
                    };
                    (id.as_str(), name.as_str(), input, tool_result)
                }
            })
            .collect();

        while let Some((tool_use_id, tool_name, actual_input, mut tool_result)) = futs.next().await
        {
            // Sanitize tool result: strip blobs, enforce byte limit.
            let max_result_bytes = options
                .max_tool_result_bytes
                .unwrap_or(sanitize::DEFAULT_MAX_TOOL_RESULT_BYTES);
            tool_result.content =
                sanitize::sanitize_tool_result(&tool_result.content, max_result_bytes);

            // Run PostToolUse hooks
            hook_registry
                .run_post_tool_use(
                    tool_name,
                    actual_input,
                    &serde_json::to_value(&tool_result.content).unwrap_or_default(),
                    tool_use_id,
                    &session_id,
                    &cwd,
                )
                .await;

            let result_content = tool_result
                .raw_content
                .unwrap_or_else(|| json!(tool_result.content));

            let api_block = ApiContentBlock::ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: result_content,
                is_error: if tool_result.is_error {
                    Some(true)
                } else {
                    None
                },
                cache_control: None,
                name: Some(tool_name.to_string()),
            };

            // Stream this individual result to the frontend immediately
            let result_msg = Message::User(UserMessage {
                uuid: Some(Uuid::new_v4()),
                session_id: session_id.clone(),
                content: vec![api_block_to_content_block(&api_block)],
                parent_tool_use_id: None,
                is_synthetic: true,
                tool_use_result: None,
            });
            if options.persist_session {
                let _ = session
                    .append_message(&serde_json::to_value(&result_msg).unwrap_or_default())
                    .await;
            }
            if tx.send(Ok(result_msg)).is_err() {
                return Ok(());
            }

            tool_results.push(api_block);
        }

        // Add all tool results to conversation for the next API call
        conversation.push(ApiMessage {
            role: "user".to_string(),
            content: tool_results,
        });

        // --- Lightweight pruning (between turns, before full compaction) ---
        if let Some(context_budget) = options.context_budget {
            let prune_pct = options
                .prune_threshold_pct
                .unwrap_or(compact::DEFAULT_PRUNE_THRESHOLD_PCT);
            if compact::should_prune(response.usage.input_tokens, context_budget, prune_pct) {
                let max_chars = options
                    .prune_tool_result_max_chars
                    .unwrap_or(compact::DEFAULT_PRUNE_TOOL_RESULT_MAX_CHARS);
                let min_keep = options.min_keep_messages.unwrap_or(4);
                let removed = compact::prune_tool_results(&mut conversation, max_chars, min_keep);
                if removed > 0 {
                    debug!(
                        chars_removed = removed,
                        input_tokens = response.usage.input_tokens,
                        "Pruned oversized tool results to free context space"
                    );
                }
            }
        }

        // --- Compaction check (between turns) ---
        if let Some(context_budget) = options.context_budget {
            if compact::should_compact(response.usage.input_tokens, context_budget) {
                let min_keep = options.min_keep_messages.unwrap_or(4);
                let split_point = compact::find_split_point(&conversation, min_keep);
                if split_point > 0 {
                    debug!(
                        input_tokens = response.usage.input_tokens,
                        context_budget,
                        split_point,
                        "Context budget exceeded, compacting conversation"
                    );

                    let compaction_model = options
                        .compaction_model
                        .as_deref()
                        .unwrap_or(compact::DEFAULT_COMPACTION_MODEL);

                    // Fire pre-compact handler so the host can persist key facts
                    if let Some(ref handler) = options.pre_compact_handler {
                        let msgs_to_compact = conversation[..split_point].to_vec();
                        handler(msgs_to_compact).await;
                    }

                    let summary_prompt =
                        compact::build_summary_prompt(&conversation[..split_point]);

                    let summary_max_tokens = options.summary_max_tokens.unwrap_or(4096);
                    let compact_provider: &dyn LlmProvider = match &options.compaction_provider {
                        Some(cp) => cp.as_ref(),
                        None => provider.as_ref(),
                    };
                    let fallback_provider: Option<&dyn LlmProvider> =
                        if options.compaction_provider.is_some() {
                            Some(provider.as_ref())
                        } else {
                            None
                        };
                    match compact::call_summarizer(
                        compact_provider,
                        &summary_prompt,
                        compaction_model,
                        fallback_provider,
                        &model,
                        summary_max_tokens,
                    )
                    .await
                    {
                        Ok(summary) => {
                            let pre_tokens = response.usage.input_tokens;
                            let messages_compacted = split_point;

                            compact::splice_conversation(&mut conversation, split_point, &summary);

                            // Emit CompactBoundary system message
                            let compact_msg = Message::System(SystemMessage {
                                subtype: SystemSubtype::CompactBoundary,
                                uuid: Uuid::new_v4(),
                                session_id: session_id.clone(),
                                agents: None,
                                claude_code_version: None,
                                cwd: None,
                                tools: None,
                                mcp_servers: None,
                                model: None,
                                permission_mode: None,
                                compact_metadata: Some(CompactMetadata {
                                    trigger: CompactTrigger::Auto,
                                    pre_tokens,
                                }),
                            });

                            if options.persist_session {
                                let _ = session
                                    .append_message(
                                        &serde_json::to_value(&compact_msg).unwrap_or_default(),
                                    )
                                    .await;
                            }
                            let _ = tx.send(Ok(compact_msg));

                            debug!(
                                pre_tokens,
                                messages_compacted,
                                summary_len = summary.len(),
                                "Conversation compacted"
                            );
                        }
                        Err(e) => {
                            warn!("Compaction failed, continuing without compaction: {}", e);
                        }
                    }
                }
            }
        }
    }
}

/// Consume a streaming response, emitting `Message::StreamEvent` for each text
/// delta, and accumulate the full `MessageResponse` for the agent loop.
async fn accumulate_stream(
    event_stream: &mut std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<ClientStreamEvent>> + Send>,
    >,
    tx: &mpsc::UnboundedSender<Result<Message>>,
    session_id: &str,
) -> Result<MessageResponse> {
    use crate::client::StreamEvent as SE;

    // Accumulated state
    let mut message_id = String::new();
    let mut model = String::new();
    let mut role = String::from("assistant");
    let mut content_blocks: Vec<ApiContentBlock> = Vec::new();
    let mut stop_reason: Option<String> = None;
    let mut usage = ApiUsage::default();

    // Track in-progress content blocks by index
    // For text blocks: accumulate text. For tool_use: accumulate JSON string.
    let mut block_texts: Vec<String> = Vec::new();
    let mut block_types: Vec<String> = Vec::new(); // "text", "tool_use", "thinking"
    let mut block_tool_ids: Vec<String> = Vec::new();
    let mut block_tool_names: Vec<String> = Vec::new();

    while let Some(event_result) = FuturesStreamExt::next(event_stream).await {
        let event = event_result?;
        match event {
            SE::MessageStart { message } => {
                message_id = message.id;
                model = message.model;
                role = message.role;
                usage = message.usage;
            }
            SE::ContentBlockStart {
                index,
                content_block,
            } => {
                // Ensure vectors are large enough
                while block_texts.len() <= index {
                    block_texts.push(String::new());
                    block_types.push(String::new());
                    block_tool_ids.push(String::new());
                    block_tool_names.push(String::new());
                }
                match &content_block {
                    ApiContentBlock::Text { .. } => {
                        block_types[index] = "text".to_string();
                    }
                    ApiContentBlock::ToolUse { id, name, input } => {
                        block_types[index] = "tool_use".to_string();
                        block_tool_ids[index] = id.clone();
                        block_tool_names[index] = name.clone();
                        // OpenAI/Ollama streaming delivers the complete input
                        // in ContentBlockStart (not via InputJsonDelta like
                        // Anthropic). Store it so ContentBlockStop can parse it.
                        let input_str = input.to_string();
                        if input_str != "{}" {
                            block_texts[index] = input_str;
                        }
                    }
                    ApiContentBlock::Thinking { .. } => {
                        block_types[index] = "thinking".to_string();
                    }
                    _ => {}
                }
            }
            SE::ContentBlockDelta { index, delta } => {
                while block_texts.len() <= index {
                    block_texts.push(String::new());
                    block_types.push(String::new());
                    block_tool_ids.push(String::new());
                    block_tool_names.push(String::new());
                }
                match &delta {
                    ContentDelta::TextDelta { text } => {
                        block_texts[index].push_str(text);
                        // Emit streaming event so downstream consumers get per-token updates
                        let stream_event = Message::StreamEvent(StreamEventMessage {
                            event: serde_json::json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": { "type": "text_delta", "text": text }
                            }),
                            parent_tool_use_id: None,
                            uuid: Uuid::new_v4(),
                            session_id: session_id.to_string(),
                        });
                        if tx.send(Ok(stream_event)).is_err() {
                            return Err(AgentError::Cancelled);
                        }
                    }
                    ContentDelta::InputJsonDelta { partial_json } => {
                        block_texts[index].push_str(partial_json);
                    }
                    ContentDelta::ThinkingDelta { thinking } => {
                        block_texts[index].push_str(thinking);
                    }
                }
            }
            SE::ContentBlockStop { index } => {
                if index < block_types.len() {
                    let block = match block_types[index].as_str() {
                        "text" => ApiContentBlock::Text {
                            text: std::mem::take(&mut block_texts[index]),
                            cache_control: None,
                        },
                        "tool_use" => {
                            let input: serde_json::Value =
                                serde_json::from_str(&block_texts[index])
                                    .unwrap_or(serde_json::Value::Object(Default::default()));
                            ApiContentBlock::ToolUse {
                                id: std::mem::take(&mut block_tool_ids[index]),
                                name: std::mem::take(&mut block_tool_names[index]),
                                input,
                            }
                        }
                        "thinking" => ApiContentBlock::Thinking {
                            thinking: std::mem::take(&mut block_texts[index]),
                        },
                        _ => continue,
                    };
                    // Place blocks at the correct index
                    while content_blocks.len() <= index {
                        content_blocks.push(ApiContentBlock::Text {
                            text: String::new(),
                            cache_control: None,
                        });
                    }
                    content_blocks[index] = block;
                }
            }
            SE::MessageDelta {
                delta,
                usage: delta_usage,
            } => {
                stop_reason = delta.stop_reason;
                // MessageDelta carries output_tokens for the whole message
                usage.output_tokens = delta_usage.output_tokens;
            }
            SE::MessageStop => {
                break;
            }
            SE::Error { error } => {
                return Err(AgentError::Api(error.message));
            }
            SE::Ping => {}
        }
    }

    Ok(MessageResponse {
        id: message_id,
        role,
        content: content_blocks,
        model,
        stop_reason,
        usage,
    })
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
                ApiContentBlock::Image { .. }
                | ApiContentBlock::ToolUse { .. }
                | ApiContentBlock::Thinking { .. } => {}
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
                ApiContentBlock::Image { .. }
                | ApiContentBlock::ToolUse { .. }
                | ApiContentBlock::Thinking { .. } => {}
            }
        }
    }
}

/// Convert an API content block to our ContentBlock type.
fn api_block_to_content_block(block: &ApiContentBlock) -> ContentBlock {
    match block {
        ApiContentBlock::Text { text, .. } => ContentBlock::Text { text: text.clone() },
        ApiContentBlock::Image { .. } => ContentBlock::Text {
            text: "[image]".to_string(),
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

/// Repair orphaned `tool_use` blocks at the tail of a conversation.
///
/// The Claude API requires every `tool_use` block in an assistant message to have
/// a corresponding `tool_result` block in the immediately following user message.
/// If a session was interrupted mid-tool-execution (crash, timeout, kill), the
/// assistant message with `tool_use` blocks may have been persisted without the
/// subsequent `tool_result` messages. This function detects that case and appends
/// a synthetic user message with error `tool_result` blocks so the conversation
/// can be resumed without API validation errors.
fn repair_orphaned_tool_uses(conversation: &mut Vec<ApiMessage>) {
    // Walk backwards to find the last assistant message.
    let last_assistant_idx = conversation.iter().rposition(|m| m.role == "assistant");

    let Some(idx) = last_assistant_idx else {
        return;
    };

    // Collect tool_use IDs from that assistant message.
    let tool_use_ids: Vec<String> = conversation[idx]
        .content
        .iter()
        .filter_map(|block| match block {
            ApiContentBlock::ToolUse { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();

    if tool_use_ids.is_empty() {
        return;
    }

    // Collect tool_result IDs from all subsequent user messages.
    let mut answered_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in &conversation[idx + 1..] {
        if msg.role == "user" {
            for block in &msg.content {
                if let ApiContentBlock::ToolResult { tool_use_id, .. } = block {
                    answered_ids.insert(tool_use_id.clone());
                }
            }
        }
    }

    // Build synthetic tool_result blocks for any orphaned tool_use IDs.
    let orphaned: Vec<ApiContentBlock> = tool_use_ids
        .into_iter()
        .filter(|id| !answered_ids.contains(id))
        .map(|id| {
            warn!(tool_use_id = %id, "Repairing orphaned tool_use with synthetic error tool_result");
            ApiContentBlock::ToolResult {
                tool_use_id: id,
                content: json!("[Session interrupted — tool execution was not completed]"),
                is_error: Some(true),
                cache_control: None,
                name: None,
            }
        })
        .collect();

    if !orphaned.is_empty() {
        warn!(
            count = orphaned.len(),
            "Injected synthetic tool_result(s) for orphaned tool_use blocks"
        );
        conversation.push(ApiMessage {
            role: "user".to_string(),
            content: orphaned,
        });
    }
}

/// Build a ResultMessage.
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Helper: execute tools concurrently using the same FuturesUnordered pattern
    /// as the production code, collecting (tool_use_id, content, completion_order).
    async fn run_concurrent_tools(
        tools: Vec<(String, String, serde_json::Value)>,
        handler: impl Fn(
            String,
            serde_json::Value,
        ) -> Pin<Box<dyn futures::Future<Output = Option<ToolResult>> + Send>>,
    ) -> Vec<(String, String, usize)> {
        let order = Arc::new(AtomicUsize::new(0));
        let handler = Arc::new(handler);

        struct PermittedTool {
            tool_use_id: String,
            tool_name: String,
            actual_input: serde_json::Value,
        }

        let permitted: Vec<PermittedTool> = tools
            .into_iter()
            .map(|(id, name, input)| PermittedTool {
                tool_use_id: id,
                tool_name: name,
                actual_input: input,
            })
            .collect();

        let mut futs: FuturesUnordered<_> = permitted
            .iter()
            .map(|pt| {
                let handler = handler.clone();
                let order = order.clone();
                let name = pt.tool_name.clone();
                let input = pt.actual_input.clone();
                let id = pt.tool_use_id.clone();
                async move {
                    let result = handler(name, input).await;
                    let seq = order.fetch_add(1, Ordering::SeqCst);
                    (id, result, seq)
                }
            })
            .collect();

        let mut results = Vec::new();
        while let Some((id, result, seq)) = futs.next().await {
            let content = result
                .map(|r| r.content)
                .unwrap_or_else(|| "no handler".into());
            results.push((id, content, seq));
        }
        results
    }

    #[tokio::test]
    async fn concurrent_tools_all_complete() {
        let results = run_concurrent_tools(
            vec![
                ("t1".into(), "Read".into(), json!({"path": "a.txt"})),
                ("t2".into(), "Read".into(), json!({"path": "b.txt"})),
                ("t3".into(), "Read".into(), json!({"path": "c.txt"})),
            ],
            |name, input| {
                Box::pin(async move {
                    let path = input["path"].as_str().unwrap_or("?");
                    Some(ToolResult {
                        content: format!("{}: {}", name, path),
                        is_error: false,
                        raw_content: None,
                    })
                })
            },
        )
        .await;

        assert_eq!(results.len(), 3);
        let ids: Vec<&str> = results.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(ids.contains(&"t1"));
        assert!(ids.contains(&"t2"));
        assert!(ids.contains(&"t3"));
    }

    #[tokio::test]
    async fn slow_tool_does_not_block_fast_tools() {
        let start = Instant::now();

        let results = run_concurrent_tools(
            vec![
                ("slow".into(), "Bash".into(), json!({})),
                ("fast1".into(), "Read".into(), json!({})),
                ("fast2".into(), "Read".into(), json!({})),
            ],
            |name, _input| {
                Box::pin(async move {
                    if name == "Bash" {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        Some(ToolResult {
                            content: "slow done".into(),
                            is_error: false,
                            raw_content: None,
                        })
                    } else {
                        // Fast tools complete immediately
                        Some(ToolResult {
                            content: "fast done".into(),
                            is_error: false,
                            raw_content: None,
                        })
                    }
                })
            },
        )
        .await;

        let elapsed = start.elapsed();

        // All three should complete
        assert_eq!(results.len(), 3);

        // Fast tools should complete before the slow tool (lower order index)
        let slow = results.iter().find(|(id, _, _)| id == "slow").unwrap();
        let fast1 = results.iter().find(|(id, _, _)| id == "fast1").unwrap();
        let fast2 = results.iter().find(|(id, _, _)| id == "fast2").unwrap();

        assert!(fast1.2 < slow.2, "fast1 should complete before slow");
        assert!(fast2.2 < slow.2, "fast2 should complete before slow");

        // Total time should be ~200ms (concurrent), not ~400ms+ (sequential)
        assert!(
            elapsed < Duration::from_millis(400),
            "elapsed {:?} should be under 400ms (concurrent execution)",
            elapsed
        );
    }

    #[tokio::test]
    async fn results_streamed_individually_as_they_complete() {
        // Simulate the streaming pattern from the production code:
        // each tool result is sent to the channel as it completes.
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, String)>();

        let tools = vec![
            ("t_slow".into(), "Slow".into(), json!({})),
            ("t_fast".into(), "Fast".into(), json!({})),
        ];

        struct PT {
            tool_use_id: String,
            tool_name: String,
        }

        let permitted: Vec<PT> = tools
            .into_iter()
            .map(|(id, name, _)| PT {
                tool_use_id: id,
                tool_name: name,
            })
            .collect();

        let mut futs: FuturesUnordered<_> = permitted
            .iter()
            .map(|pt| {
                let name = pt.tool_name.clone();
                let id = pt.tool_use_id.clone();
                async move {
                    if name == "Slow" {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    let result = ToolResult {
                        content: format!("{} result", name),
                        is_error: false,
                        raw_content: None,
                    };
                    (id, result)
                }
            })
            .collect();

        // Process results as they complete (like production code)
        while let Some((id, result)) = futs.next().await {
            tx.send((id, result.content)).unwrap();
        }
        drop(tx);

        // Collect what was streamed
        let mut streamed = Vec::new();
        while let Some(item) = rx.recv().await {
            streamed.push(item);
        }

        assert_eq!(streamed.len(), 2);
        // Fast should arrive first
        assert_eq!(streamed[0].0, "t_fast");
        assert_eq!(streamed[0].1, "Fast result");
        assert_eq!(streamed[1].0, "t_slow");
        assert_eq!(streamed[1].1, "Slow result");
    }

    #[tokio::test]
    async fn error_tool_does_not_prevent_other_tools() {
        let results = run_concurrent_tools(
            vec![
                ("t_ok".into(), "Read".into(), json!({})),
                ("t_err".into(), "Fail".into(), json!({})),
            ],
            |name, _input| {
                Box::pin(async move {
                    if name == "Fail" {
                        Some(ToolResult {
                            content: "something went wrong".into(),
                            is_error: true,
                            raw_content: None,
                        })
                    } else {
                        Some(ToolResult {
                            content: "ok".into(),
                            is_error: false,
                            raw_content: None,
                        })
                    }
                })
            },
        )
        .await;

        assert_eq!(results.len(), 2);
        let ok = results.iter().find(|(id, _, _)| id == "t_ok").unwrap();
        let err = results.iter().find(|(id, _, _)| id == "t_err").unwrap();
        assert_eq!(ok.1, "ok");
        assert_eq!(err.1, "something went wrong");
    }

    #[tokio::test]
    async fn external_handler_none_falls_through_correctly() {
        // When handler returns None for a tool, the production code falls through
        // to the built-in executor. Test that the pattern works.
        let results = run_concurrent_tools(
            vec![
                ("t_custom".into(), "MyTool".into(), json!({"x": 1})),
                ("t_builtin".into(), "Read".into(), json!({"path": "/tmp"})),
            ],
            |name, _input| {
                Box::pin(async move {
                    if name == "MyTool" {
                        Some(ToolResult {
                            content: "custom handled".into(),
                            is_error: false,
                            raw_content: None,
                        })
                    } else {
                        // Returns None => would fall through to built-in executor
                        None
                    }
                })
            },
        )
        .await;

        assert_eq!(results.len(), 2);
        let custom = results.iter().find(|(id, _, _)| id == "t_custom").unwrap();
        let builtin = results.iter().find(|(id, _, _)| id == "t_builtin").unwrap();
        assert_eq!(custom.1, "custom handled");
        assert_eq!(builtin.1, "no handler"); // our test helper treats None as "no handler"
    }

    #[tokio::test]
    async fn single_tool_works_same_as_before() {
        let results = run_concurrent_tools(
            vec![("t1".into(), "Read".into(), json!({"path": "file.txt"}))],
            |_name, _input| {
                Box::pin(async move {
                    Some(ToolResult {
                        content: "file contents".into(),
                        is_error: false,
                        raw_content: None,
                    })
                })
            },
        )
        .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "t1");
        assert_eq!(results[0].1, "file contents");
        assert_eq!(results[0].2, 0); // first (and only) completion
    }

    #[tokio::test]
    async fn empty_tool_list_produces_no_results() {
        let results =
            run_concurrent_tools(vec![], |_name, _input| Box::pin(async move { None })).await;

        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn tool_use_ids_preserved_through_concurrent_execution() {
        let results = run_concurrent_tools(
            vec![
                ("toolu_abc123".into(), "Read".into(), json!({})),
                ("toolu_def456".into(), "Write".into(), json!({})),
                ("toolu_ghi789".into(), "Bash".into(), json!({})),
            ],
            |name, _input| {
                Box::pin(async move {
                    // Add varying delays to shuffle completion order
                    match name.as_str() {
                        "Read" => tokio::time::sleep(Duration::from_millis(30)).await,
                        "Write" => tokio::time::sleep(Duration::from_millis(10)).await,
                        _ => tokio::time::sleep(Duration::from_millis(50)).await,
                    }
                    Some(ToolResult {
                        content: format!("{} result", name),
                        is_error: false,
                        raw_content: None,
                    })
                })
            },
        )
        .await;

        assert_eq!(results.len(), 3);

        // Regardless of completion order, IDs must match their tools
        for (id, content, _) in &results {
            match id.as_str() {
                "toolu_abc123" => assert_eq!(content, "Read result"),
                "toolu_def456" => assert_eq!(content, "Write result"),
                "toolu_ghi789" => assert_eq!(content, "Bash result"),
                other => panic!("unexpected tool_use_id: {}", other),
            }
        }
    }

    #[tokio::test]
    async fn concurrent_execution_timing_is_parallel() {
        // 5 tools each taking 50ms should complete in ~50ms total, not 250ms
        let tools: Vec<_> = (0..5)
            .map(|i| (format!("t{}", i), "Tool".into(), json!({})))
            .collect();

        let start = Instant::now();

        let results = run_concurrent_tools(tools, |_name, _input| {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Some(ToolResult {
                    content: "done".into(),
                    is_error: false,
                    raw_content: None,
                })
            })
        })
        .await;

        let elapsed = start.elapsed();

        assert_eq!(results.len(), 5);
        // Should complete in roughly 50ms, definitely under 200ms
        assert!(
            elapsed < Duration::from_millis(200),
            "5 x 50ms tools took {:?} — should be ~50ms if concurrent",
            elapsed
        );
    }

    #[tokio::test]
    async fn api_block_to_content_block_preserves_tool_result_fields() {
        let block = ApiContentBlock::ToolResult {
            tool_use_id: "toolu_abc".into(),
            content: json!("result text"),
            is_error: Some(true),
            cache_control: None,
            name: None,
        };

        let content = api_block_to_content_block(&block);
        match content {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "toolu_abc");
                assert_eq!(content, json!("result text"));
                assert_eq!(is_error, Some(true));
            }
            _ => panic!("expected ToolResult content block"),
        }
    }

    #[tokio::test]
    async fn streamed_messages_each_contain_single_tool_result() {
        // Verify that the streaming pattern produces one User message per tool result
        let (tx, mut rx) = mpsc::unbounded_channel::<Result<Message>>();
        let session_id = "test-session".to_string();

        // Simulate what the production code does
        let tool_ids = vec!["t1", "t2", "t3"];
        for id in &tool_ids {
            let api_block = ApiContentBlock::ToolResult {
                tool_use_id: id.to_string(),
                content: json!(format!("result for {}", id)),
                is_error: None,
                cache_control: None,
                name: None,
            };

            let result_msg = Message::User(UserMessage {
                uuid: Some(Uuid::new_v4()),
                session_id: session_id.clone(),
                content: vec![api_block_to_content_block(&api_block)],
                parent_tool_use_id: None,
                is_synthetic: true,
                tool_use_result: None,
            });
            tx.send(Ok(result_msg)).unwrap();
        }
        drop(tx);

        let mut messages = Vec::new();
        while let Some(Ok(msg)) = rx.recv().await {
            messages.push(msg);
        }

        assert_eq!(messages.len(), 3, "should have 3 individual messages");

        for (i, msg) in messages.iter().enumerate() {
            if let Message::User(user) = msg {
                assert_eq!(
                    user.content.len(),
                    1,
                    "each message should have exactly 1 content block"
                );
                assert!(user.is_synthetic);
                if let ContentBlock::ToolResult { tool_use_id, .. } = &user.content[0] {
                    assert_eq!(tool_use_id, tool_ids[i]);
                } else {
                    panic!("expected ToolResult block");
                }
            } else {
                panic!("expected User message");
            }
        }
    }

    #[tokio::test]
    async fn accumulate_stream_emits_text_deltas_and_builds_response() {
        use crate::client::{
            ApiContentBlock, ApiUsage, ContentDelta, MessageResponse, StreamEvent as SE,
        };

        // Build a fake stream of SSE events
        let events: Vec<Result<SE>> = vec![
            Ok(SE::MessageStart {
                message: MessageResponse {
                    id: "msg_123".into(),
                    role: "assistant".into(),
                    content: vec![],
                    model: "claude-test".into(),
                    stop_reason: None,
                    usage: ApiUsage {
                        input_tokens: 100,
                        output_tokens: 0,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                    },
                },
            }),
            Ok(SE::ContentBlockStart {
                index: 0,
                content_block: ApiContentBlock::Text {
                    text: String::new(),
                    cache_control: None,
                },
            }),
            Ok(SE::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta {
                    text: "Hello".into(),
                },
            }),
            Ok(SE::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta {
                    text: " world".into(),
                },
            }),
            Ok(SE::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta { text: "!".into() },
            }),
            Ok(SE::ContentBlockStop { index: 0 }),
            Ok(SE::MessageDelta {
                delta: crate::client::MessageDelta {
                    stop_reason: Some("end_turn".into()),
                },
                usage: ApiUsage {
                    input_tokens: 0,
                    output_tokens: 15,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                },
            }),
            Ok(SE::MessageStop),
        ];

        let stream = futures::stream::iter(events);
        let mut boxed_stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<SE>> + Send>> =
            Box::pin(stream);

        let (tx, mut rx) = mpsc::unbounded_channel();

        let response = accumulate_stream(&mut boxed_stream, &tx, "test-session")
            .await
            .expect("accumulate_stream should succeed");

        // Verify accumulated response
        assert_eq!(response.id, "msg_123");
        assert_eq!(response.model, "claude-test");
        assert_eq!(response.stop_reason, Some("end_turn".into()));
        assert_eq!(response.usage.output_tokens, 15);
        assert_eq!(response.content.len(), 1);
        if let ApiContentBlock::Text { text, .. } = &response.content[0] {
            assert_eq!(text, "Hello world!");
        } else {
            panic!("expected Text content block");
        }

        // Verify 3 StreamEvent messages were emitted (one per text delta)
        let mut stream_events = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            stream_events.push(msg.unwrap());
        }
        assert_eq!(stream_events.len(), 3);

        // Verify each is a StreamEvent with the correct text
        let expected_texts = ["Hello", " world", "!"];
        for (i, msg) in stream_events.iter().enumerate() {
            if let Message::StreamEvent(se) = msg {
                let delta = se.event.get("delta").unwrap();
                let text = delta.get("text").unwrap().as_str().unwrap();
                assert_eq!(text, expected_texts[i]);
                assert_eq!(se.session_id, "test-session");
            } else {
                panic!("expected StreamEvent message at index {}", i);
            }
        }
    }

    #[tokio::test]
    async fn accumulate_stream_handles_tool_use() {
        use crate::client::{
            ApiContentBlock, ApiUsage, ContentDelta, MessageResponse, StreamEvent as SE,
        };

        let events: Vec<Result<SE>> = vec![
            Ok(SE::MessageStart {
                message: MessageResponse {
                    id: "msg_456".into(),
                    role: "assistant".into(),
                    content: vec![],
                    model: "claude-test".into(),
                    stop_reason: None,
                    usage: ApiUsage::default(),
                },
            }),
            // Text block
            Ok(SE::ContentBlockStart {
                index: 0,
                content_block: ApiContentBlock::Text {
                    text: String::new(),
                    cache_control: None,
                },
            }),
            Ok(SE::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta {
                    text: "Let me check.".into(),
                },
            }),
            Ok(SE::ContentBlockStop { index: 0 }),
            // Tool use block
            Ok(SE::ContentBlockStart {
                index: 1,
                content_block: ApiContentBlock::ToolUse {
                    id: "toolu_abc".into(),
                    name: "Read".into(),
                    input: serde_json::json!({}),
                },
            }),
            Ok(SE::ContentBlockDelta {
                index: 1,
                delta: ContentDelta::InputJsonDelta {
                    partial_json: r#"{"path":"/tmp/f.txt"}"#.into(),
                },
            }),
            Ok(SE::ContentBlockStop { index: 1 }),
            Ok(SE::MessageDelta {
                delta: crate::client::MessageDelta {
                    stop_reason: Some("tool_use".into()),
                },
                usage: ApiUsage {
                    input_tokens: 0,
                    output_tokens: 20,
                    ..Default::default()
                },
            }),
            Ok(SE::MessageStop),
        ];

        let stream = futures::stream::iter(events);
        let mut boxed_stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<SE>> + Send>> =
            Box::pin(stream);

        let (tx, _rx) = mpsc::unbounded_channel();
        let response = accumulate_stream(&mut boxed_stream, &tx, "test-session")
            .await
            .expect("should succeed");

        assert_eq!(response.content.len(), 2);
        if let ApiContentBlock::Text { text, .. } = &response.content[0] {
            assert_eq!(text, "Let me check.");
        } else {
            panic!("expected Text block at index 0");
        }
        if let ApiContentBlock::ToolUse { id, name, input } = &response.content[1] {
            assert_eq!(id, "toolu_abc");
            assert_eq!(name, "Read");
            assert_eq!(input["path"], "/tmp/f.txt");
        } else {
            panic!("expected ToolUse block at index 1");
        }
        assert_eq!(response.stop_reason, Some("tool_use".into()));
    }

    /// OpenAI/Ollama streaming delivers the complete tool input inside
    /// `ContentBlockStart` (no `InputJsonDelta` follows). Verify that
    /// `accumulate_stream` preserves that input instead of defaulting to `{}`.
    #[tokio::test]
    async fn accumulate_stream_preserves_openai_tool_input() {
        use crate::client::{ApiContentBlock, ApiUsage, StreamEvent as SE};

        let events: Vec<Result<SE>> = vec![
            Ok(SE::MessageStart {
                message: MessageResponse {
                    id: "msg_oai".into(),
                    role: "assistant".into(),
                    content: vec![],
                    model: "qwen3:8b".into(),
                    stop_reason: None,
                    usage: ApiUsage::default(),
                },
            }),
            // Tool use with full input in ContentBlockStart (OpenAI/Ollama pattern)
            Ok(SE::ContentBlockStart {
                index: 0,
                content_block: ApiContentBlock::ToolUse {
                    id: "call_123".into(),
                    name: "Bash".into(),
                    input: serde_json::json!({"command": "ls -la", "timeout": 5000}),
                },
            }),
            // No InputJsonDelta — OpenAI/Ollama doesn't send one
            Ok(SE::ContentBlockStop { index: 0 }),
            Ok(SE::MessageDelta {
                delta: crate::client::MessageDelta {
                    stop_reason: Some("tool_use".into()),
                },
                usage: ApiUsage {
                    input_tokens: 0,
                    output_tokens: 10,
                    ..Default::default()
                },
            }),
            Ok(SE::MessageStop),
        ];

        let stream = futures::stream::iter(events);
        let mut boxed_stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<SE>> + Send>> =
            Box::pin(stream);

        let (tx, _rx) = mpsc::unbounded_channel();
        let response = accumulate_stream(&mut boxed_stream, &tx, "test-session")
            .await
            .expect("should succeed");

        assert_eq!(response.content.len(), 1);
        if let ApiContentBlock::ToolUse { id, name, input } = &response.content[0] {
            assert_eq!(id, "call_123");
            assert_eq!(name, "Bash");
            assert_eq!(input["command"], "ls -la");
            assert_eq!(input["timeout"], 5000);
        } else {
            panic!("expected ToolUse block");
        }
    }

    #[test]
    fn repair_orphaned_tool_uses_injects_synthetic_results() {
        let mut conversation = vec![
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "run a command".to_string(),
                    cache_control: None,
                }],
            },
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![
                    ApiContentBlock::Text {
                        text: "Sure, let me run that.".to_string(),
                        cache_control: None,
                    },
                    ApiContentBlock::ToolUse {
                        id: "toolu_orphaned_1".to_string(),
                        name: "Bash".to_string(),
                        input: json!({"command": "ls"}),
                    },
                    ApiContentBlock::ToolUse {
                        id: "toolu_orphaned_2".to_string(),
                        name: "Bash".to_string(),
                        input: json!({"command": "pwd"}),
                    },
                ],
            },
        ];

        repair_orphaned_tool_uses(&mut conversation);

        assert_eq!(conversation.len(), 3);
        let repaired = &conversation[2];
        assert_eq!(repaired.role, "user");
        assert_eq!(repaired.content.len(), 2);

        for block in &repaired.content {
            match block {
                ApiContentBlock::ToolResult {
                    tool_use_id,
                    is_error,
                    ..
                } => {
                    assert!(tool_use_id == "toolu_orphaned_1" || tool_use_id == "toolu_orphaned_2");
                    assert_eq!(*is_error, Some(true));
                }
                _ => panic!("expected ToolResult block"),
            }
        }
    }

    #[test]
    fn repair_orphaned_tool_uses_noop_when_results_exist() {
        let mut conversation = vec![
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![ApiContentBlock::ToolUse {
                    id: "toolu_ok".to_string(),
                    name: "Bash".to_string(),
                    input: json!({"command": "ls"}),
                }],
            },
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::ToolResult {
                    tool_use_id: "toolu_ok".to_string(),
                    content: json!("file1\nfile2"),
                    is_error: None,
                    cache_control: None,
                    name: Some("Bash".to_string()),
                }],
            },
        ];

        let original_len = conversation.len();
        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), original_len);
    }

    #[test]
    fn repair_orphaned_tool_uses_partial_results() {
        // One tool_use has a result, the other doesn't
        let mut conversation = vec![
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![
                    ApiContentBlock::ToolUse {
                        id: "toolu_answered".to_string(),
                        name: "Bash".to_string(),
                        input: json!({"command": "ls"}),
                    },
                    ApiContentBlock::ToolUse {
                        id: "toolu_missing".to_string(),
                        name: "Bash".to_string(),
                        input: json!({"command": "pwd"}),
                    },
                ],
            },
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::ToolResult {
                    tool_use_id: "toolu_answered".to_string(),
                    content: json!("ok"),
                    is_error: None,
                    cache_control: None,
                    name: Some("Bash".to_string()),
                }],
            },
        ];

        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), 3);

        let repaired = &conversation[2];
        assert_eq!(repaired.content.len(), 1);
        if let ApiContentBlock::ToolResult { tool_use_id, .. } = &repaired.content[0] {
            assert_eq!(tool_use_id, "toolu_missing");
        } else {
            panic!("expected ToolResult");
        }
    }

    #[test]
    fn repair_orphaned_tool_uses_noop_no_tool_use() {
        let mut conversation = vec![
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "hello".to_string(),
                    cache_control: None,
                }],
            },
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "hi there".to_string(),
                    cache_control: None,
                }],
            },
        ];

        let original_len = conversation.len();
        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), original_len);
    }

    #[test]
    fn repair_orphaned_tool_uses_noop_empty_conversation() {
        let mut conversation: Vec<ApiMessage> = vec![];
        repair_orphaned_tool_uses(&mut conversation);
        assert!(conversation.is_empty());
    }

    #[test]
    fn repair_orphaned_tool_uses_noop_only_user_messages() {
        let mut conversation = vec![ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: "hello".to_string(),
                cache_control: None,
            }],
        }];

        let original_len = conversation.len();
        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), original_len);
    }

    #[test]
    fn repair_orphaned_single_tool_use() {
        let mut conversation = vec![ApiMessage {
            role: "assistant".to_string(),
            content: vec![ApiContentBlock::ToolUse {
                id: "toolu_single".to_string(),
                name: "Read".to_string(),
                input: json!({"path": "/tmp/test"}),
            }],
        }];

        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), 2);

        let repaired = &conversation[1];
        assert_eq!(repaired.role, "user");
        assert_eq!(repaired.content.len(), 1);
        if let ApiContentBlock::ToolResult {
            tool_use_id,
            is_error,
            content,
            ..
        } = &repaired.content[0]
        {
            assert_eq!(tool_use_id, "toolu_single");
            assert_eq!(*is_error, Some(true));
            assert!(content.as_str().unwrap().contains("Session interrupted"));
        } else {
            panic!("expected ToolResult");
        }
    }

    #[test]
    fn repair_orphaned_long_session_with_completed_cycles() {
        // Simulate a long session: multiple completed tool cycles, then orphan at tail
        let mut conversation = vec![
            // Turn 1: user asks
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "list files".to_string(),
                    cache_control: None,
                }],
            },
            // Turn 1: assistant calls tool
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![ApiContentBlock::ToolUse {
                    id: "toolu_turn1".to_string(),
                    name: "Bash".to_string(),
                    input: json!({"command": "ls"}),
                }],
            },
            // Turn 1: tool result
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::ToolResult {
                    tool_use_id: "toolu_turn1".to_string(),
                    content: json!("file1.txt\nfile2.txt"),
                    is_error: None,
                    cache_control: None,
                    name: Some("Bash".to_string()),
                }],
            },
            // Turn 1: assistant responds
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "Here are your files.".to_string(),
                    cache_control: None,
                }],
            },
            // Turn 2: user asks again
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "now start the tunnel".to_string(),
                    cache_control: None,
                }],
            },
            // Turn 2: assistant calls tool (ORPHANED - session crashed here)
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![
                    ApiContentBlock::Text {
                        text: "Starting tunnel now.".to_string(),
                        cache_control: None,
                    },
                    ApiContentBlock::ToolUse {
                        id: "toolu_crash".to_string(),
                        name: "Bash".to_string(),
                        input: json!({"command": "cloudflared tunnel run"}),
                    },
                ],
            },
        ];

        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), 7); // original 6 + 1 synthetic

        let repaired = &conversation[6];
        assert_eq!(repaired.role, "user");
        assert_eq!(repaired.content.len(), 1);
        if let ApiContentBlock::ToolResult { tool_use_id, .. } = &repaired.content[0] {
            assert_eq!(tool_use_id, "toolu_crash");
        } else {
            panic!("expected ToolResult for orphaned tool_use");
        }
    }

    #[test]
    fn repair_orphaned_user_text_after_tool_use_no_result() {
        // Assistant calls a tool, but the next user message is text (not a tool_result).
        // This can happen if value_to_api_message dropped the tool_result message
        // and a new user prompt was added.
        let mut conversation = vec![
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![ApiContentBlock::ToolUse {
                    id: "toolu_lost".to_string(),
                    name: "Bash".to_string(),
                    input: json!({"command": "ls"}),
                }],
            },
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "never mind, do something else".to_string(),
                    cache_control: None,
                }],
            },
        ];

        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), 3);

        let repaired = &conversation[2];
        assert_eq!(repaired.content.len(), 1);
        if let ApiContentBlock::ToolResult { tool_use_id, .. } = &repaired.content[0] {
            assert_eq!(tool_use_id, "toolu_lost");
        } else {
            panic!("expected ToolResult");
        }
    }

    #[test]
    fn repair_orphaned_idempotent() {
        // Calling repair twice should not add duplicate synthetic results
        let mut conversation = vec![ApiMessage {
            role: "assistant".to_string(),
            content: vec![ApiContentBlock::ToolUse {
                id: "toolu_idem".to_string(),
                name: "Bash".to_string(),
                input: json!({"command": "ls"}),
            }],
        }];

        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), 2);

        // Call again — the tool_result now exists, so it should be a no-op
        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), 2);
    }

    #[test]
    fn repair_orphaned_results_split_across_multiple_user_messages() {
        // Two tool_uses, results come in separate user messages (streamed individually)
        let mut conversation = vec![
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![
                    ApiContentBlock::ToolUse {
                        id: "toolu_a".to_string(),
                        name: "Bash".to_string(),
                        input: json!({"command": "ls"}),
                    },
                    ApiContentBlock::ToolUse {
                        id: "toolu_b".to_string(),
                        name: "Read".to_string(),
                        input: json!({"path": "/tmp/x"}),
                    },
                ],
            },
            // First result in its own message
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::ToolResult {
                    tool_use_id: "toolu_a".to_string(),
                    content: json!("ok"),
                    is_error: None,
                    cache_control: None,
                    name: Some("Bash".to_string()),
                }],
            },
            // Second result in a separate message
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::ToolResult {
                    tool_use_id: "toolu_b".to_string(),
                    content: json!("file content"),
                    is_error: None,
                    cache_control: None,
                    name: Some("Read".to_string()),
                }],
            },
        ];

        let original_len = conversation.len();
        repair_orphaned_tool_uses(&mut conversation);
        // Both results exist — no repair needed
        assert_eq!(conversation.len(), original_len);
    }

    #[test]
    fn repair_orphaned_with_thinking_blocks() {
        // Assistant message has thinking + tool_use — thinking shouldn't interfere
        let mut conversation = vec![ApiMessage {
            role: "assistant".to_string(),
            content: vec![
                ApiContentBlock::Thinking {
                    thinking: "Let me think about this...".to_string(),
                },
                ApiContentBlock::Text {
                    text: "I'll check that.".to_string(),
                    cache_control: None,
                },
                ApiContentBlock::ToolUse {
                    id: "toolu_think".to_string(),
                    name: "Bash".to_string(),
                    input: json!({"command": "cat /etc/hosts"}),
                },
            ],
        }];

        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), 2);

        let repaired = &conversation[1];
        assert_eq!(repaired.content.len(), 1);
        if let ApiContentBlock::ToolResult {
            tool_use_id,
            is_error,
            ..
        } = &repaired.content[0]
        {
            assert_eq!(tool_use_id, "toolu_think");
            assert_eq!(*is_error, Some(true));
        } else {
            panic!("expected ToolResult");
        }
    }

    #[test]
    fn repair_orphaned_earlier_assistant_with_tool_use_is_ignored() {
        // An earlier assistant message has tool_use (fully resolved), and the LAST
        // assistant message is text-only. No repair should happen — only the last
        // assistant message is checked.
        let mut conversation = vec![
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![ApiContentBlock::ToolUse {
                    id: "toolu_early".to_string(),
                    name: "Bash".to_string(),
                    input: json!({"command": "ls"}),
                }],
            },
            ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::ToolResult {
                    tool_use_id: "toolu_early".to_string(),
                    content: json!("output"),
                    is_error: None,
                    cache_control: None,
                    name: Some("Bash".to_string()),
                }],
            },
            ApiMessage {
                role: "assistant".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "All done!".to_string(),
                    cache_control: None,
                }],
            },
        ];

        let original_len = conversation.len();
        repair_orphaned_tool_uses(&mut conversation);
        assert_eq!(conversation.len(), original_len);
    }
}
