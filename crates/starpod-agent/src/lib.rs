pub mod flush;
pub mod tools;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::Local;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

use agent_sdk::{ExternalToolHandlerFn, LlmProvider, Message, ModelRegistry, Options, PermissionMode, Query, QueryAttachment};
use agent_sdk::{AnthropicProvider, GeminiProvider, OpenAiProvider};
use agent_sdk::options::{SystemPrompt, ThinkingConfig};
use starpod_core::{FollowupMode, ReasoningEffort};
use tokio::sync::mpsc;

use starpod_core::{
    Attachment, ChatMessage, ChatResponse, ChatUsage, StarpodConfig, StarpodError, Result,
    AgentConfig, ResolvedPaths,
};
use starpod_cron::CronStore;
use starpod_memory::{MemoryStore, UserMemoryView};
use starpod_session::{Channel, SessionDecision, SessionManager, UsageRecord};
use starpod_skills::SkillStore;

use crate::tools::{custom_tool_definitions, handle_custom_tool, ToolContext};

/// All custom tool names.
const CUSTOM_TOOLS: &[&str] = &[
    "MemorySearch", "MemoryWrite", "MemoryAppendDaily",
    "EnvGet",
    "FileRead", "FileWrite", "FileList", "FileDelete",
    "SkillActivate", "SkillCreate", "SkillUpdate", "SkillDelete", "SkillList",
    "CronAdd", "CronList", "CronRemove", "CronRuns",
    "CronRun", "CronUpdate", "HeartbeatWake",
    "WebSearch", "WebFetch",
    "BrowserOpen", "BrowserClick", "BrowserType",
    "BrowserExtract", "BrowserEval", "BrowserWaitFor", "BrowserClose",
];

/// The Starpod agent orchestrator.
///
/// Wires together memory, sessions, vault, skills, cron, and the agent-sdk
/// to provide a high-level `chat()` interface.
///
/// Config is wrapped in `RwLock` for hot reload support — config files can be
/// updated on disk and the agent will pick up changes on the next request.
pub struct StarpodAgent {
    memory: Arc<MemoryStore>,
    session_mgr: Arc<SessionManager>,
    skills: Arc<SkillStore>,
    cron: Arc<CronStore>,
    paths: ResolvedPaths,
    config: RwLock<StarpodConfig>,
}

impl StarpodAgent {
    /// Create a new StarpodAgent from a `StarpodConfig`.
    ///
    /// Constructs synthetic `ResolvedPaths` from the config's `db_dir` and `project_root`.
    /// Prefer `with_paths()` for workspace-aware construction.
    pub async fn new(config: StarpodConfig) -> Result<Self> {
        let agent_config = AgentConfig {
            name: config.agent_name.clone(),
            skills: Vec::new(),
            server_addr: config.server_addr.clone(),
            models: config.models.clone(),
            max_turns: config.max_turns,
            max_tokens: config.max_tokens,
            reasoning_effort: config.reasoning_effort,
            compaction_model: config.compaction_model.clone(),
            agent_name: config.agent_name.clone(),
            timezone: config.timezone.clone(),
            followup_mode: config.followup_mode,
            providers: config.providers.clone(),
            channels: config.channels.clone(),
            memory: config.memory.clone(),
            cron: config.cron.clone(),
            compaction: config.compaction.clone(),
            browser: config.browser.clone(),
            attachments: config.attachments.clone(),
            auth: config.auth.clone(),
            internet: config.internet.clone(),
            self_improve: config.self_improve,
        };

        let starpod_dir = config.db_dir.parent().unwrap_or(&config.db_dir).to_path_buf();
        let instance_root = starpod_dir.parent().unwrap_or(&starpod_dir).to_path_buf();
        let home_dir = instance_root.join("home");
        let paths = ResolvedPaths {
            mode: starpod_core::Mode::SingleAgent {
                starpod_dir: starpod_dir.clone(),
            },
            agent_toml: starpod_dir.join("config").join("agent.toml"),
            agent_home: starpod_dir.clone(),
            config_dir: starpod_dir.join("config"),
            db_dir: config.db_dir.clone(),
            skills_dir: starpod_dir.join("skills"),
            project_root: home_dir.clone(),
            instance_root,
            home_dir,
            users_dir: starpod_dir.join("users"),
            env_file: None,
        };

        Self::with_paths(agent_config, paths).await
    }

    /// Create a new StarpodAgent from an `AgentConfig` and `ResolvedPaths`.
    ///
    /// This is the workspace-aware constructor that uses resolved paths for
    /// all file locations instead of deriving them from `db_dir`.
    pub async fn with_paths(agent_config: AgentConfig, paths: ResolvedPaths) -> Result<Self> {
        // Convert AgentConfig → StarpodConfig for the config RwLock
        let config = agent_config.clone().into_starpod_config(&paths);

        // Memory: config_dir has SOUL.md + lifecycle files; agent_home for runtime data; db_dir has memory.db
        let mut memory = MemoryStore::new(&paths.agent_home, &paths.config_dir, &paths.db_dir).await?;
        memory.set_half_life_days(config.memory.half_life_days);
        memory.set_mmr_lambda(config.memory.mmr_lambda);
        memory.set_chunk_size(config.memory.chunk_size);
        memory.set_chunk_overlap(config.memory.chunk_overlap);
        memory.set_bootstrap_file_cap(config.memory.bootstrap_file_cap);

        #[cfg(feature = "embeddings")]
        if config.memory.vector_search {
            use starpod_memory::embedder::LocalEmbedder;
            memory.set_embedder(Arc::new(LocalEmbedder::new()));
            debug!("Vector search enabled with local embedder");
        }

        // Session DB in db_dir
        std::fs::create_dir_all(&paths.db_dir).map_err(|e| {
            StarpodError::Config(format!("Failed to create db dir {}: {}", paths.db_dir.display(), e))
        })?;
        let session_db = paths.db_dir.join("session.db");
        let session_mgr = SessionManager::new(&session_db).await?;

        // Skills from resolved skills_dir, with optional filter
        let skills = SkillStore::new(&paths.skills_dir)?
            .with_filter(agent_config.skills.clone());

        // Cron in db_dir
        let cron_db = paths.db_dir.join("cron.db");
        let mut cron = CronStore::new(&cron_db).await?;
        cron.set_default_max_retries(config.cron.default_max_retries);
        cron.set_default_timeout_secs(config.cron.default_timeout_secs);

        Ok(Self {
            memory: Arc::new(memory),
            session_mgr: Arc::new(session_mgr),
            skills: Arc::new(skills),
            cron: Arc::new(cron),
            paths,
            config: RwLock::new(config),
        })
    }

    /// Get the resolved paths.
    pub fn paths(&self) -> &ResolvedPaths {
        &self.paths
    }

    /// Snapshot the current config (cheap clone, no lock held after return).
    fn snapshot_config(&self) -> StarpodConfig {
        self.config.read().unwrap().clone()
    }

    /// Hot-reload the agent config. Updates per-request settings (model, provider,
    /// agent_name, etc.) and applies memory tuning parameters immediately.
    ///
    /// Settings that require restart: `server_addr`, `TELEGRAM_BOT_TOKEN` env var.
    pub fn reload_config(&self, new_config: StarpodConfig) {
        // Apply memory tuning parameters to the live MemoryStore.
        // MemoryStore is behind Arc but set_* methods need &mut, so we
        // rely on the fact that these are only called from here (single writer).
        // For now, memory params only take effect on next reindex/search.
        // TODO: expose set_* via interior mutability on MemoryStore.

        info!(
            model = %new_config.model(),
            provider = %new_config.provider(),
            agent_name = %new_config.agent_name,
            "Config reloaded",
        );

        *self.config.write().unwrap() = new_config;
    }

    /// Path to the downloads directory (lives in the project root, not inside `.starpod/`).
    fn downloads_dir(&self) -> PathBuf {
        self.snapshot_config().project_root.join("downloads")
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
    /// Also includes the saved path for all attachments so the agent knows where to find them.
    fn build_query_attachments(
        attachments: &[Attachment],
        saved_paths: &[PathBuf],
    ) -> (Vec<QueryAttachment>, String) {
        let mut query_atts = Vec::new();
        let mut extra_text = String::new();

        for (i, att) in attachments.iter().enumerate() {
            let path = saved_paths.get(i)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(save failed)".to_string());

            if att.is_image() {
                query_atts.push(QueryAttachment {
                    file_name: att.file_name.clone(),
                    mime_type: att.mime_type.clone(),
                    base64_data: att.data.clone(),
                });
                // Still tell the agent where the image was saved on disk
                extra_text.push_str(&format!(
                    "\n[Uploaded image: {} ({}) saved to: {}]",
                    att.file_name, att.mime_type, path
                ));
            } else {
                extra_text.push_str(&format!(
                    "\n[Uploaded file: {} ({}) saved to: {}]",
                    att.file_name, att.mime_type, path
                ));
            }
        }

        (query_atts, extra_text)
    }

    /// List files currently in the downloads directory (up to 20, most recent first).
    async fn list_downloads_context(&self) -> String {
        let dir = self.downloads_dir();
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(_) => return String::new(),
        };

        let mut files: Vec<(String, u64)> = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(meta) = entry.metadata().await {
                if meta.is_file() {
                    let modified = meta.modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    files.push((entry.file_name().to_string_lossy().to_string(), modified));
                }
            }
        }

        if files.is_empty() {
            return String::new();
        }

        // Sort by modified time descending (most recent first)
        files.sort_by(|a, b| b.1.cmp(&a.1));
        files.truncate(20);

        let list: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        format!(
            "\n[Files already in downloads/: {}]",
            list.join(", ")
        )
    }

    /// Build the system prompt from bootstrap context + skill catalog.
    async fn build_system_prompt(&self, session_id: &str, config: &StarpodConfig, user_id: Option<&str>, activated_skill: Option<&str>) -> Result<String> {
        let agent_name = &config.agent_name;
        let bootstrap = if let Some(uid) = user_id {
            let user_dir = self.paths.users_dir.join(uid);
            let uv = UserMemoryView::new(Arc::clone(&self.memory), user_dir).await?;
            uv.bootstrap_context(config.memory.bootstrap_file_cap)?
        } else {
            self.memory.bootstrap_context()?
        };
        let skill_catalog = self.skills.skill_catalog_excluding(activated_skill)?;
        let date_str = Local::now().format("%A, %B %d, %Y at %H:%M").to_string();
        let tz_str = config.resolved_timezone().unwrap_or_else(|| "UTC".to_string());

        let mut prompt = format!(
            "You are {agent_name}, a personal AI assistant.\n\n{bootstrap}\n\n---\n\
             Current date/time: {date_str}\nTimezone: {tz_str}\nSession ID: {session_id}\n\
             Home directory: ~/\n\
             Working directory: ~/\n\n\
             You have access to memory tools (MemorySearch, MemoryWrite, MemoryAppendDaily), \
             environment tools (EnvGet), file tools (FileRead, FileWrite, FileList, FileDelete), \
             skill tools (SkillActivate, SkillCreate, SkillUpdate, SkillDelete, SkillList), \
             scheduling tools (CronAdd, CronList, CronRemove, CronRuns, CronRun, CronUpdate, HeartbeatWake), \
             and browser tools (BrowserOpen, BrowserClick, BrowserType, BrowserExtract, BrowserEval, BrowserWaitFor, BrowserClose).\n\
             Browser tools let you automate web tasks: BrowserOpen navigates to a URL (auto-launches a browser process), \
             BrowserExtract gets text content, BrowserClick/BrowserType interact with elements by CSS selector, \
             BrowserEval runs JavaScript, BrowserWaitFor waits for a condition (URL change, element, or JS expression), \
             and BrowserClose ends the session.\n\
             You can read image files (png, jpg, gif, webp) with the Read tool — the image will be loaded \
             directly into the conversation so you can see and analyze it. For other file types like CSV or \
             PDF, use Python via the Bash tool.\n\n\
             IMPORTANT — two separate domains of information:\n\
             • Your personal knowledge, memory, soul, and user profile are accessed ONLY through \
             MemorySearch (to query) and MemoryWrite/MemoryAppendDaily (to persist). Never try to \
             access internal system files directly — they are not visible to you.\n\
             • Your workspace is ~/ (the home directory). Use FileRead, FileWrite, FileList, FileDelete, \
             Read, Glob, Grep, and Bash to explore and work with files here.\n\
             • Files uploaded by the user (from any channel: Telegram, web, API) are saved to ~/downloads/. \
             When the user references a previously uploaded file, always check this directory first.\n\
             You may ONLY access files within your home directory ~/. \
             Do not read, write, or execute anything outside this boundary.\n\
             IMPORTANT: Always create files and run commands within ~/, never in /tmp or other external directories.",
        );

        // ── Memory nudging ────────────────────────────────────────────
        prompt.push_str(
            "\n\n--- MEMORY GUIDANCE ---\n\
             Proactively persist knowledge — do not wait to be asked:\n\
             • When the user corrects you or says \"remember this\" / \"don't do that again\" \
             → save to USER.md via MemoryWrite so you never repeat the mistake.\n\
             • When the user shares a preference, habit, name, or personal detail \
             → update USER.md.\n\
             • When you discover an environment fact, API quirk, or non-obvious workflow \
             → append to MEMORY.md.\n\
             • After every substantive conversation, append a brief summary to the daily log \
             via MemoryAppendDaily — what was discussed, decisions made, and outcomes.\n\
             Prioritize what reduces future user effort — the most valuable memory is one \
             that prevents the user from having to correct or remind you again.\n\
             Do NOT save: task progress, TODO lists, or information that only matters right now.",
        );

        // ── Self-improve guidance (skill auto-creation + improvement) ─
        if config.self_improve {
            prompt.push_str(
                "\n\n--- SELF-IMPROVE MODE (beta) ---\n\
                 You have self-improvement enabled. This means:\n\n\
                 SKILL AUTO-CREATION:\n\
                 After completing a complex task (roughly 5+ tool calls), fixing a tricky error, \
                 or discovering a non-trivial workflow, save the approach as a skill with SkillCreate \
                 so you can reuse it next time. Include clear steps, context on when to use it, \
                 and any pitfalls you encountered. Do not create skills for trivial or one-off tasks.\n\n\
                 SKILL SELF-IMPROVEMENT:\n\
                 When using a skill and finding it outdated, incomplete, or wrong, update it \
                 immediately with SkillUpdate — don't wait to be asked. Skills that aren't \
                 maintained become liabilities. If a skill's instructions led you astray, \
                 fix them so the next invocation succeeds.\n\n\
                 SKILL ENVIRONMENT DECLARATIONS:\n\
                 When creating or updating skills that interact with external APIs, declare their \
                 environment requirements using the `env` parameter — `secrets` for API keys/tokens \
                 (e.g. GITHUB_TOKEN, WEATHER_API_KEY), `variables` for configurable settings with \
                 defaults (e.g. DEFAULT_ORG, MAX_RESULTS). Use UPPER_SNAKE_CASE for key names. \
                 Only declare env when the skill genuinely needs external access — do not add env \
                 to skills that only use built-in tools.",
            );
        }

        // Inject skill catalog (progressive disclosure — names + descriptions only).
        // The activated skill (if any) is already excluded by skill_catalog_excluding().
        if !skill_catalog.is_empty() {
            prompt.push_str("\n\nThe following skills provide specialized instructions for specific tasks.\n\
                             When a task matches a skill's description, call the SkillActivate tool \
                             with the skill's name to load its full instructions before proceeding.\n\n");
            prompt.push_str(&skill_catalog);
        }

        Ok(prompt)
    }

}

/// Append execution-context block to the system prompt when the message
/// originates from a scheduled job (cron or heartbeat) so the LLM knows
/// to act directly rather than re-scheduling.
///
/// Detection is based on `channel_id` ("scheduler") and `user_id` ("heartbeat")
/// since cron jobs now use the actual user_id from `JobContext`, while heartbeat
/// still uses a synthetic user_id.
fn append_execution_context(prompt: &mut String, channel_id: Option<&str>, user_id: Option<&str>) {
    if user_id == Some("heartbeat") {
        prompt.push_str(
            "\n\n--- EXECUTION CONTEXT ---\n\
             You are executing a HEARTBEAT (periodic background check). The message below \
             comes from HEARTBEAT.md. Carry out the instructions directly. Do NOT schedule \
             new cron jobs unless the heartbeat instructions explicitly ask you to.",
        );
    } else if channel_id == Some("scheduler") || user_id == Some("cron") {
        prompt.push_str(
            "\n\n--- EXECUTION CONTEXT ---\n\
             You are executing a SCHEDULED CRON JOB right now. The message below is the \
             cron job's prompt — carry out the instruction directly. Do NOT schedule \
             another reminder or cron job unless the prompt explicitly asks you to. \
             If the task is to remind or notify the user, deliver the reminder content \
             directly in your response.",
        );
    }
}

impl StarpodAgent {
    /// Map reasoning effort config to ThinkingConfig.
    fn thinking_config(config: &StarpodConfig) -> Option<ThinkingConfig> {
        config.reasoning_effort.map(|effort| match effort {
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

    /// Build the LLM provider for the default (or given) provider.
    fn build_provider(&self, config: &StarpodConfig) -> Result<Box<dyn LlmProvider>> {
        self.build_provider_for(config.provider(), config)
    }

    /// Build an LLM provider for the given provider name using config for API key / base URL.
    fn build_provider_for(&self, provider_name: &str, config: &StarpodConfig) -> Result<Box<dyn LlmProvider>> {
        let api_key = config.resolved_provider_api_key(provider_name)
            .ok_or_else(|| StarpodError::Config(format!(
                "No API key found for provider '{}'. Set it in config.toml or via environment variable.",
                provider_name
            )))?;
        let base_url = config.resolved_provider_base_url(provider_name)
            .ok_or_else(|| StarpodError::Config(format!(
                "Unknown provider: '{}'",
                provider_name
            )))?;

        let pricing = self.load_model_registry();

        let provider: Box<dyn LlmProvider> = match provider_name {
            "anthropic" => Box::new(
                AnthropicProvider::new(api_key, base_url).with_pricing(pricing)
            ),
            "gemini" => Box::new(
                GeminiProvider::with_base_url(api_key, base_url).with_pricing(pricing)
            ),
            // OpenAI-compatible providers
            "openai" | "groq" | "deepseek" | "openrouter" | "ollama" => {
                Box::new(
                    OpenAiProvider::with_base_url(api_key, base_url, provider_name).with_pricing(pricing)
                )
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

    /// Load the pricing registry: embedded defaults + optional config override.
    fn load_model_registry(&self) -> Arc<ModelRegistry> {
        let mut registry = ModelRegistry::with_defaults();

        let pricing_path = self.paths.config_dir.join("models.toml");
        if pricing_path.exists() {
            match std::fs::read_to_string(&pricing_path) {
                Ok(contents) => match ModelRegistry::from_toml(&contents) {
                    Ok(overrides) => {
                        debug!(path = %pricing_path.display(), "loaded pricing overrides");
                        registry.merge(overrides);
                    }
                    Err(e) => {
                        warn!(path = %pricing_path.display(), error = %e, "failed to parse pricing.toml, using defaults");
                    }
                },
                Err(e) => {
                    warn!(path = %pricing_path.display(), error = %e, "failed to read pricing.toml, using defaults");
                }
            }
        }

        Arc::new(registry)
    }

    /// Build the pre-compaction handler that saves key facts before context is discarded.
    ///
    /// When `memory_flush` is enabled, runs a silent agentic LLM turn to intelligently
    /// persist important information. Otherwise falls back to a simple text dump.
    ///
    /// When a `user_id` is provided, daily logs are routed to the per-user directory
    /// (`users/{id}/memory/`) via `UserMemoryView`, falling back to the agent-level store.
    async fn build_pre_compact_handler(
        &self,
        config: &StarpodConfig,
        user_id: Option<&str>,
    ) -> agent_sdk::PreCompactHandlerFn {
        let memory = Arc::clone(&self.memory);

        // Build user view early so all fallback paths can use it
        let user_view_for_fallback: Option<Arc<starpod_memory::UserMemoryView>> = match user_id {
            Some(uid) => {
                let user_dir = self.paths.users_dir.join(uid);
                match starpod_memory::UserMemoryView::new(Arc::clone(&memory), user_dir).await {
                    Ok(uv) => Some(Arc::new(uv)),
                    Err(e) => {
                        warn!(error = %e, "Failed to create UserMemoryView for pre-compact fallback");
                        None
                    }
                }
            }
            None => None,
        };

        if !config.compaction.memory_flush {
            // Legacy fallback: dumb text dump
            return Box::new(move |messages: Vec<agent_sdk::client::ApiMessage>| {
                let memory = Arc::clone(&memory);
                let user_view = user_view_for_fallback.clone();
                Box::pin(async move {
                    let mut text_parts: Vec<String> = Vec::new();
                    for msg in &messages {
                        for block in &msg.content {
                            if let agent_sdk::client::ApiContentBlock::Text { text, .. } = block {
                                text_parts.push(text.clone());
                            }
                        }
                    }
                    if text_parts.is_empty() {
                        return;
                    }
                    let combined = text_parts.join("\n");
                    let truncated = if combined.len() > 2000 {
                        let mut end = 2000;
                        while end > 0 && !combined.is_char_boundary(end) { end -= 1; }
                        format!("{}...", &combined[..end])
                    } else {
                        combined
                    };
                    let entry = format!("## Pre-compaction save\n{}", truncated);
                    let result = if let Some(ref uv) = user_view {
                        uv.append_daily(&entry).await
                    } else {
                        memory.append_daily(&entry).await
                    };
                    if let Err(e) = result {
                        warn!("Failed to save pre-compaction context: {}", e);
                    }
                })
            });
        }

        // Agentic flush: build provider and user view for the closure
        let flush_model = config.compaction.flush_model.clone()
            .or_else(|| config.compaction_model.clone())
            .unwrap_or_else(|| config.model().to_string());

        let provider: Arc<dyn LlmProvider> = match self.build_provider(config) {
            Ok(p) => Arc::from(p),
            Err(e) => {
                warn!(error = %e, "Failed to build provider for memory flush, falling back to dumb dump");
                return Box::new(move |messages: Vec<agent_sdk::client::ApiMessage>| {
                    let memory = Arc::clone(&memory);
                    let user_view = user_view_for_fallback.clone();
                    Box::pin(async move {
                        let mut parts: Vec<String> = Vec::new();
                        for msg in &messages {
                            for block in &msg.content {
                                if let agent_sdk::client::ApiContentBlock::Text { text, .. } = block {
                                    parts.push(text.clone());
                                }
                            }
                        }
                        if !parts.is_empty() {
                            let combined = parts.join("\n");
                            let truncated = if combined.len() > 2000 {
                                let mut end = 2000;
                                while end > 0 && !combined.is_char_boundary(end) { end -= 1; }
                                format!("{}...", &combined[..end])
                            } else { combined };
                            let result = if let Some(ref uv) = user_view {
                                uv.append_daily(&format!("## Pre-compaction save\n{}", truncated)).await
                            } else {
                                memory.append_daily(&format!("## Pre-compaction save\n{}", truncated)).await
                            };
                            if let Err(e) = result {
                                warn!("Failed to save pre-compaction context: {}", e);
                            }
                        }
                    })
                });
            }
        };

        // Build optional user view (async)
        let user_view: Option<Arc<starpod_memory::UserMemoryView>> = match user_id {
            Some(uid) => {
                let user_dir = self.paths.users_dir.join(uid);
                match starpod_memory::UserMemoryView::new(Arc::clone(&memory), user_dir).await {
                    Ok(uv) => Some(Arc::new(uv)),
                    Err(e) => {
                        warn!(error = %e, "Failed to create UserMemoryView for flush");
                        None
                    }
                }
            }
            None => None,
        };

        Box::new(move |messages: Vec<agent_sdk::client::ApiMessage>| {
            let provider = Arc::clone(&provider);
            let memory = Arc::clone(&memory);
            let user_view = user_view.clone();
            let flush_model = flush_model.clone();
            Box::pin(async move {
                flush::run_memory_flush(
                    provider.as_ref(),
                    &flush_model,
                    &messages,
                    &memory,
                    user_view.as_deref(),
                ).await;
            })
        })
    }

    /// Build the external tool handler closure.
    async fn build_tool_handler(&self, config: &StarpodConfig, user_id: Option<&str>) -> ExternalToolHandlerFn {
        let user_view = match user_id {
            Some(uid) => {
                let user_dir = self.paths.users_dir.join(uid);
                match UserMemoryView::new(Arc::clone(&self.memory), user_dir).await {
                    Ok(uv) => Some(uv),
                    Err(e) => {
                        warn!(error = %e, user_id = uid, "Failed to create UserMemoryView");
                        None
                    }
                }
            }
            None => None,
        };

        let brave_api_key = std::env::var("BRAVE_API_KEY").ok();

        let ctx = Arc::new(ToolContext {
            memory: Arc::clone(&self.memory),
            user_view,
            skills: Arc::clone(&self.skills),
            cron: Arc::clone(&self.cron),
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: config.browser.enabled,
            browser_cdp_url: config.browser.cdp_url.clone(),
            user_tz: config.resolved_timezone(),
            home_dir: self.paths.home_dir.clone(),
            agent_home: self.paths.agent_home.clone(),
            user_id: user_id.map(|s| s.to_string()),
            http_client: reqwest::Client::new(),
            internet: config.internet.clone(),
            brave_api_key,
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
        let config = self.snapshot_config();

        // Step 1: Resolve session via channel routing
        let (channel, key) = resolve_channel(&message);
        let gap = config.channel_gap_minutes(channel.as_str());
        let user_id = message.user_id.as_deref().unwrap_or("admin");
        let (session_id, is_resuming) = match self.session_mgr.resolve_session_for_user(&channel, &key, gap, user_id).await? {
            SessionDecision::Continue(id) => {
                debug!(session_id = %id, channel = %channel.as_str(), "Continuing existing session");
                (id, true)
            }
            SessionDecision::New { closed_session_id } => {
                // Export the closed session's transcript to memory (in background)
                if let Some(ref closed_id) = closed_session_id {
                    self.export_session_to_memory(closed_id).await;
                }
                let id = self.session_mgr.create_session_full(
                    &channel,
                    &key,
                    message.user_id.as_deref().unwrap_or("admin"),
                    message.triggered_by.as_deref(),
                ).await?;
                debug!(session_id = %id, channel = %channel.as_str(), "Created new session");
                (id, false)
            }
        };
        self.session_mgr.touch_session(&session_id).await?;
        let _ = self.session_mgr.set_title_if_empty(&session_id, &message.text).await;

        // Step 2: Save attachments to downloads/ and build query attachments
        let saved_paths = self.save_attachments(&message.attachments).await;
        let (query_atts, mut extra_text) =
            Self::build_query_attachments(&message.attachments, &saved_paths);

        // When files are uploaded, also list existing downloads for context
        if !message.attachments.is_empty() {
            let dl_ctx = self.list_downloads_context().await;
            extra_text.push_str(&dl_ctx);
        }

        // Append upload context to prompt
        let prompt = if extra_text.is_empty() {
            message.text.clone()
        } else {
            format!("{}{}", message.text, extra_text)
        };

        // Step 3: Build system prompt
        let mut system_prompt = self.build_system_prompt(&session_id, &config, message.user_id.as_deref(), None).await?;

        append_execution_context(&mut system_prompt, message.channel_id.as_deref(), message.user_id.as_deref());

        // Step 4: Resolve model (may be overridden per-message) and build provider
        let (resolved_provider, resolved_model) = config
            .resolve_model(message.model.as_deref())
            .map_err(|e| StarpodError::Config(e))?;
        let provider = self.build_provider_for(&resolved_provider, &config)?;

        let mut builder = Options::builder()
            .allowed_tools(Self::allowed_tools())
            .system_prompt(SystemPrompt::Custom(system_prompt))
            .permission_mode(PermissionMode::BypassPermissions)
            .model(&resolved_model)
            .max_turns(config.max_turns)
            .max_tokens(config.max_tokens)
            .context_budget(config.compaction.context_budget)
            .summary_max_tokens(config.compaction.summary_max_tokens)
            .min_keep_messages(config.compaction.min_keep_messages)
            .max_tool_result_bytes(config.compaction.max_tool_result_bytes)
            .prune_threshold_pct(config.compaction.prune_threshold_pct)
            .prune_tool_result_max_chars(config.compaction.prune_tool_result_max_chars)
            .external_tool_handler(self.build_tool_handler(&config, message.user_id.as_deref()).await)
            .pre_compact_handler(self.build_pre_compact_handler(&config, message.user_id.as_deref()).await)
            .custom_tools(custom_tool_definitions())
            .attachments(query_atts)
            .provider(provider)
            .cwd(config.project_root.to_string_lossy().to_string())
            .additional_directories(vec![])
            .hook_dirs(vec![config.db_dir.join("hooks")]);

        // Resume existing session to load conversation history, or set ID for new ones
        if is_resuming {
            builder = builder.resume(session_id.clone());
        } else {
            builder = builder.session_id(session_id.clone());
        }

        // Compaction model: "provider/model" format
        if let Some(ref cm) = config.compaction_model {
            if let Some((cp, cm_name)) = starpod_core::parse_model_spec(cm) {
                builder = builder.compaction_model(cm_name);
                if cp != resolved_provider {
                    match self.build_provider_for(cp, &config) {
                        Ok(p) => { builder = builder.compaction_provider(p); }
                        Err(e) => {
                            tracing::warn!(provider = cp, error = %e, "Failed to build compaction provider, falling back to primary");
                        }
                    }
                }
            }
        }

        if let Some(key) = config.resolved_api_key() {
            builder = builder.api_key(key);
        }
        if let Some(thinking) = Self::thinking_config(&config) {
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
                                model: resolved_model.clone(),
                                user_id: message.user_id.clone().unwrap_or_else(|| "admin".into()),
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

        // Step 5: Save messages to session history
        let _ = self.session_mgr.save_message(&session_id, "user", &message.text).await;
        if !result_text.is_empty() {
            let _ = self.session_mgr.save_message(&session_id, "assistant", &result_text).await;
        }

        // Step 6: Append summary to daily log (opt-in, off by default when memory flush is enabled)
        if config.memory.auto_log {
            let summary = truncate(&result_text, 200);
            let agent_name = &config.agent_name;
            let entry = format!(
                "**User**: {}\n**{agent_name}**: {}",
                truncate(&message.text, 200),
                summary,
            );
            let _ = self.append_daily_for_user(message.user_id.as_deref(), &entry).await;
        }

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
        let config = self.snapshot_config();

        let (channel, key) = resolve_channel(message);
        let gap = config.channel_gap_minutes(channel.as_str());
        let user_id = message.user_id.as_deref().unwrap_or("admin");
        let (session_id, is_resuming) = match self.session_mgr.resolve_session_for_user(&channel, &key, gap, user_id).await? {
            SessionDecision::Continue(id) => {
                debug!(session_id = %id, channel = %channel.as_str(), "Continuing existing session");
                (id, true)
            }
            SessionDecision::New { closed_session_id } => {
                if let Some(ref closed_id) = closed_session_id {
                    self.export_session_to_memory(closed_id).await;
                }
                let id = self.session_mgr.create_session_full(
                    &channel,
                    &key,
                    message.user_id.as_deref().unwrap_or("admin"),
                    message.triggered_by.as_deref(),
                ).await?;
                debug!(session_id = %id, channel = %channel.as_str(), "Created new session");
                (id, false)
            }
        };
        self.session_mgr.touch_session(&session_id).await?;
        let _ = self.session_mgr.set_title_if_empty(&session_id, &message.text).await;

        // Save attachments and build query attachments
        let saved_paths = self.save_attachments(&message.attachments).await;
        let (query_atts, mut extra_text) =
            Self::build_query_attachments(&message.attachments, &saved_paths);

        // When files are uploaded, also list existing downloads for context
        if !message.attachments.is_empty() {
            let dl_ctx = self.list_downloads_context().await;
            extra_text.push_str(&dl_ctx);
        }

        let mut prompt = if extra_text.is_empty() {
            message.text.clone()
        } else {
            format!("{}{}", message.text, extra_text)
        };

        // Slash-command skill activation: /skill-name [args]
        // When the message starts with /<name>, activate the skill inline so the
        // LLM executes it immediately without an extra SkillActivate round-trip.
        let mut activated_skill: Option<String> = None;
        if let Some(skill_name) = message.text.strip_prefix('/') {
            let skill_name = skill_name.split_whitespace().next().unwrap_or("");
            if !skill_name.is_empty() {
                if let Ok(Some(content)) = self.skills.activate_skill(skill_name) {
                    let user_args = message.text[1 + skill_name.len()..].trim();
                    let execute_preamble = format!(
                        "The user invoked the /{skill_name} skill{}. \
                         IMPORTANT: Execute the skill instructions below immediately — do NOT ask \
                         clarifying questions, do NOT summarize the skill, do NOT ask for confirmation. \
                         Start executing the first step right now. Use any defaults specified in the \
                         skill when the user has not provided explicit overrides.",
                        if user_args.is_empty() {
                            String::new()
                        } else {
                            format!(" with the following input: {user_args}")
                        }
                    );
                    prompt = format!("{execute_preamble}\n\n{content}");
                    activated_skill = Some(skill_name.to_string());
                    debug!(skill = %skill_name, "Slash-command skill activated inline");
                }
            }
        }

        let system_prompt = self.build_system_prompt(&session_id, &config, message.user_id.as_deref(), activated_skill.as_deref()).await?;

        // Resolve model (may be overridden per-message)
        let (resolved_provider, resolved_model) = config
            .resolve_model(message.model.as_deref())
            .map_err(|e| StarpodError::Config(e))?;
        let provider = self.build_provider_for(&resolved_provider, &config)?;

        // Create the followup channel — sender goes to caller, receiver to the agent loop
        let (followup_tx, followup_rx) = mpsc::unbounded_channel::<String>();

        let mut builder = Options::builder()
            .allowed_tools(Self::allowed_tools())
            .system_prompt(SystemPrompt::Custom(system_prompt))
            .permission_mode(PermissionMode::BypassPermissions)
            .model(&resolved_model)
            .max_turns(config.max_turns)
            .max_tokens(config.max_tokens)
            .context_budget(config.compaction.context_budget)
            .summary_max_tokens(config.compaction.summary_max_tokens)
            .min_keep_messages(config.compaction.min_keep_messages)
            .max_tool_result_bytes(config.compaction.max_tool_result_bytes)
            .prune_threshold_pct(config.compaction.prune_threshold_pct)
            .prune_tool_result_max_chars(config.compaction.prune_tool_result_max_chars)
            .external_tool_handler(self.build_tool_handler(&config, message.user_id.as_deref()).await)
            .pre_compact_handler(self.build_pre_compact_handler(&config, message.user_id.as_deref()).await)
            .custom_tools(custom_tool_definitions())
            .followup_rx(followup_rx)
            .attachments(query_atts)
            .provider(provider)
            .cwd(config.project_root.to_string_lossy().to_string())
            .additional_directories(vec![])
            .hook_dirs(vec![config.db_dir.join("hooks")])
            .include_partial_messages(true);

        // Resume existing session to load conversation history, or set ID for new ones
        if is_resuming {
            builder = builder.resume(session_id.clone());
        } else {
            builder = builder.session_id(session_id.clone());
        }

        // Compaction model: "provider/model" format
        if let Some(ref cm) = config.compaction_model {
            if let Some((cp, cm_name)) = starpod_core::parse_model_spec(cm) {
                builder = builder.compaction_model(cm_name);
                if cp != resolved_provider {
                    match self.build_provider_for(cp, &config) {
                        Ok(p) => { builder = builder.compaction_provider(p); }
                        Err(e) => {
                            tracing::warn!(provider = cp, error = %e, "Failed to build compaction provider, falling back to primary");
                        }
                    }
                }
            }
        }

        if let Some(key) = config.resolved_api_key() {
            builder = builder.api_key(key);
        }
        if let Some(thinking) = Self::thinking_config(&config) {
            builder = builder.thinking(thinking);
        }

        let options = builder.build();

        let stream = agent_sdk::query(&prompt, options);
        Ok((stream, session_id, followup_tx))
    }

    /// Get the configured followup mode.
    pub fn followup_mode(&self) -> FollowupMode {
        self.snapshot_config().followup_mode
    }

    /// Finalize a streaming chat — record usage and append daily log.
    pub async fn finalize_chat(
        &self,
        session_id: &str,
        user_text: &str,
        result_text: &str,
        result: &agent_sdk::ResultMessage,
        user_id: Option<&str>,
    ) {
        let config = self.snapshot_config();

        if let Some(u) = &result.usage {
            let _ = self.session_mgr.record_usage(
                session_id,
                &UsageRecord {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cache_read: u.cache_read_input_tokens,
                    cache_write: u.cache_creation_input_tokens,
                    cost_usd: result.total_cost_usd,
                    model: config.model().to_string(),
                    user_id: user_id.unwrap_or("admin").to_string(),
                },
                result.num_turns,
            ).await;
        }

        if config.memory.auto_log {
            let summary = truncate(result_text, 200);
            let agent_name = &config.agent_name;
            let entry = format!(
                "**User**: {}\n**{agent_name}**: {}",
                truncate(user_text, 200),
                summary,
            );
            let _ = self.append_daily_for_user(user_id, &entry).await;
        }
    }

    /// Append to daily log via user view when a user_id is present, falling back to agent-level store.
    async fn append_daily_for_user(&self, user_id: Option<&str>, text: &str) -> starpod_core::Result<()> {
        if let Some(uid) = user_id {
            let user_dir = self.paths.users_dir.join(uid);
            if let Ok(uv) = UserMemoryView::new(Arc::clone(&self.memory), user_dir).await {
                return uv.append_daily(text).await;
            }
        }
        self.memory.append_daily(text).await
    }

    /// Export a closed session's transcript to `knowledge/sessions/` for long-term recall.
    ///
    /// Formats all messages as markdown and writes to the memory store so they
    /// become searchable. Runs in the background to avoid blocking the chat flow.
    async fn export_session_to_memory(&self, session_id: &str) {
        let config = self.snapshot_config();
        if !config.memory.export_sessions {
            return;
        }

        let meta = match self.session_mgr.get_session(session_id).await {
            Ok(Some(m)) => m,
            _ => return,
        };

        let messages = match self.session_mgr.get_messages(session_id).await {
            Ok(msgs) if !msgs.is_empty() => msgs,
            _ => return,
        };

        // Build a slug from the title for the filename
        let title = meta.title.as_deref().unwrap_or("untitled");
        let slug: String = title
            .chars()
            .take(50)
            .map(|c| if c.is_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        let id_prefix = &session_id[..8.min(session_id.len())];
        let filename = format!("memory/sessions/{slug}-{id_prefix}.md");

        // Format the transcript
        let mut transcript = format!(
            "# Session: {}\n\n\
             - **Date**: {}\n\
             - **Channel**: {}\n\
             - **Messages**: {}\n",
            title, &meta.created_at[..10.min(meta.created_at.len())],
            meta.channel, meta.message_count,
        );
        if let Some(ref summary) = meta.summary {
            transcript.push_str(&format!("- **Summary**: {}\n", summary));
        }
        transcript.push_str("\n---\n\n");

        for msg in &messages {
            let role_label = match msg.role.as_str() {
                "user" => "User",
                "assistant" => &config.agent_name,
                other => other,
            };
            transcript.push_str(&format!("**{}**: {}\n\n", role_label, msg.content));
        }

        // Route per-user when user_id is present (non-empty and not synthetic)
        let write_result = if !meta.user_id.is_empty() && meta.user_id != "heartbeat" && meta.user_id != "cron" {
            let user_dir = self.paths.users_dir.join(&meta.user_id);
            match UserMemoryView::new(Arc::clone(&self.memory), user_dir).await {
                Ok(uv) => uv.write_file(&filename, &transcript).await,
                Err(e) => Err(e),
            }
        } else {
            self.memory.write_file(&filename, &transcript).await
        };

        if let Err(e) = write_result {
            warn!(error = %e, session_id, "Failed to export session transcript to memory");
        } else {
            debug!(session_id, filename, "Exported session transcript to memory");
        }
    }

    /// Get a reference to the memory store.
    pub fn memory(&self) -> &Arc<MemoryStore> {
        &self.memory
    }

    /// Get a reference to the session manager.
    pub fn session_mgr(&self) -> &Arc<SessionManager> {
        &self.session_mgr
    }

    /// Get a reference to the skill store.
    pub fn skills(&self) -> &Arc<SkillStore> {
        &self.skills
    }

    /// Get a reference to the cron store.
    pub fn cron(&self) -> &Arc<CronStore> {
        &self.cron
    }

    /// Get a snapshot of the current config.
    pub fn config(&self) -> StarpodConfig {
        self.snapshot_config()
    }

    /// Run startup lifecycle prompts (boot + bootstrap) in the background.
    ///
    /// See [`run_lifecycle_prompts`] for details.
    pub fn run_lifecycle(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let agent = Arc::clone(self);
        tokio::spawn(async move {
            run_lifecycle_prompts(&agent).await;
        })
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
                    user_id: ctx.user_id.or(Some("cron".into())),
                    channel_id: Some(channel_id),
                    channel_session_key: session_key,
                    attachments: Vec::new(),
                    triggered_by: Some(ctx.job_name.clone()),
                    model: None,
                };
                match agent.chat(msg).await {
                    Ok(resp) => Ok(starpod_cron::JobResult {
                        session_id: resp.session_id,
                        summary: truncate(&resp.text, 500),
                    }),
                    Err(e) => Err(e.to_string()),
                }
            })
        });

        let config = self.snapshot_config();
        let user_tz = config.resolved_timezone();
        let mut scheduler = starpod_cron::CronScheduler::new(cron_store, executor, 30, user_tz)
            .with_max_concurrent_runs(config.cron.max_concurrent_runs as u32);
        if let Some(n) = notifier {
            scheduler = scheduler.with_notifier(n);
        }
        scheduler.start()
    }
}

/// Run startup lifecycle prompts (boot + bootstrap).
///
/// Called once after the server starts and the scheduler is running.
/// - **Boot** (`BOOT.md`): runs on every server start if non-empty.
/// - **Bootstrap** (`BOOTSTRAP.md`): runs once on first init if non-empty,
///   then the file is cleared so it never runs again.
///
/// Both fire the `Setup` hook event with the appropriate trigger so external
/// hooks can also react.
async fn run_lifecycle_prompts(agent: &Arc<StarpodAgent>) {
    // --- Bootstrap (first-init only) ---
    if agent.memory().has_bootstrap() {
        info!("Running bootstrap (first-init lifecycle prompt)");
        match agent.memory().read_file("BOOTSTRAP.md") {
            Ok(prompt) if !prompt.trim().is_empty() => {
                let msg = ChatMessage {
                    text: prompt,
                    user_id: Some("bootstrap".into()),
                    channel_id: Some("main".into()),
                    channel_session_key: Some("main".into()),
                    attachments: Vec::new(),
                    triggered_by: None,
                    model: None,
                };
                match agent.chat(msg).await {
                    Ok(resp) => {
                        info!(response_len = resp.text.len(), "Bootstrap completed");
                        // Clear BOOTSTRAP.md so it only runs once
                        if let Err(e) = agent.memory().clear_bootstrap() {
                            warn!(error = %e, "Failed to clear BOOTSTRAP.md after execution");
                        }
                    }
                    Err(e) => warn!(error = %e, "Bootstrap prompt failed"),
                }
            }
            _ => {}
        }
    }

    // --- Boot (every server start) ---
    match agent.memory().read_file("BOOT.md") {
        Ok(prompt) if !prompt.trim().is_empty() => {
            info!("Running boot lifecycle prompt");
            let msg = ChatMessage {
                text: prompt,
                user_id: Some("boot".into()),
                channel_id: Some("main".into()),
                channel_session_key: Some("main".into()),
                attachments: Vec::new(),
                triggered_by: None,
                model: None,
            };
            match agent.chat(msg).await {
                Ok(resp) => info!(response_len = resp.text.len(), "Boot completed"),
                Err(e) => warn!(error = %e, "Boot prompt failed"),
            }
        }
        _ => {
            debug!("BOOT.md is empty or missing — skipping boot prompt");
        }
    }
}

/// Ensure the `__heartbeat__` cron job exists.
///
/// The heartbeat is opt-in: the job is only created if HEARTBEAT.md exists
/// and has content. If the user later clears HEARTBEAT.md, execution will
/// be silently skipped (see `execute_heartbeat`).
async fn ensure_heartbeat(
    agent: &StarpodAgent,
    store: &CronStore,
) -> Result<()> {
    if store.get_job_by_name("__heartbeat__").await?.is_some() {
        return Ok(());
    }

    // Only create the heartbeat job if HEARTBEAT.md has actual content.
    // This makes the feature opt-in: no HEARTBEAT.md → no heartbeat job.
    let prompt = match agent.memory().read_file("HEARTBEAT.md") {
        Ok(content) if !content.trim().is_empty() => content,
        _ => {
            debug!("HEARTBEAT.md is empty or missing — skipping heartbeat job creation");
            return Ok(());
        }
    };

    let config = agent.config();
    let interval = config.cron.heartbeat_interval_minutes.max(1);
    let schedule = starpod_cron::Schedule::Cron {
        expr: format!("0 */{interval} * * * *"),
    };
    let resolved_tz = config.resolved_timezone();
    let user_tz = resolved_tz.as_deref();
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
            None, // agent-level heartbeat
        )
        .await?;

    info!(interval_minutes = interval, "Created __heartbeat__ cron job");
    Ok(())
}

/// Execute the heartbeat: read HEARTBEAT.md and run it if non-empty.
async fn execute_heartbeat(
    agent: &StarpodAgent,
    fallback_prompt: &str,
) -> std::result::Result<starpod_cron::JobResult, String> {
    let prompt = match agent.memory().read_file("HEARTBEAT.md") {
        Ok(content) if !content.trim().is_empty() => content,
        _ => {
            // Nothing to do — skip silently
            return Ok(starpod_cron::JobResult {
                session_id: String::new(),
                summary: "skipped".to_string(),
            });
        }
    };

    let _ = fallback_prompt; // only used as the stored prompt

    let msg = ChatMessage {
        text: prompt,
        user_id: Some("heartbeat".into()),
        channel_id: Some("main".into()),
        channel_session_key: Some("main".into()),
        attachments: Vec::new(),
        triggered_by: Some("__heartbeat__".into()),
        model: None,
    };
    match agent.chat(msg).await {
        Ok(resp) => Ok(starpod_cron::JobResult {
            session_id: resp.session_id,
            summary: truncate(&resp.text, 500),
        }),
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
        "email" => {
            // Email channel: key is the sender email address.
            // All emails from the same sender continue the same session
            // until the gap timeout (24h default) expires.
            let key = msg.channel_session_key.clone()
                .unwrap_or_else(|| "unknown@sender".into());
            (Channel::Email, key)
        }
        _ => {
            // "main", "scheduler", or any unknown → explicit Main session
            let key = msg.channel_session_key.clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            (Channel::Main, key)
        }
    }
}

/// Truncate a string to a maximum length, adding "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        // Find the nearest char boundary at or before max_len to avoid
        // panicking on multi-byte UTF-8 sequences.
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> StarpodConfig {
        StarpodConfig {
            db_dir: tmp.path().join("db"),
            db_path: Some(tmp.path().join("db").join("memory.db")),
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
        // Skills dir should exist
        assert!(tmp.path().join("skills").exists());

        // Cron db should exist in db/
        assert!(tmp.path().join("db").join("cron.db").exists());
    }

    #[tokio::test]
    async fn test_agent_with_paths() {
        let tmp = TempDir::new().unwrap();
        let agent_home = tmp.path().join("agents").join("test-bot");
        let db_dir = agent_home.join("db");
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&agent_home).unwrap();
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::create_dir_all(&skills_dir).unwrap();

        let paths = ResolvedPaths {
            mode: starpod_core::Mode::Workspace {
                root: tmp.path().to_path_buf(),
                agent_name: "test-bot".to_string(),
            },
            agent_toml: agent_home.join("agent.toml"),
            agent_home: agent_home.clone(),
            config_dir: agent_home.clone(),
            db_dir: db_dir.clone(),
            skills_dir: skills_dir.clone(),
            project_root: tmp.path().join("home"),
            instance_root: tmp.path().to_path_buf(),
            home_dir: tmp.path().join("home"),
            users_dir: agent_home.join("users"),
            env_file: None,
        };

        let config = AgentConfig {
            agent_name: "TestBot".to_string(),
            ..AgentConfig::default()
        };

        let agent = StarpodAgent::with_paths(config, paths).await.unwrap();

        // paths() returns the workspace paths
        assert_eq!(agent.paths().agent_home, agent_home);
        assert_eq!(agent.paths().skills_dir, skills_dir);
        assert_eq!(agent.paths().project_root, tmp.path().join("home"));

        // Memory uses agent_home
        let ctx = agent.memory().bootstrap_context().unwrap();
        assert!(ctx.contains("TestBot") || ctx.contains("Aster"));

        // DB dir should have session.db
        assert!(db_dir.join("session.db").exists());
        assert!(db_dir.join("cron.db").exists());
    }

    #[tokio::test]
    async fn test_agent_with_paths_skill_filter() {
        let tmp = TempDir::new().unwrap();
        let agent_home = tmp.path().join("agent");
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&agent_home).unwrap();

        // Create two skills in the shared skills dir
        let skill_a = skills_dir.join("alpha");
        let skill_b = skills_dir.join("beta");
        std::fs::create_dir_all(&skill_a).unwrap();
        std::fs::create_dir_all(&skill_b).unwrap();
        std::fs::write(skill_a.join("SKILL.md"), "---\nname: alpha\ndescription: A\n---\nBody A").unwrap();
        std::fs::write(skill_b.join("SKILL.md"), "---\nname: beta\ndescription: B\n---\nBody B").unwrap();

        let paths = ResolvedPaths {
            mode: starpod_core::Mode::SingleAgent {
                starpod_dir: agent_home.clone(),
            },
            agent_toml: agent_home.join("agent.toml"),
            agent_home: agent_home.clone(),
            config_dir: agent_home.clone(),
            db_dir: agent_home.join("db"),
            skills_dir: skills_dir.clone(),
            project_root: tmp.path().join("home"),
            instance_root: tmp.path().to_path_buf(),
            home_dir: tmp.path().join("home"),
            users_dir: agent_home.join("users"),
            env_file: None,
        };

        // Filter to only "alpha"
        let config = AgentConfig {
            skills: vec!["alpha".to_string()],
            ..AgentConfig::default()
        };

        let agent = StarpodAgent::with_paths(config, paths).await.unwrap();

        let names = agent.skills().skill_names().unwrap();
        assert_eq!(names, vec!["alpha"]);
    }

    #[tokio::test]
    async fn test_reload_config() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        assert_eq!(agent.config().model(), "claude-haiku-4-5");

        // Reload with updated config
        let mut new_config = test_config(&tmp);
        new_config.models = vec!["anthropic/claude-opus-4-6".to_string()];
        new_config.agent_name = "Nova".to_string();
        agent.reload_config(new_config);

        let snapshot = agent.config();
        assert_eq!(snapshot.model(), "claude-opus-4-6");
        assert_eq!(snapshot.agent_name, "Nova");
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
        assert!(names.contains(&"EnvGet"));
        assert!(names.contains(&"FileRead"));
        assert!(names.contains(&"FileWrite"));
        assert!(names.contains(&"FileList"));
        assert!(names.contains(&"FileDelete"));
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

        assert!(names.contains(&"MemoryRead"));
        // Browser tools
        assert!(names.contains(&"BrowserOpen"));
        assert!(names.contains(&"BrowserWaitFor"));
        assert!(names.contains(&"BrowserClick"));
        assert!(names.contains(&"BrowserType"));
        assert!(names.contains(&"BrowserExtract"));
        assert!(names.contains(&"BrowserEval"));
        assert!(names.contains(&"BrowserClose"));
        assert!(names.contains(&"WebSearch"));
        assert!(names.contains(&"WebFetch"));
        assert_eq!(defs.len(), 30);
    }

    #[tokio::test]
    async fn test_custom_tool_handler() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let agent = StarpodAgent::new(config).await.unwrap();

        let ctx = ToolContext {
            memory: Arc::clone(agent.memory()),
            user_view: None,
            skills: Arc::clone(agent.skills()),
            cron: Arc::clone(agent.cron()),
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            agent_home: tmp.path().join(".starpod"),
            user_id: Some("admin".into()),
            http_client: reqwest::Client::new(),
            internet: starpod_core::InternetConfig::default(),
            brave_api_key: None,
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
        assert!(r.content.contains("Manual run recorded"));

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
        assert!(r.content.contains("success") || r.content.contains("Success")); // the run we completed

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
            false, None, 3, 7200, starpod_cron::SessionMode::Main, None,
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
        // Images now also get a save-path note in extra_text
        assert!(extra_text.contains("photo.png"));
        assert!(extra_text.contains("/tmp/photo.png"));
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

    #[tokio::test]
    async fn test_reload_config_updates_model() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Initial model is the default
        assert_eq!(agent.config().model(), "claude-haiku-4-5");

        // Reload with a new model
        let mut new_cfg = test_config(&tmp);
        new_cfg.models = vec!["anthropic/claude-opus-4-6".to_string()];
        agent.reload_config(new_cfg);

        assert_eq!(agent.config().model(), "claude-opus-4-6");
    }

    #[tokio::test]
    async fn test_reload_config_updates_agent_name() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        assert_eq!(agent.config().agent_name, "Aster");

        let mut new_cfg = test_config(&tmp);
        new_cfg.agent_name = "Nova".to_string();
        agent.reload_config(new_cfg);

        assert_eq!(agent.config().agent_name, "Nova");
    }

    #[tokio::test]
    async fn test_reload_config_updates_provider() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        assert_eq!(agent.config().provider(), "anthropic");

        let mut new_cfg = test_config(&tmp);
        new_cfg.models = vec!["openai/gpt-4o".to_string()];
        agent.reload_config(new_cfg);

        assert_eq!(agent.config().provider(), "openai");
    }

    #[tokio::test]
    async fn test_config_returns_snapshot() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Get a snapshot
        let mut snapshot = agent.config();
        assert_eq!(snapshot.model(), "claude-haiku-4-5");

        // Mutate the snapshot
        snapshot.models = vec!["anthropic/mutated-model".to_string()];

        // The agent's config should be unaffected
        assert_eq!(
            agent.config().model(),
            "claude-haiku-4-5",
            "Mutating a snapshot should not affect the agent's config"
        );
    }

    #[tokio::test]
    async fn test_export_sessions_disabled() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.export_sessions = false;

        let agent = StarpodAgent::new(cfg).await.unwrap();

        assert!(
            !agent.config().memory.export_sessions,
            "Agent config should reflect export_sessions=false"
        );
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
        // Both image and non-image files now get save-path notes
        assert!(extra_text.contains("report.pdf"));
        assert!(extra_text.contains("photo.jpg"));
    }

    #[tokio::test]
    async fn test_pre_compact_legacy_routes_to_user_dir() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.auto_log = false; // irrelevant here
        cfg.compaction.memory_flush = false; // force legacy fallback path
        let agent = StarpodAgent::new(cfg.clone()).await.unwrap();

        // Build legacy pre-compact handler for user "bob"
        let handler = agent.build_pre_compact_handler(&cfg, Some("bob")).await;

        // Simulate a compaction with one text message
        let messages = vec![agent_sdk::client::ApiMessage {
            role: "assistant".to_string(),
            content: vec![agent_sdk::client::ApiContentBlock::Text {
                text: "Important context about Bob's preferences".to_string(),
                cache_control: None,
            }],
        }];
        handler(messages).await;

        // Verify the daily log landed in users/bob/memory/
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let user_daily = tmp.path().join("users").join("bob").join("memory").join(format!("{}.md", today));
        assert!(user_daily.exists(), "Pre-compact daily log should be in user dir");

        let content = std::fs::read_to_string(&user_daily).unwrap();
        assert!(content.contains("Pre-compaction save"));
        assert!(content.contains("Important context"));

        // Agent-level should NOT have it
        let agent_daily = tmp.path().join("memory").join(format!("{}.md", today));
        assert!(!agent_daily.exists(), "Pre-compact log should NOT be in agent-level dir");
    }

    #[tokio::test]
    async fn test_append_daily_for_user_routes_to_user_dir() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Append with a user_id — should write to users/{id}/memory/
        agent.append_daily_for_user(Some("alice"), "Hello from Alice").await.unwrap();

        let user_memory_dir = tmp.path().join("users").join("alice").join("memory");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let daily_file = user_memory_dir.join(format!("{}.md", today));
        assert!(daily_file.exists(), "Daily log should be in user dir");

        let content = std::fs::read_to_string(&daily_file).unwrap();
        assert!(content.contains("Hello from Alice"));

        // Agent-level memory dir should NOT have today's file
        let agent_daily = tmp.path().join("memory").join(format!("{}.md", today));
        assert!(!agent_daily.exists(), "Daily log should NOT be in agent-level dir");
    }

    #[tokio::test]
    async fn test_append_daily_for_user_fallback_no_user() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Append with no user_id — should fall back to agent-level store
        agent.append_daily_for_user(None, "Agent-level entry").await.unwrap();

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        // Should be in agent-level memory (the MemoryStore root)
        let content = agent.memory().read_file(&format!("memory/{}.md", today)).unwrap();
        assert!(content.contains("Agent-level entry"));
    }

    #[test]
    fn test_append_execution_context_cron() {
        let mut prompt = "Base prompt.".to_string();
        append_execution_context(&mut prompt, None, Some("cron"));
        assert!(prompt.contains("--- EXECUTION CONTEXT ---"));
        assert!(prompt.contains("SCHEDULED CRON JOB"));
        assert!(prompt.contains("Do NOT schedule"));
    }

    #[test]
    fn test_append_execution_context_cron_via_channel() {
        let mut prompt = "Base prompt.".to_string();
        append_execution_context(&mut prompt, Some("scheduler"), Some("user123"));
        assert!(prompt.contains("--- EXECUTION CONTEXT ---"));
        assert!(prompt.contains("SCHEDULED CRON JOB"));
    }

    #[test]
    fn test_append_execution_context_heartbeat() {
        let mut prompt = "Base prompt.".to_string();
        append_execution_context(&mut prompt, None, Some("heartbeat"));
        assert!(prompt.contains("--- EXECUTION CONTEXT ---"));
        assert!(prompt.contains("HEARTBEAT"));
        assert!(prompt.contains("HEARTBEAT.md"));
    }

    #[test]
    fn test_append_execution_context_regular_user() {
        let mut prompt = "Base prompt.".to_string();
        append_execution_context(&mut prompt, Some("main"), Some("admin"));
        assert_eq!(prompt, "Base prompt.");
    }

    #[test]
    fn test_append_execution_context_none() {
        let mut prompt = "Base prompt.".to_string();
        append_execution_context(&mut prompt, None, None);
        assert_eq!(prompt, "Base prompt.");
    }
}
