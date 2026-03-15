pub mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Local;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

use agent_sdk::{ExternalToolHandlerFn, LlmProvider, Message, Options, PermissionMode, Query, QueryAttachment};
use agent_sdk::{AnthropicProvider, GeminiProvider, OpenAiProvider};
use agent_sdk::options::{SystemPrompt, ThinkingConfig};
use starpod_core::{FollowupMode, ReasoningEffort};
use tokio::sync::mpsc;

use starpod_core::{Attachment, ChatMessage, ChatResponse, ChatUsage, StarpodConfig, StarpodError, Result};
use starpod_cron::CronStore;
use starpod_memory::MemoryStore;
use starpod_session::{Channel, SessionDecision, SessionManager, UsageRecord};
use starpod_skills::SkillStore;
use starpod_vault::Vault;

use crate::tools::{custom_tool_definitions, handle_custom_tool, ToolContext};

/// All custom tool names.
const CUSTOM_TOOLS: &[&str] = &[
    "MemorySearch", "MemoryWrite", "MemoryAppendDaily",
    "VaultGet", "VaultSet",
    "SkillActivate", "SkillCreate", "SkillUpdate", "SkillDelete", "SkillList",
    "CronAdd", "CronList", "CronRemove", "CronRuns",
    "CronRun", "CronUpdate", "HeartbeatWake",
];

/// The Starpod agent orchestrator.
///
/// Wires together memory, sessions, vault, skills, cron, and the agent-sdk
/// to provide a high-level `chat()` interface.
pub struct StarpodAgent {
    memory: Arc<MemoryStore>,
    session_mgr: Arc<SessionManager>,
    vault: Arc<Vault>,
    skills: Arc<SkillStore>,
    cron: Arc<CronStore>,
    config: StarpodConfig,
}

impl StarpodAgent {
    /// Create a new StarpodAgent from config.
    pub async fn new(config: StarpodConfig) -> Result<Self> {
        let memory = MemoryStore::new(&config.data_dir).await?;

        let session_db = config.data_dir.join("session.db");
        let sessions_dir = config.data_dir.join("sessions");
        let session_mgr = SessionManager::new(&session_db, &sessions_dir).await?;

        let vault_db = config.data_dir.join("vault.db");
        let master_key = derive_vault_key(&config);
        let vault = Vault::new(&vault_db, &master_key).await?;

        let skills = SkillStore::new(&config.data_dir)?;

        let cron_db = config.data_dir.join("cron.db");
        let cron = CronStore::new(&cron_db).await?;

        Ok(Self {
            memory: Arc::new(memory),
            session_mgr: Arc::new(session_mgr),
            vault: Arc::new(vault),
            skills: Arc::new(skills),
            cron: Arc::new(cron),
            config,
        })
    }

    /// Path to the downloads directory (lives in the project root, not inside `.starpod/`).
    fn downloads_dir(&self) -> PathBuf {
        self.config.project_root.join("downloads")
    }

    /// Save attachments to disk under `{project_root}/downloads/`.
    /// Returns a list of saved file paths.
    async fn save_attachments(
        &self,
        attachments: &[Attachment],
    ) -> Vec<PathBuf> {
        if attachments.is_empty() {
            return Vec::new();
        }

        let dir = self.downloads_dir();
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!(error = %e, "Failed to create downloads directory");
            return Vec::new();
        }

        let ts = Local::now().format("%Y%m%d_%H%M%S");
        let mut paths = Vec::new();
        for att in attachments {
            let safe_name = att.file_name
                .replace(['/', '\\', ':', '\0'], "_")
                .replace("..", "_");
            let filename = format!("{ts}_{safe_name}");
            let path = dir.join(&filename);

            // Decode base64 and write
            use base64::Engine;
            match base64::engine::general_purpose::STANDARD.decode(&att.data) {
                Ok(bytes) => {
                    if let Err(e) = tokio::fs::write(&path, &bytes).await {
                        warn!(error = %e, file = %filename, "Failed to save attachment");
                    } else {
                        debug!(path = %path.display(), "Saved attachment");
                        paths.push(path);
                    }
                }
                Err(e) => {
                    warn!(error = %e, file = %filename, "Failed to decode base64 attachment");
                }
            }
        }
        paths
    }

    /// Convert chat attachments to agent-sdk query attachments.
    /// Images are passed through for vision; non-images get a text note instead.
    fn build_query_attachments(
        attachments: &[Attachment],
        saved_paths: &[PathBuf],
    ) -> (Vec<QueryAttachment>, String) {
        let mut query_atts = Vec::new();
        let mut extra_text = String::new();

        for (i, att) in attachments.iter().enumerate() {
            if att.is_image() {
                query_atts.push(QueryAttachment {
                    file_name: att.file_name.clone(),
                    mime_type: att.mime_type.clone(),
                    base64_data: att.data.clone(),
                });
            } else {
                // Non-image: tell Claude the file was saved
                let path = saved_paths.get(i)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(save failed)".to_string());
                extra_text.push_str(&format!(
                    "\n[Attached file: {} ({}) saved to: {}]",
                    att.file_name, att.mime_type, path
                ));
            }
        }

        (query_atts, extra_text)
    }

    /// Build the system prompt from bootstrap context + skill catalog + identity + user.
    fn build_system_prompt(&self, session_id: &str) -> Result<String> {
        let agent_name = self.config.identity.display_name();
        let bootstrap = self.memory.bootstrap_context()?;
        let skill_catalog = self.skills.skill_catalog()?;
        let date_str = Local::now().format("%A, %B %d, %Y at %H:%M").to_string();
        let project_root = self.config.project_root.display();

        let mut prompt = format!(
            "You are {agent_name}, a personal AI assistant.\n\n{bootstrap}\n\n---\n\
             Current date/time: {date_str}\nSession ID: {session_id}\n\
             Project root: {project_root}\n\
             Working directory: {project_root}\n\n\
             You have access to memory tools (MemorySearch, MemoryWrite, MemoryAppendDaily), \
             vault tools (VaultGet, VaultSet), skill tools (SkillActivate, SkillCreate, SkillUpdate, SkillDelete, SkillList), \
             and scheduling tools (CronAdd, CronList, CronRemove, CronRuns, CronRun, CronUpdate, HeartbeatWake).\n\
             You can read image files (png, jpg, gif, webp) with the Read tool — the image will be loaded \
             directly into the conversation so you can see and analyze it. For other file types like CSV or \
             PDF, use Python via the Bash tool.\n\n\
             IMPORTANT — two separate domains of information:\n\
             • Your personal knowledge, memory, soul, and user profile live inside `.starpod/data/` \
             (SOUL.md, USER.md, MEMORY.md, memory/, knowledge/). Use MemorySearch to query this knowledge \
             and MemoryWrite to persist new information there.\n\
             • The user's project files (code, documents, data) live in the project root directory ({project_root}). \
             Use Read, Glob, Grep, and Bash to explore and work with these files.\n\
             Never confuse the two: `.starpod/` is YOUR persistent brain; the project root is the USER's workspace.\n\
             You may ONLY access files within the project root and the `.starpod/` directory. \
             Do not read, write, or execute anything outside these boundaries.\n\
             IMPORTANT: Always create files and run commands within the project root ({project_root}), never in /tmp or other external directories.",
        );

        // Inject agent personality
        if let Some(ref soul) = self.config.identity.soul {
            if !soul.is_empty() {
                prompt.push_str(&format!("\n\nPersonality: {soul}"));
            }
        }

        // Inject user context
        if let Some(ref name) = self.config.user.name {
            prompt.push_str(&format!("\nUser's name: {name}"));
        }
        if let Some(ref tz) = self.config.user.timezone {
            prompt.push_str(&format!("\nUser's timezone: {tz}"));
        }

        // Inject skill catalog (progressive disclosure — names + descriptions only)
        if !skill_catalog.is_empty() {
            prompt.push_str("\n\nThe following skills provide specialized instructions for specific tasks.\n\
                             When a task matches a skill's description, call the SkillActivate tool \
                             with the skill's name to load its full instructions before proceeding.\n\n");
            prompt.push_str(&skill_catalog);
        }

        Ok(prompt)
    }

    /// Map reasoning effort config to ThinkingConfig.
    fn thinking_config(&self) -> Option<ThinkingConfig> {
        self.config.reasoning_effort.map(|effort| match effort {
            ReasoningEffort::Low => ThinkingConfig::Enabled { budget_tokens: 4096 },
            ReasoningEffort::Medium => ThinkingConfig::Enabled { budget_tokens: 10240 },
            ReasoningEffort::High => ThinkingConfig::Enabled { budget_tokens: 32768 },
        })
    }

    /// Build the allowed tools list (built-in + custom).
    fn allowed_tools() -> Vec<String> {
        let mut tools: Vec<String> = vec![
            "Read".into(), "Bash".into(), "Glob".into(), "Grep".into(),
        ];
        tools.extend(CUSTOM_TOOLS.iter().map(|s| s.to_string()));
        tools
    }

    /// Build the LLM provider based on `config.provider`.
    fn build_provider(&self) -> Result<Box<dyn LlmProvider>> {
        let provider_name = &self.config.provider;
        let api_key = self.config.resolved_provider_api_key(provider_name)
            .ok_or_else(|| StarpodError::Config(format!(
                "No API key found for provider '{}'. Set it in config.toml or via environment variable.",
                provider_name
            )))?;
        let base_url = self.config.resolved_provider_base_url(provider_name)
            .ok_or_else(|| StarpodError::Config(format!(
                "Unknown provider: '{}'",
                provider_name
            )))?;

        let provider: Box<dyn LlmProvider> = match provider_name.as_str() {
            "anthropic" => Box::new(AnthropicProvider::new(api_key, base_url)),
            "gemini" => Box::new(GeminiProvider::with_base_url(api_key, base_url)),
            // OpenAI-compatible providers
            "openai" | "groq" | "deepseek" | "openrouter" | "ollama" => {
                Box::new(OpenAiProvider::with_base_url(api_key, base_url, provider_name))
            }
            other => {
                return Err(StarpodError::Config(format!(
                    "Unsupported provider: '{}'. Supported: anthropic, openai, gemini, groq, deepseek, openrouter, ollama",
                    other
                )));
            }
        };

        Ok(provider)
    }

    /// Build the external tool handler closure.
    fn build_tool_handler(&self) -> ExternalToolHandlerFn {
        let ctx = Arc::new(ToolContext {
            memory: Arc::clone(&self.memory),
            vault: Arc::clone(&self.vault),
            skills: Arc::clone(&self.skills),
            cron: Arc::clone(&self.cron),
            user_tz: self.config.user.timezone.clone(),
        });

        Box::new(move |tool_name, input| {
            let ctx = Arc::clone(&ctx);
            Box::pin(async move {
                handle_custom_tool(&ctx, &tool_name, &input).await
            })
        })
    }

    /// Process a chat message through the full Starpod pipeline.
    pub async fn chat(&self, message: ChatMessage) -> Result<ChatResponse> {
        // Step 1: Resolve session via channel routing
        let (channel, key) = resolve_channel(&message);
        let session_id = match self.session_mgr.resolve_session(&channel, &key).await? {
            SessionDecision::Continue(id) => {
                debug!(session_id = %id, channel = %channel.as_str(), "Continuing existing session");
                id
            }
            SessionDecision::New => {
                let id = self.session_mgr.create_session(&channel, &key).await?;
                debug!(session_id = %id, channel = %channel.as_str(), "Created new session");
                id
            }
        };
        self.session_mgr.touch_session(&session_id).await?;
        let _ = self.session_mgr.set_title_if_empty(&session_id, &message.text).await;

        // Step 2: Save attachments to downloads/ and build query attachments
        let saved_paths = self.save_attachments(&message.attachments).await;
        let (query_atts, extra_text) =
            Self::build_query_attachments(&message.attachments, &saved_paths);

        // Append file info to prompt if there are non-image attachments
        let prompt = if extra_text.is_empty() {
            message.text.clone()
        } else {
            format!("{}{}", message.text, extra_text)
        };

        // Step 3: Build system prompt
        let system_prompt = self.build_system_prompt(&session_id)?;

        // Step 4: Build provider and options, then run query
        let provider = self.build_provider()?;

        let mut builder = Options::builder()
            .allowed_tools(Self::allowed_tools())
            .system_prompt(SystemPrompt::Custom(system_prompt))
            .permission_mode(PermissionMode::BypassPermissions)
            .model(&self.config.model)
            .max_turns(self.config.max_turns)
            .session_id(session_id.clone())
            .context_budget(160_000)
            .external_tool_handler(self.build_tool_handler())
            .custom_tools(custom_tool_definitions())
            .attachments(query_atts)
            .provider(provider)
            .cwd(self.config.project_root.to_string_lossy().to_string())
            .additional_directories(vec![
                self.config.data_dir.to_string_lossy().to_string(),
            ])
            .hook_dirs(vec![self.config.data_dir.join("hooks")]);

        if let Some(ref cm) = self.config.compaction_model {
            builder = builder.compaction_model(cm);
        }

        if let Some(key) = self.config.resolved_api_key() {
            builder = builder.api_key(key);
        }
        if let Some(thinking) = self.thinking_config() {
            builder = builder.thinking(thinking);
        }

        let options = builder.build();

        let mut stream = agent_sdk::query(&prompt, options);

        // Step 5: Collect result
        let mut result_text = String::new();
        let mut usage = ChatUsage::default();

        while let Some(msg_result) = stream.next().await {
            match msg_result {
                Ok(Message::Assistant(assistant)) => {
                    for block in &assistant.content {
                        if let agent_sdk::ContentBlock::Text { text } = block {
                            if !result_text.is_empty() {
                                result_text.push('\n');
                            }
                            result_text.push_str(text);
                        }
                    }
                }
                Ok(Message::Result(result)) => {
                    if result_text.is_empty() {
                        if let Some(text) = &result.result {
                            result_text = text.clone();
                        }
                    }

                    if let Some(u) = &result.usage {
                        usage = ChatUsage {
                            input_tokens: u.input_tokens,
                            output_tokens: u.output_tokens,
                            cache_read_tokens: u.cache_read_input_tokens,
                            cache_write_tokens: u.cache_creation_input_tokens,
                            cost_usd: result.total_cost_usd,
                        };

                        let _ = self.session_mgr.record_usage(
                            &session_id,
                            &UsageRecord {
                                input_tokens: u.input_tokens,
                                output_tokens: u.output_tokens,
                                cache_read: u.cache_read_input_tokens,
                                cache_write: u.cache_creation_input_tokens,
                                cost_usd: result.total_cost_usd,
                                model: self.config.model.clone(),
                            },
                            result.num_turns,
                        ).await;
                    }

                    if result.is_error {
                        if let Some(err) = result.errors.first() {
                            error!(error = %err, "Agent error");
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    error!(error = %e, "Stream error");
                    return Err(StarpodError::Agent(e.to_string()));
                }
            }
        }

        // Step 5: Append summary to daily log
        let summary = if result_text.len() > 200 {
            format!("{}...", &result_text[..200])
        } else {
            result_text.clone()
        };
        let agent_name = self.config.identity.display_name();
        let _ = self.memory.append_daily(&format!(
            "**User**: {}\n**{agent_name}**: {}",
            truncate(&message.text, 200),
            summary,
        )).await;

        Ok(ChatResponse {
            text: result_text,
            session_id,
            usage: Some(usage),
        })
    }

    /// Start a streaming chat that yields raw agent-sdk messages.
    ///
    /// Returns (Query stream, session_id, followup_tx). The caller should consume
    /// the stream for real-time display, then call `finalize_chat()` with the
    /// collected results.
    ///
    /// The returned `followup_tx` can be used to inject followup messages into
    /// the running agent loop (when `followup_mode = "inject"`). Messages sent
    /// through this channel are drained at each iteration boundary and appended
    /// as user messages before the next API call.
    pub async fn chat_stream(
        &self,
        message: &ChatMessage,
    ) -> Result<(Query, String, mpsc::UnboundedSender<String>)> {
        let (channel, key) = resolve_channel(message);
        let session_id = match self.session_mgr.resolve_session(&channel, &key).await? {
            SessionDecision::Continue(id) => {
                debug!(session_id = %id, channel = %channel.as_str(), "Continuing existing session");
                id
            }
            SessionDecision::New => {
                let id = self.session_mgr.create_session(&channel, &key).await?;
                debug!(session_id = %id, channel = %channel.as_str(), "Created new session");
                id
            }
        };
        self.session_mgr.touch_session(&session_id).await?;
        let _ = self.session_mgr.set_title_if_empty(&session_id, &message.text).await;

        // Save attachments and build query attachments
        let saved_paths = self.save_attachments(&message.attachments).await;
        let (query_atts, extra_text) =
            Self::build_query_attachments(&message.attachments, &saved_paths);

        let prompt = if extra_text.is_empty() {
            message.text.clone()
        } else {
            format!("{}{}", message.text, extra_text)
        };

        let system_prompt = self.build_system_prompt(&session_id)?;
        let provider = self.build_provider()?;

        // Create the followup channel — sender goes to caller, receiver to the agent loop
        let (followup_tx, followup_rx) = mpsc::unbounded_channel::<String>();

        let mut builder = Options::builder()
            .allowed_tools(Self::allowed_tools())
            .system_prompt(SystemPrompt::Custom(system_prompt))
            .permission_mode(PermissionMode::BypassPermissions)
            .model(&self.config.model)
            .max_turns(self.config.max_turns)
            .session_id(session_id.clone())
            .context_budget(160_000)
            .external_tool_handler(self.build_tool_handler())
            .custom_tools(custom_tool_definitions())
            .followup_rx(followup_rx)
            .attachments(query_atts)
            .provider(provider)
            .cwd(self.config.project_root.to_string_lossy().to_string())
            .additional_directories(vec![
                self.config.data_dir.to_string_lossy().to_string(),
            ])
            .hook_dirs(vec![self.config.data_dir.join("hooks")]);

        if let Some(ref cm) = self.config.compaction_model {
            builder = builder.compaction_model(cm);
        }

        if let Some(key) = self.config.resolved_api_key() {
            builder = builder.api_key(key);
        }
        if let Some(thinking) = self.thinking_config() {
            builder = builder.thinking(thinking);
        }

        let options = builder.build();

        let stream = agent_sdk::query(&prompt, options);
        Ok((stream, session_id, followup_tx))
    }

    /// Get the configured followup mode.
    pub fn followup_mode(&self) -> FollowupMode {
        self.config.followup_mode
    }

    /// Finalize a streaming chat — record usage and append daily log.
    pub async fn finalize_chat(
        &self,
        session_id: &str,
        user_text: &str,
        result_text: &str,
        result: &agent_sdk::ResultMessage,
    ) {
        if let Some(u) = &result.usage {
            let _ = self.session_mgr.record_usage(
                session_id,
                &UsageRecord {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cache_read: u.cache_read_input_tokens,
                    cache_write: u.cache_creation_input_tokens,
                    cost_usd: result.total_cost_usd,
                    model: self.config.model.clone(),
                },
                result.num_turns,
            ).await;
        }

        let summary = if result_text.len() > 200 {
            format!("{}...", &result_text[..200])
        } else {
            result_text.to_string()
        };
        let agent_name = self.config.identity.display_name();
        let _ = self.memory.append_daily(&format!(
            "**User**: {}\n**{agent_name}**: {}",
            truncate(user_text, 200),
            summary,
        )).await;
    }

    /// Get a reference to the memory store.
    pub fn memory(&self) -> &Arc<MemoryStore> {
        &self.memory
    }

    /// Get a reference to the session manager.
    pub fn session_mgr(&self) -> &Arc<SessionManager> {
        &self.session_mgr
    }

    /// Get a reference to the vault.
    pub fn vault(&self) -> &Arc<Vault> {
        &self.vault
    }

    /// Get a reference to the skill store.
    pub fn skills(&self) -> &Arc<SkillStore> {
        &self.skills
    }

    /// Get a reference to the cron store.
    pub fn cron(&self) -> &Arc<CronStore> {
        &self.cron
    }

    /// Get a reference to the config.
    pub fn config(&self) -> &StarpodConfig {
        &self.config
    }

    /// Start the cron scheduler as a background task.
    ///
    /// The executor callback sends the job prompt through `chat()`.
    /// Session routing depends on `JobContext.session_mode`:
    /// - `Isolated`: channel_id="scheduler", no session key (each run is its own session)
    /// - `Main`: channel_id="main", channel_session_key="main" (shared main session)
    ///
    /// An optional `notifier` is called after each job completes to deliver
    /// results to the user (e.g. via Telegram).
    /// Returns a JoinHandle for the background task.
    pub fn start_scheduler(
        self: &Arc<Self>,
        notifier: Option<starpod_cron::NotificationSender>,
    ) -> tokio::task::JoinHandle<()> {
        let cron_store = Arc::clone(&self.cron);
        let agent = Arc::clone(self);

        // Ensure heartbeat job exists
        let heartbeat_agent = Arc::clone(&agent);
        let heartbeat_store = Arc::clone(&cron_store);
        tokio::spawn(async move {
            if let Err(e) = ensure_heartbeat(&heartbeat_agent, &heartbeat_store).await {
                warn!(error = %e, "Failed to ensure heartbeat job");
            }
        });

        let executor: starpod_cron::JobExecutor = Arc::new(move |ctx: starpod_cron::JobContext| {
            let agent = Arc::clone(&agent);
            Box::pin(async move {
                // Special handling for heartbeat
                if ctx.job_name == "__heartbeat__" {
                    return execute_heartbeat(&agent, &ctx.prompt).await;
                }

                let (channel_id, session_key) = match ctx.session_mode {
                    starpod_cron::SessionMode::Isolated => {
                        ("scheduler".to_string(), None)
                    }
                    starpod_cron::SessionMode::Main => {
                        ("main".to_string(), Some("main".to_string()))
                    }
                };

                let msg = ChatMessage {
                    text: ctx.prompt,
                    user_id: Some("cron".into()),
                    channel_id: Some(channel_id),
                    channel_session_key: session_key,
                    attachments: Vec::new(),
                };
                match agent.chat(msg).await {
                    Ok(resp) => Ok(truncate(&resp.text, 500)),
                    Err(e) => Err(e.to_string()),
                }
            })
        });

        let user_tz = self.config.user.timezone.clone();
        let mut scheduler = starpod_cron::CronScheduler::new(cron_store, executor, 30, user_tz);
        if let Some(n) = notifier {
            scheduler = scheduler.with_notifier(n);
        }
        scheduler.start()
    }
}

/// Default heartbeat prompt when HEARTBEAT.md doesn't exist yet.
const DEFAULT_HEARTBEAT_PROMPT: &str =
    "You are running a heartbeat check. Review HEARTBEAT.md in your memory store for any pending tasks or instructions. If empty, do nothing.";

/// Ensure the `__heartbeat__` cron job exists.
async fn ensure_heartbeat(
    agent: &StarpodAgent,
    store: &CronStore,
) -> Result<()> {
    if store.get_job_by_name("__heartbeat__").await?.is_some() {
        return Ok(());
    }

    // Read HEARTBEAT.md from memory if it exists, otherwise use default
    let prompt = match agent.memory().read_file("HEARTBEAT.md") {
        Ok(content) if !content.trim().is_empty() => content,
        _ => DEFAULT_HEARTBEAT_PROMPT.to_string(),
    };

    // Create heartbeat job: every 30 minutes, main session
    let schedule = starpod_cron::Schedule::Cron {
        expr: "0 */30 * * * *".to_string(),
    };
    let user_tz = agent.config().user.timezone.as_deref();
    store
        .add_job_full(
            "__heartbeat__",
            &prompt,
            &schedule,
            false,
            user_tz,
            3,
            7200,
            starpod_cron::SessionMode::Main,
        )
        .await?;

    info!("Created __heartbeat__ cron job (every 30 minutes)");
    Ok(())
}

/// Execute the heartbeat: read HEARTBEAT.md and run it if non-empty.
async fn execute_heartbeat(
    agent: &StarpodAgent,
    fallback_prompt: &str,
) -> std::result::Result<String, String> {
    let prompt = match agent.memory().read_file("HEARTBEAT.md") {
        Ok(content) if !content.trim().is_empty() => content,
        _ => {
            // Nothing to do — skip silently
            return Ok("skipped".to_string());
        }
    };

    let _ = fallback_prompt; // only used as the stored prompt

    let msg = ChatMessage {
        text: prompt,
        user_id: Some("heartbeat".into()),
        channel_id: Some("main".into()),
        channel_session_key: Some("main".into()),
        attachments: Vec::new(),
    };
    match agent.chat(msg).await {
        Ok(resp) => Ok(truncate(&resp.text, 500)),
        Err(e) => Err(e.to_string()),
    }
}

/// Map a ChatMessage to a (Channel, session_key) pair for session routing.
fn resolve_channel(msg: &ChatMessage) -> (Channel, String) {
    match msg.channel_id.as_deref().unwrap_or("main") {
        "telegram" => {
            let key = msg.channel_session_key.clone()
                .or_else(|| msg.user_id.clone())
                .unwrap_or_else(|| "default".into());
            (Channel::Telegram, key)
        }
        _ => {
            // "main", "scheduler", or any unknown → explicit Main session
            let key = msg.channel_session_key.clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            (Channel::Main, key)
        }
    }
}

/// Derive a 32-byte vault key.
fn derive_vault_key(config: &StarpodConfig) -> [u8; 32] {
    let resolved = config.resolved_api_key();
    let seed = resolved
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("starpod-default-vault-key-change-me!");

    let mut key = [0u8; 32];
    let bytes = seed.as_bytes();
    for (i, byte) in bytes.iter().enumerate().take(32) {
        key[i] = *byte;
    }
    if bytes.len() < 32 {
        for i in bytes.len()..32 {
            key[i] = key[i % bytes.len()].wrapping_add(i as u8);
        }
    }
    key
}

/// Truncate a string to a maximum length, adding "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> StarpodConfig {
        StarpodConfig {
            data_dir: tmp.path().to_path_buf(),
            db_path: Some(tmp.path().join("memory.db")),
            project_root: tmp.path().to_path_buf(),
            ..StarpodConfig::default()
        }
    }

    #[tokio::test]
    async fn test_agent_construction() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Memory should be initialized
        let ctx = agent.memory().bootstrap_context().unwrap();
        assert!(ctx.contains("Aster"));

        // Vault should work
        agent.vault().set("test", "value").await.unwrap();
        assert_eq!(agent.vault().get("test").await.unwrap().as_deref(), Some("value"));

        // Skills dir should exist
        assert!(tmp.path().join("skills").exists());

        // Cron db should exist
        assert!(tmp.path().join("cron.db").exists());
    }

    #[test]
    fn test_custom_tool_definitions() {
        let defs = custom_tool_definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();

        // Memory tools
        assert!(names.contains(&"MemorySearch"));
        assert!(names.contains(&"MemoryWrite"));
        assert!(names.contains(&"MemoryAppendDaily"));
        // Vault tools
        assert!(names.contains(&"VaultGet"));
        assert!(names.contains(&"VaultSet"));
        // Skill tools
        assert!(names.contains(&"SkillActivate"));
        assert!(names.contains(&"SkillCreate"));
        assert!(names.contains(&"SkillUpdate"));
        assert!(names.contains(&"SkillDelete"));
        assert!(names.contains(&"SkillList"));
        // Cron tools
        assert!(names.contains(&"CronAdd"));
        assert!(names.contains(&"CronList"));
        assert!(names.contains(&"CronRemove"));
        assert!(names.contains(&"CronRuns"));
        assert!(names.contains(&"CronRun"));
        assert!(names.contains(&"CronUpdate"));
        assert!(names.contains(&"HeartbeatWake"));

        assert_eq!(defs.len(), 17);
    }

    #[tokio::test]
    async fn test_custom_tool_handler() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let agent = StarpodAgent::new(config).await.unwrap();

        let ctx = ToolContext {
            memory: Arc::clone(agent.memory()),
            vault: Arc::clone(agent.vault()),
            skills: Arc::clone(agent.skills()),
            cron: Arc::clone(agent.cron()),
            user_tz: None,
        };

        // Test MemorySearch
        let result = handle_custom_tool(
            &ctx,
            "MemorySearch",
            &serde_json::json!({"query": "Aster", "limit": 3}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Test SkillCreate + SkillList
        let result = handle_custom_tool(
            &ctx,
            "SkillCreate",
            &serde_json::json!({"name": "test-skill", "description": "A test skill.", "body": "Do testing."}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        let result = handle_custom_tool(
            &ctx,
            "SkillList",
            &serde_json::json!({}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("test-skill"));

        // Test CronAdd + CronList
        let result = handle_custom_tool(
            &ctx,
            "CronAdd",
            &serde_json::json!({
                "name": "test-job",
                "prompt": "Check status",
                "schedule": {"kind": "interval", "every_ms": 60000}
            }),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        let result = handle_custom_tool(
            &ctx,
            "CronList",
            &serde_json::json!({}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("test-job"));

        // Test CronAdd with new params (max_retries, session_mode)
        let result = handle_custom_tool(
            &ctx,
            "CronAdd",
            &serde_json::json!({
                "name": "advanced-job",
                "prompt": "Advanced check",
                "schedule": {"kind": "interval", "every_ms": 120000},
                "max_retries": 5,
                "timeout_secs": 300,
                "session_mode": "main"
            }),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Verify advanced-job has correct settings via CronList
        let result = handle_custom_tool(&ctx, "CronList", &serde_json::json!({})).await;
        let r = result.unwrap();
        assert!(r.content.contains("advanced-job"));
        assert!(r.content.contains("\"max_retries\": 5"));
        assert!(r.content.contains("\"session_mode\": \"main\""));

        // Test CronUpdate
        let result = handle_custom_tool(
            &ctx,
            "CronUpdate",
            &serde_json::json!({
                "name": "test-job",
                "prompt": "Updated prompt",
                "enabled": false,
                "session_mode": "main"
            }),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Test CronUpdate on nonexistent job
        let result = handle_custom_tool(
            &ctx,
            "CronUpdate",
            &serde_json::json!({"name": "no-such-job", "prompt": "x"}),
        )
        .await;
        assert!(result.is_some());
        assert!(result.unwrap().is_error);

        // Test CronRun (records a run start)
        let result = handle_custom_tool(
            &ctx,
            "CronRun",
            &serde_json::json!({"name": "test-job"}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("Manual run started"));

        // Test CronRun on nonexistent job
        let result = handle_custom_tool(
            &ctx,
            "CronRun",
            &serde_json::json!({"name": "nope"}),
        )
        .await;
        assert!(result.is_some());
        assert!(result.unwrap().is_error);

        // Test CronRuns (should show the run we just created)
        let result = handle_custom_tool(
            &ctx,
            "CronRuns",
            &serde_json::json!({"name": "test-job", "limit": 5}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("running")); // the run we started

        // Test CronRuns on nonexistent job
        let result = handle_custom_tool(
            &ctx,
            "CronRuns",
            &serde_json::json!({"name": "nope"}),
        )
        .await;
        assert!(result.is_some());
        assert!(result.unwrap().is_error);

        // Test HeartbeatWake (no heartbeat job exists, should error)
        let result = handle_custom_tool(
            &ctx,
            "HeartbeatWake",
            &serde_json::json!({"mode": "now"}),
        )
        .await;
        assert!(result.is_some());
        assert!(result.unwrap().is_error); // no __heartbeat__ job yet

        // Test HeartbeatWake with mode="next" (always succeeds)
        let result = handle_custom_tool(
            &ctx,
            "HeartbeatWake",
            &serde_json::json!({"mode": "next"}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Test HeartbeatWake with default mode (no mode specified)
        let result = handle_custom_tool(
            &ctx,
            "HeartbeatWake",
            &serde_json::json!({}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Create a heartbeat job, then test wake "now"
        ctx.cron.add_job_full(
            "__heartbeat__", "heartbeat prompt",
            &starpod_cron::Schedule::Cron { expr: "0 */30 * * * *".into() },
            false, None, 3, 7200, starpod_cron::SessionMode::Main,
        ).await.unwrap();

        let result = handle_custom_tool(
            &ctx,
            "HeartbeatWake",
            &serde_json::json!({"mode": "now", "message": "wake up!"}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("next scheduler tick"));

        // Verify heartbeat's next_run_at was set to ~now
        let hb = ctx.cron.get_job_by_name("__heartbeat__").await.unwrap().unwrap();
        let now = chrono::Utc::now().timestamp();
        assert!(hb.next_run_at.unwrap() <= now + 2);

        // Test unknown tool
        let result = handle_custom_tool(
            &ctx,
            "UnknownTool",
            &serde_json::json!({}),
        )
        .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_save_attachments() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(b"hello world");
        let attachments = vec![Attachment {
            file_name: "test.txt".into(),
            mime_type: "text/plain".into(),
            data,
        }];

        let paths = agent.save_attachments(&attachments).await;
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());

        // Verify content
        let content = tokio::fs::read(&paths[0]).await.unwrap();
        assert_eq!(content, b"hello world");

        // Verify directory structure
        assert!(paths[0].to_string_lossy().contains("downloads"));
    }

    #[tokio::test]
    async fn test_save_attachments_empty() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        let paths = agent.save_attachments(&[]).await;
        assert!(paths.is_empty());
        // downloads dir should not be created for empty attachments
        assert!(!tmp.path().join("downloads").exists());
    }

    #[tokio::test]
    async fn test_save_attachments_sanitizes_filename() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(b"data");
        let attachments = vec![Attachment {
            file_name: "../../../etc/passwd".into(),
            mime_type: "text/plain".into(),
            data,
        }];

        let paths = agent.save_attachments(&attachments).await;
        assert_eq!(paths.len(), 1);
        // The path should NOT traverse up — slashes replaced with _
        let name = paths[0].file_name().unwrap().to_string_lossy();
        assert!(!name.contains('/'));
        assert!(!name.contains(".."));
    }

    #[test]
    fn test_build_query_attachments_images() {
        let attachments = vec![
            Attachment {
                file_name: "photo.png".into(),
                mime_type: "image/png".into(),
                data: "base64data".into(),
            },
        ];
        let saved = vec![std::path::PathBuf::from("/tmp/photo.png")];

        let (query_atts, extra_text) = StarpodAgent::build_query_attachments(&attachments, &saved);
        assert_eq!(query_atts.len(), 1);
        assert_eq!(query_atts[0].mime_type, "image/png");
        assert!(extra_text.is_empty());
    }

    #[test]
    fn test_build_query_attachments_non_images() {
        let attachments = vec![
            Attachment {
                file_name: "doc.pdf".into(),
                mime_type: "application/pdf".into(),
                data: "base64data".into(),
            },
        ];
        let saved = vec![std::path::PathBuf::from("/tmp/doc.pdf")];

        let (query_atts, extra_text) = StarpodAgent::build_query_attachments(&attachments, &saved);
        assert!(query_atts.is_empty());
        assert!(extra_text.contains("doc.pdf"));
        assert!(extra_text.contains("/tmp/doc.pdf"));
    }

    #[test]
    fn test_build_query_attachments_mixed() {
        let attachments = vec![
            Attachment {
                file_name: "photo.jpg".into(),
                mime_type: "image/jpeg".into(),
                data: "imgdata".into(),
            },
            Attachment {
                file_name: "report.pdf".into(),
                mime_type: "application/pdf".into(),
                data: "pdfdata".into(),
            },
        ];
        let saved = vec![
            std::path::PathBuf::from("/tmp/photo.jpg"),
            std::path::PathBuf::from("/tmp/report.pdf"),
        ];

        let (query_atts, extra_text) = StarpodAgent::build_query_attachments(&attachments, &saved);
        assert_eq!(query_atts.len(), 1);
        assert_eq!(query_atts[0].file_name, "photo.jpg");
        assert!(extra_text.contains("report.pdf"));
        assert!(!extra_text.contains("photo.jpg"));
    }
}
