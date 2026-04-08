pub mod flush;
pub mod nudge;
pub mod tools;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::Local;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

use agent_sdk::options::{SystemPrompt, ThinkingConfig};
use agent_sdk::{
    AnthropicProvider, BedrockProvider, GeminiProvider, OpenAiProvider, VertexProvider,
};
use agent_sdk::{
    ExternalToolHandlerFn, LlmProvider, Message, ModelRegistry, OllamaDiscovery, Options,
    PermissionMode, Query, QueryAttachment,
};
use starpod_core::{FollowupMode, ReasoningEffort};
use tokio::sync::mpsc;

use starpod_core::{
    AgentConfig, Attachment, ChatMessage, ChatResponse, ChatUsage, ResolvedPaths, Result,
    StarpodConfig, StarpodError,
};
use starpod_cron::CronStore;
use starpod_db::CoreDb;
use starpod_memory::{MemoryStore, UserMemoryView};
use starpod_session::{Channel, SessionDecision, SessionManager, UsageRecord};
use starpod_skills::SkillStore;

use crate::tools::{custom_tool_definitions, handle_custom_tool, ToolContext};

/// All custom tool names.
const CUSTOM_TOOLS: &[&str] = &[
    "MemorySearch",
    "MemoryWrite",
    "MemoryAppendDaily",
    "EnvGet",
    "FileRead",
    "FileWrite",
    "FileList",
    "FileDelete",
    "SkillActivate",
    "SkillCreate",
    "SkillUpdate",
    "SkillDelete",
    "SkillList",
    "CronAdd",
    "CronList",
    "CronRemove",
    "CronRuns",
    "CronRun",
    "CronUpdate",
    "HeartbeatWake",
    "WebSearch",
    "WebFetch",
    "BrowserOpen",
    "BrowserClick",
    "BrowserType",
    "BrowserExtract",
    "BrowserEval",
    "BrowserWaitFor",
    "BrowserClose",
    "Attach",
    "VaultGet",
    "VaultList",
    "VaultSet",
    "VaultDelete",
    "ConnectorList",
    "ConnectorAdd",
    "ConnectorRemove",
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
    vault: Option<Arc<starpod_vault::Vault>>,
    core_db: Arc<CoreDb>,
    paths: ResolvedPaths,
    config: RwLock<StarpodConfig>,
    /// Cached model registry (populated lazily with Ollama discovery).
    model_registry: tokio::sync::RwLock<Option<Arc<ModelRegistry>>>,
    /// Per-session bootstrap snapshot cache.
    ///
    /// The bootstrap context (SOUL.md, USER.md, MEMORY.md, daily logs) is
    /// frozen at session start and reused for every subsequent turn in that
    /// session. This avoids re-reading files from disk on every turn and —
    /// crucially — keeps the system-prompt prefix byte-identical across
    /// turns, which lets the LLM provider's prompt cache stay warm.
    ///
    /// Mid-session `MemoryWrite` calls still update files on disk, but the
    /// snapshot is only refreshed when a new session begins.
    bootstrap_cache: tokio::sync::RwLock<HashMap<String, String>>,
    /// Per-session user message counter for memory nudge scheduling.
    ///
    /// Maps `session_id → (user_id, message_count)`. Tracks how many user
    /// messages have been processed in each session. When the count reaches
    /// `config.memory.nudge_interval`, a background LLM call reviews the
    /// conversation and persists important information.
    ///
    /// The `user_id` is stored alongside the count so that
    /// [`flush_stale_sessions`] can find all sessions belonging to a user
    /// without querying the database.
    nudge_counters: tokio::sync::RwLock<HashMap<String, (String, u32)>>,
    /// Handle to the running secret proxy (Phase 2+). When `Some`, tool
    /// subprocesses get `HTTP_PROXY`/`HTTPS_PROXY` env vars pointing to it.
    #[cfg(feature = "secret-proxy")]
    proxy_handle: Option<starpod_proxy::ProxyHandle>,
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
            proxy: config.proxy.clone(),
            self_improve: config.self_improve,
        };

        let starpod_dir = config
            .db_dir
            .parent()
            .unwrap_or(&config.db_dir)
            .to_path_buf();
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
            connectors_dir: starpod_dir.join("connectors"),
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
        let mut memory =
            MemoryStore::new(&paths.agent_home, &paths.config_dir, &paths.db_dir).await?;
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

        // Unified core database (sessions + cron + auth)
        let core_db = Arc::new(CoreDb::new(&paths.db_dir).await?);
        let pool = core_db.pool().clone();

        let session_mgr = SessionManager::from_pool(pool.clone());

        // Skills from resolved skills_dir, with optional filter
        let skills = SkillStore::new(&paths.skills_dir)?.with_filter(agent_config.skills.clone());

        // Cron from shared pool
        let mut cron = CronStore::from_pool(pool);
        cron.set_default_max_retries(config.cron.default_max_retries);
        cron.set_default_timeout_secs(config.cron.default_timeout_secs);

        // Open vault if the key file exists (created at serve time by vault env populate)
        let vault = {
            let vault_key_path = paths.db_dir.join(".vault_key");
            if vault_key_path.exists() {
                let master_key = starpod_vault::derive_master_key(&paths.db_dir)?;
                let v =
                    starpod_vault::Vault::new(&paths.db_dir.join("vault.db"), &master_key).await?;
                Some(Arc::new(v))
            } else {
                None
            }
        };

        // Start the secret proxy if enabled (Phase 2+)
        #[cfg(feature = "secret-proxy")]
        let proxy_handle = if config.proxy.enabled {
            match starpod_vault::derive_master_key(&paths.db_dir) {
                Ok(master_key) => {
                    match starpod_proxy::start_proxy(starpod_proxy::ProxyConfig {
                        master_key,
                        data_dir: paths.db_dir.clone(),
                    })
                    .await
                    {
                        Ok(handle) => {
                            tracing::info!(port = handle.port(), "Secret proxy started");
                            Some(handle)
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to start secret proxy: {e} — falling back to no proxy"
                            );
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("No vault key for proxy: {e} — falling back to no proxy");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            memory: Arc::new(memory),
            session_mgr: Arc::new(session_mgr),
            skills: Arc::new(skills),
            cron: Arc::new(cron),
            vault,
            core_db,
            paths,
            config: RwLock::new(config),
            model_registry: tokio::sync::RwLock::new(None),
            bootstrap_cache: tokio::sync::RwLock::new(HashMap::new()),
            nudge_counters: tokio::sync::RwLock::new(HashMap::new()),
            #[cfg(feature = "secret-proxy")]
            proxy_handle,
        })
    }

    /// Get the resolved paths.
    pub fn paths(&self) -> &ResolvedPaths {
        &self.paths
    }

    /// Get the shared core database.
    pub fn core_db(&self) -> &Arc<CoreDb> {
        &self.core_db
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
    async fn save_attachments(&self, attachments: &[Attachment]) -> Vec<PathBuf> {
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
            let safe_name = att
                .file_name
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
            let path = saved_paths
                .get(i)
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
                    let modified = meta
                        .modified()
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
        format!("\n[Files already in downloads/: {}]", list.join(", "))
    }

    /// Build the system prompt from bootstrap context + skill catalog.
    ///
    /// The bootstrap context (SOUL.md, USER.md, MEMORY.md, daily logs) is
    /// frozen per session — computed once on the first turn and reused for
    /// all subsequent turns. This keeps the prompt prefix stable for the
    /// LLM provider's prompt cache and avoids redundant disk I/O.
    async fn build_system_prompt(
        &self,
        session_id: &str,
        config: &StarpodConfig,
        user_id: Option<&str>,
        activated_skill: Option<&str>,
    ) -> Result<String> {
        let agent_name = &config.agent_name;

        // Check the per-session bootstrap cache first.
        let bootstrap = {
            let cache = self.bootstrap_cache.read().await;
            cache.get(session_id).cloned()
        };
        let bootstrap = match bootstrap {
            Some(cached) => cached,
            None => {
                let fresh = if let Some(uid) = user_id {
                    let user_dir = self.paths.users_dir.join(uid);
                    let uv = UserMemoryView::new(Arc::clone(&self.memory), user_dir).await?;
                    uv.bootstrap_context(config.memory.bootstrap_file_cap)?
                } else {
                    self.memory.bootstrap_context()?
                };
                let mut cache = self.bootstrap_cache.write().await;
                cache.insert(session_id.to_string(), fresh.clone());
                fresh
            }
        };
        let skill_catalog = self.skills.skill_catalog_excluding(activated_skill)?;
        let date_str = Local::now().format("%A, %B %d, %Y at %H:%M").to_string();
        let tz_str = config
            .resolved_timezone()
            .unwrap_or_else(|| "UTC".to_string());

        // ── Build connectors section ────────────────────────────────────
        // Query the connectors table and format as `<connectors>` XML for
        // the system prompt. Each connector includes its name, type, status,
        // description, config key-value pairs, and resolved vault key names.
        // This gives the LLM full visibility into available service connections.
        //
        // Remaining vault keys that are NOT owned by any connector are still
        // listed in a separate "ENVIRONMENT VARIABLES" section below.
        let connector_store =
            starpod_db::connectors::ConnectorStore::from_pool(self.core_db.pool().clone());
        let connectors_section = match connector_store.list().await {
            Ok(rows) if !rows.is_empty() => {
                let mut xml = String::from("\n\n--- CONNECTORS ---\n<connectors>\n");
                for r in &rows {
                    xml.push_str(&format!(
                        "  <connector name=\"{}\" type=\"{}\" status=\"{}\" description=\"{}\">\n",
                        r.name, r.connector_type, r.status, r.description,
                    ));
                    if !r.config.is_empty() {
                        let attrs: Vec<String> = r
                            .config
                            .iter()
                            .map(|(k, v)| format!("{k}=\"{v}\""))
                            .collect();
                        xml.push_str(&format!("    <config {} />\n", attrs.join(" ")));
                    } else {
                        xml.push_str("    <config />\n");
                    }
                    if !r.secrets.is_empty() {
                        xml.push_str(&format!(
                            "    <secrets>{}</secrets>\n",
                            r.secrets.join(", ")
                        ));
                    }
                    xml.push_str("  </connector>\n");
                }
                xml.push_str("</connectors>\n\
                              Connectors represent configured service connections. Their secrets are \
                              stored in the vault — retrieve them with VaultGet (e.g. VaultGet({\"key\": \"GITHUB_TOKEN\"})) \
                              before using them in API calls. Do NOT assume secrets are available as \
                              environment variables. Never hardcode secret values in commands — store \
                              them in a variable and reference it. \
                              Manage connectors with ConnectorList, ConnectorAdd, ConnectorRemove.");
                xml
            }
            _ => String::new(),
        };

        // Collect vault keys that are NOT owned by any connector (standalone secrets)
        let mut connector_keys: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        if let Ok(rows) = connector_store.list().await {
            for r in &rows {
                for s in &r.secrets {
                    connector_keys.insert(s.clone());
                }
            }
        }
        let env_vars_section = if let Some(ref vault) = self.vault {
            match vault.list_keys().await {
                Ok(keys) => {
                    let user_keys: Vec<&str> = keys
                        .iter()
                        .map(|k| k.as_str())
                        .filter(|k| {
                            !starpod_vault::is_system_key(k)
                                && std::env::var(k).is_ok()
                                && !connector_keys.contains(*k)
                        })
                        .collect();
                    if user_keys.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "\n\n--- ENVIRONMENT VARIABLES ---\n\
                             You have the following environment variables available: {}\n\
                             These are pre-configured credentials and settings. You can:\n\
                             • Read them with the EnvGet tool (e.g. EnvGet({{\"key\": \"{}\"}})).\n\
                             • Use them directly in Bash/SSH commands — they are real process environment \
                             variables, so any shell command, script, or program you run inherits them \
                             automatically (e.g. `${}` in a shell, `os.environ[\"{}\"]` in Python, \
                             `process.env.{}` in Node).\n\
                             Do NOT hardcode these values — always reference them as environment variables.",
                            user_keys.join(", "),
                            user_keys[0],
                            user_keys[0],
                            user_keys[0],
                            user_keys[0],
                        )
                    }
                }
                Err(e) => {
                    warn!("Failed to list vault keys for system prompt: {}", e);
                    String::new()
                }
            }
        } else {
            String::new()
        };

        let mut prompt = format!(
            "You are {agent_name}, a personal AI assistant.\n\n{bootstrap}\n\n---\n\
             Current date/time: {date_str}\nTimezone: {tz_str}\nSession ID: {session_id}\n\
             Home directory: ~/\n\
             Working directory: ~/\n\n\
             You have access to memory tools (MemorySearch, MemoryWrite, MemoryAppendDaily), \
             environment tools (EnvGet), file tools (FileRead, FileWrite, FileList, FileDelete), \
             skill tools (SkillActivate, SkillCreate, SkillUpdate, SkillDelete, SkillList), \
             scheduling tools (CronAdd, CronList, CronRemove, CronRuns, CronRun, CronUpdate, HeartbeatWake), \
             browser tools (BrowserOpen, BrowserClick, BrowserType, BrowserExtract, BrowserEval, BrowserWaitFor, BrowserClose), \
             and connector tools (ConnectorList, ConnectorAdd, ConnectorRemove).\n\
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
             IMPORTANT: Always create files and run commands within ~/, never in /tmp or other external directories.\n\n\
             --- FILESYSTEM ORGANIZATION ---\n\
             You are the steward of your home directory. Keep it clean and well-organized — \
             think of it as a real computer that the user will live with long-term.\n\n\
             Standard directories (recommended — use them whenever they fit):\n\
             • ~/desktop/   — items the user wants quick access to; shortcuts, pinned files, daily scratch notes\n\
             • ~/documents/ — long-lived text: reports, notes, reference material, generated documents\n\
             • ~/projects/  — code and structured project folders (each project gets its own subfolder)\n\
             • ~/downloads/ — user-uploaded files (managed automatically, do not move or rename these)\n\
             • ~/scripts/   — reusable scripts, automation, shell snippets the user may run again\n\
             • ~/temp/      — throwaway work: intermediate outputs, one-off experiments, scratch data. \
             Clean this up when done — files here are not meant to persist.\n\n\
             File creation guidelines:\n\
             • Prefer placing files in the standard directories above. Only create new top-level folders \
             when nothing existing fits and the use case clearly warrants it.\n\
             • Use clear, descriptive filenames in lowercase with hyphens (e.g. monthly-report.md, not Report.md).\n\
             • Group related files in subdirectories rather than dumping many files in one flat folder \
             (e.g. ~/projects/website/ instead of ~/website-index.html, ~/website-style.css).\n\n\
             File lifecycle:\n\
             • Temporary/intermediate files belong in ~/temp/. Delete them when the task is done.\n\
             • When replacing a file with a new version under a different name, remove the old one.\n\
             • Before creating a new directory, check with FileList if a suitable one already exists.\n\
             • If you notice the filesystem getting cluttered, proactively suggest tidying up.",
        );

        // ── Connectors ────────────────────────────────────────────────
        if !connectors_section.is_empty() {
            prompt.push_str(&connectors_section);
        }

        // ── Environment variables (vault, excluding connector-owned keys) ──
        if !env_vars_section.is_empty() {
            prompt.push_str(&env_vars_section);
        }

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
                 SKILL CONNECTOR DECLARATIONS:\n\
                 When creating or updating skills that interact with external services, declare their \
                 connector requirements using the `connectors` parameter — a list of connector names \
                 (e.g. [\"github\", \"postgres\"]). Only declare connectors when the skill genuinely \
                 needs external access — do not add connectors to skills that only use built-in tools.",
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

/// Resolve a model spec (which may be in `"provider/model"` format) into
/// separate `(provider, model)` strings.  Falls back to the config's default
/// provider/model when `spec` is `None` or lacks a provider prefix.
///
/// This is used by background tasks (nudge, flush) that accept an optional
/// model override from config.  The override may contain a provider prefix
/// (e.g. `"anthropic/claude-haiku-4-5-20251001"`) that must be stripped
/// before sending the model name to the API.
fn resolve_background_model(spec: Option<&str>, config: &StarpodConfig) -> (String, String) {
    match spec {
        Some(s) => match starpod_core::parse_model_spec(s) {
            Some((p, m)) => (p.to_string(), m.to_string()),
            None => (config.provider().to_string(), s.to_string()),
        },
        None => (config.provider().to_string(), config.model().to_string()),
    }
}

impl StarpodAgent {
    /// Map reasoning effort config to ThinkingConfig.
    fn thinking_config(config: &StarpodConfig) -> Option<ThinkingConfig> {
        config.reasoning_effort.map(|effort| match effort {
            ReasoningEffort::Low => ThinkingConfig::Enabled {
                budget_tokens: 4096,
            },
            ReasoningEffort::Medium => ThinkingConfig::Enabled {
                budget_tokens: 10240,
            },
            ReasoningEffort::High => ThinkingConfig::Enabled {
                budget_tokens: 32768,
            },
        })
    }

    /// Build the allowed tools list (built-in + custom).
    fn allowed_tools() -> Vec<String> {
        let mut tools: Vec<String> =
            vec!["Read".into(), "Bash".into(), "Glob".into(), "Grep".into()];
        tools.extend(CUSTOM_TOOLS.iter().map(|s| s.to_string()));
        tools
    }

    /// Build an LLM provider for the given provider name using config for API key / base URL.
    async fn build_provider_for(
        &self,
        provider_name: &str,
        config: &StarpodConfig,
    ) -> Result<Box<dyn LlmProvider>> {
        let api_key = config.resolved_provider_api_key(provider_name)
            .ok_or_else(|| StarpodError::Config(format!(
                "No API key found for provider '{}'. Set it in config.toml or via environment variable.",
                provider_name
            )))?;
        let base_url = config
            .resolved_provider_base_url(provider_name)
            .ok_or_else(|| {
                StarpodError::Config(format!("Unknown provider: '{}'", provider_name))
            })?;

        let pricing = self.load_model_registry().await;

        let provider: Box<dyn LlmProvider> = match provider_name {
            "anthropic" => {
                Box::new(AnthropicProvider::new(api_key, base_url).with_pricing(pricing))
            }
            "bedrock" => {
                // Bedrock handles its own auth via AWS SigV4 — region from config options or env
                let opts = config.provider_options("bedrock");
                let region = opts
                    .get("region")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| std::env::var("AWS_REGION").ok())
                    .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
                    .unwrap_or_else(|| "us-east-1".to_string());
                let provider = BedrockProvider::with_region(region)
                    .map_err(|e| StarpodError::Config(format!("Bedrock provider error: {e}")))?;
                Box::new(provider.with_pricing(pricing))
            }
            "vertex" => {
                // Vertex AI handles its own auth via Google ADC — project_id and region from config options or env
                let opts = config.provider_options("vertex");
                let project_id = opts.get("project_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| std::env::var("GOOGLE_CLOUD_PROJECT").ok())
                    .or_else(|| std::env::var("GCP_PROJECT_ID").ok())
                    .ok_or_else(|| StarpodError::Config(
                        "Vertex AI requires project_id in [providers.vertex.options] or GOOGLE_CLOUD_PROJECT env var".into()
                    ))?;
                let region = opts
                    .get("region")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| std::env::var("GOOGLE_CLOUD_LOCATION").ok())
                    .or_else(|| std::env::var("GCP_REGION").ok())
                    .unwrap_or_else(|| "us-central1".to_string());
                let provider = VertexProvider::new(project_id, region)
                    .await
                    .map_err(|e| StarpodError::Config(format!("Vertex AI provider error: {e}")))?;
                Box::new(provider.with_pricing(pricing))
            }
            "gemini" => {
                Box::new(GeminiProvider::with_base_url(api_key, base_url).with_pricing(pricing))
            }
            // OpenAI-compatible providers
            "openai" | "groq" | "deepseek" | "openrouter" | "ollama" => {
                let mut opts = config.provider_options(provider_name).clone();
                // Ollama: default keep_alive to ensure KV cache reuse between agentic turns
                if provider_name == "ollama" && !opts.contains_key("keep_alive") {
                    opts.insert("keep_alive".into(), serde_json::json!("5m"));
                }
                Box::new(
                    OpenAiProvider::with_base_url(api_key, base_url, provider_name)
                        .with_pricing(pricing)
                        .with_extra_body(opts),
                )
            }
            other => {
                return Err(StarpodError::Config(format!(
                    "Unsupported provider: '{}'. Supported: anthropic, bedrock, vertex, openai, gemini, groq, deepseek, openrouter, ollama",
                    other
                )));
            }
        };

        Ok(provider)
    }

    /// Load the model registry: embedded defaults + optional config override + Ollama discovery.
    ///
    /// The registry is cached after first load. Ollama models are discovered
    /// asynchronously on first call — if Ollama isn't running, this falls back
    /// gracefully to the static catalog.
    async fn load_model_registry(&self) -> Arc<ModelRegistry> {
        // Return cached if available.
        {
            let cached = self.model_registry.read().await;
            if let Some(ref reg) = *cached {
                return Arc::clone(reg);
            }
        }

        let mut registry = ModelRegistry::with_defaults();

        // Layer 1: user overrides from config/models.toml.
        let pricing_path = self.paths.config_dir.join("models.toml");
        if pricing_path.exists() {
            match std::fs::read_to_string(&pricing_path) {
                Ok(contents) => match ModelRegistry::from_toml(&contents) {
                    Ok(overrides) => {
                        debug!(path = %pricing_path.display(), "loaded pricing overrides");
                        registry.merge(overrides);
                    }
                    Err(e) => {
                        warn!(path = %pricing_path.display(), error = %e, "failed to parse models.toml, using defaults");
                    }
                },
                Err(e) => {
                    warn!(path = %pricing_path.display(), error = %e, "failed to read models.toml, using defaults");
                }
            }
        }

        // Layer 2: Ollama auto-discovery.
        let config = self.config.read().unwrap().clone();
        if let Some(base_url) = config.resolved_provider_base_url("ollama") {
            let discovery = OllamaDiscovery::new(&base_url);
            match discovery.discover_all().await {
                Ok(ollama_models) => {
                    debug!(count = ollama_models.len(), "discovered ollama models");
                    registry.merge(ollama_models);
                }
                Err(e) => {
                    debug!(error = %e, "ollama discovery unavailable, using static catalog only");
                }
            }
        }

        let result = Arc::new(registry);
        *self.model_registry.write().await = Some(Arc::clone(&result));
        result
    }

    /// Invalidate the cached model registry (e.g. after config change).
    pub async fn invalidate_model_registry(&self) {
        *self.model_registry.write().await = None;
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
                        while end > 0 && !combined.is_char_boundary(end) {
                            end -= 1;
                        }
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

        // Agentic flush: resolve model spec and build the correct provider.
        let flush_spec = config
            .compaction
            .flush_model
            .clone()
            .or_else(|| config.compaction_model.clone());
        let (flush_provider_name, flush_model) =
            resolve_background_model(flush_spec.as_deref(), config);

        let provider: Arc<dyn LlmProvider> = match self
            .build_provider_for(&flush_provider_name, config)
            .await
        {
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
                                if let agent_sdk::client::ApiContentBlock::Text { text, .. } = block
                                {
                                    parts.push(text.clone());
                                }
                            }
                        }
                        if !parts.is_empty() {
                            let combined = parts.join("\n");
                            let truncated = if combined.len() > 2000 {
                                let mut end = 2000;
                                while end > 0 && !combined.is_char_boundary(end) {
                                    end -= 1;
                                }
                                format!("{}...", &combined[..end])
                            } else {
                                combined
                            };
                            let result = if let Some(ref uv) = user_view {
                                uv.append_daily(&format!("## Pre-compaction save\n{}", truncated))
                                    .await
                            } else {
                                memory
                                    .append_daily(&format!("## Pre-compaction save\n{}", truncated))
                                    .await
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
                )
                .await;
            })
        })
    }

    /// Build the external tool handler closure.
    async fn build_tool_handler(
        &self,
        config: &StarpodConfig,
        user_id: Option<&str>,
        attachments: Arc<tokio::sync::Mutex<Vec<Attachment>>>,
    ) -> ExternalToolHandlerFn {
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

        let connector_store =
            starpod_db::connectors::ConnectorStore::from_pool(self.core_db.pool().clone());

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
            vault: self.vault.clone(),
            user_md_limit: config.memory.user_md_limit,
            memory_md_limit: config.memory.memory_md_limit,
            attachments,
            proxy_enabled: config.proxy.enabled,
            connector_store: Some(connector_store),
            connectors_dir: self.paths.connectors_dir.clone(),
            oauth_proxy_url: Some(
                std::env::var("OAUTH_PROXY_URL")
                    .or_else(|_| std::env::var("STARPOD_URL"))
                    .unwrap_or_else(|_| "https://console.starpod.sh".to_string()),
            ),
        });

        Box::new(move |tool_name, input| {
            let ctx = Arc::clone(&ctx);
            Box::pin(async move {
                let result = handle_custom_tool(&ctx, &tool_name, &input).await;
                // If a known custom tool returned None, it means required parameters
                // were missing/invalid (the `?` operator on Option bailed out).
                // Return an explicit error instead of falling through to the built-in
                // executor which doesn't know about these tools.
                if result.is_none() && CUSTOM_TOOLS.contains(&tool_name.as_str()) {
                    return Some(agent_sdk::ToolResult {
                        content: format!(
                            "Invalid or missing parameters for tool '{tool_name}'. Input received: {input}"
                        ),
                        is_error: true,
                        raw_content: None,
                    });
                }
                result
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
        let (session_id, is_resuming) = match self
            .session_mgr
            .resolve_session_for_user(&channel, &key, gap, user_id)
            .await?
        {
            SessionDecision::Continue(id) => {
                debug!(session_id = %id, channel = %channel.as_str(), "Continuing existing session");
                (id, true)
            }
            SessionDecision::New { closed_session_id } => {
                // Export the closed session's transcript to memory (in background)
                if let Some(ref closed_id) = closed_session_id {
                    self.export_session_to_memory(closed_id).await;
                }
                let id = self
                    .session_mgr
                    .create_session_full(
                        &channel,
                        &key,
                        message.user_id.as_deref().unwrap_or("admin"),
                        message.triggered_by.as_deref(),
                    )
                    .await?;
                debug!(session_id = %id, channel = %channel.as_str(), "Created new session");
                (id, false)
            }
        };
        self.session_mgr.touch_session(&session_id).await?;
        let _ = self
            .session_mgr
            .set_title_if_empty(&session_id, &message.text)
            .await;

        // Flush un-nudged messages from other sessions this user left behind
        self.flush_stale_sessions(&session_id, user_id, &config)
            .await;

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
        let mut system_prompt = self
            .build_system_prompt(&session_id, &config, message.user_id.as_deref(), None)
            .await?;

        append_execution_context(
            &mut system_prompt,
            message.channel_id.as_deref(),
            message.user_id.as_deref(),
        );

        // Step 4: Resolve model (may be overridden per-message) and build provider
        let (resolved_provider, resolved_model) = config
            .resolve_model(message.model.as_deref())
            .map_err(StarpodError::Config)?;
        let provider = self.build_provider_for(&resolved_provider, &config).await?;

        // Attachment accumulator — populated by the Attach tool during the agent loop
        let out_attachments: Arc<tokio::sync::Mutex<Vec<Attachment>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));

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
            .external_tool_handler(
                self.build_tool_handler(
                    &config,
                    message.user_id.as_deref(),
                    Arc::clone(&out_attachments),
                )
                .await,
            )
            .pre_compact_handler(
                self.build_pre_compact_handler(&config, message.user_id.as_deref())
                    .await,
            )
            .custom_tools(custom_tool_definitions())
            .attachments(query_atts)
            .provider(provider)
            .cwd(config.project_root.to_string_lossy().to_string())
            .additional_directories(vec![])
            .env_blocklist(
                starpod_vault::SYSTEM_KEYS
                    .iter()
                    .map(|k| k.to_string())
                    .collect(),
            )
            .hook_dirs(vec![config.db_dir.join("hooks")]);

        // Inject proxy env vars into tool subprocesses
        #[cfg(feature = "secret-proxy")]
        if let Some(ref handle) = self.proxy_handle {
            let proxy_url = format!("http://127.0.0.1:{}", handle.port());
            builder = builder
                .env("HTTP_PROXY", &proxy_url)
                .env("HTTPS_PROXY", &proxy_url)
                .env("http_proxy", &proxy_url)
                .env("https_proxy", &proxy_url);
            if let Some(ref ca_path) = handle.ca_cert_path {
                let ca = ca_path.to_string_lossy().to_string();
                builder = builder
                    .env("SSL_CERT_FILE", &ca)
                    .env("NODE_EXTRA_CA_CERTS", &ca)
                    .env("REQUESTS_CA_BUNDLE", &ca);
            }
            // Tier 1: network namespace pre_exec hook (Linux only)
            #[cfg(all(unix, feature = "secret-proxy-netns"))]
            if let Some(hook) = handle.pre_exec_hook() {
                builder = builder.pre_exec_fn(hook);
            }
        }

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
                    match self.build_provider_for(cp, &config).await {
                        Ok(p) => {
                            builder = builder.compaction_provider(p);
                        }
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

                        let _ = self
                            .session_mgr
                            .record_usage(
                                &session_id,
                                &UsageRecord {
                                    input_tokens: u.input_tokens,
                                    output_tokens: u.output_tokens,
                                    cache_read: u.cache_read_input_tokens,
                                    cache_write: u.cache_creation_input_tokens,
                                    cost_usd: result.total_cost_usd,
                                    model: resolved_model.clone(),
                                    user_id: message
                                        .user_id
                                        .clone()
                                        .unwrap_or_else(|| "admin".into()),
                                },
                                result.num_turns,
                            )
                            .await;
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
        let _ = self
            .session_mgr
            .save_message(&session_id, "user", &message.text)
            .await;
        if !result_text.is_empty() {
            let _ = self
                .session_mgr
                .save_message(&session_id, "assistant", &result_text)
                .await;
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
            let _ = self
                .append_daily_for_user(message.user_id.as_deref(), &entry)
                .await;
        }

        // Step 7: Background memory nudge (every N user messages)
        self.maybe_nudge_memory(&session_id, message.user_id.as_deref(), &config)
            .await;

        let attachments = out_attachments.lock().await.drain(..).collect();

        Ok(ChatResponse {
            text: result_text,
            session_id,
            usage: Some(usage),
            attachments,
        })
    }

    /// Start a streaming chat that yields raw agent-sdk messages.
    ///
    /// Returns (Query stream, session_id, followup_tx, out_attachments).
    /// The caller should consume the stream for real-time display, then call
    /// `finalize_chat()` with the collected results. After the stream ends,
    /// drain `out_attachments` for any files the agent attached via the `Attach`
    /// tool.
    ///
    /// The returned `followup_tx` can be used to inject followup messages into
    /// the running agent loop (when `followup_mode = "inject"`). Messages sent
    /// through this channel are drained at each iteration boundary and appended
    /// as user messages before the next API call.
    pub async fn chat_stream(
        &self,
        message: &ChatMessage,
    ) -> Result<(
        Query,
        String,
        mpsc::UnboundedSender<String>,
        Arc<tokio::sync::Mutex<Vec<Attachment>>>,
    )> {
        let config = self.snapshot_config();

        let (channel, key) = resolve_channel(message);
        let gap = config.channel_gap_minutes(channel.as_str());
        let user_id = message.user_id.as_deref().unwrap_or("admin");
        let (session_id, is_resuming) = match self
            .session_mgr
            .resolve_session_for_user(&channel, &key, gap, user_id)
            .await?
        {
            SessionDecision::Continue(id) => {
                debug!(session_id = %id, channel = %channel.as_str(), "Continuing existing session");
                (id, true)
            }
            SessionDecision::New { closed_session_id } => {
                if let Some(ref closed_id) = closed_session_id {
                    self.export_session_to_memory(closed_id).await;
                }
                let id = self
                    .session_mgr
                    .create_session_full(
                        &channel,
                        &key,
                        message.user_id.as_deref().unwrap_or("admin"),
                        message.triggered_by.as_deref(),
                    )
                    .await?;
                debug!(session_id = %id, channel = %channel.as_str(), "Created new session");
                (id, false)
            }
        };
        self.session_mgr.touch_session(&session_id).await?;
        let _ = self
            .session_mgr
            .set_title_if_empty(&session_id, &message.text)
            .await;

        // Flush un-nudged messages from other sessions this user left behind
        self.flush_stale_sessions(&session_id, user_id, &config)
            .await;

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

        let system_prompt = self
            .build_system_prompt(
                &session_id,
                &config,
                message.user_id.as_deref(),
                activated_skill.as_deref(),
            )
            .await?;

        // Resolve model (may be overridden per-message)
        let (resolved_provider, resolved_model) = config
            .resolve_model(message.model.as_deref())
            .map_err(StarpodError::Config)?;
        let provider = self.build_provider_for(&resolved_provider, &config).await?;

        // Create the followup channel — sender goes to caller, receiver to the agent loop
        let (followup_tx, followup_rx) = mpsc::unbounded_channel::<String>();

        // Attachment accumulator — populated by the Attach tool, drained by the caller
        let out_attachments: Arc<tokio::sync::Mutex<Vec<Attachment>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));

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
            .external_tool_handler(
                self.build_tool_handler(
                    &config,
                    message.user_id.as_deref(),
                    Arc::clone(&out_attachments),
                )
                .await,
            )
            .pre_compact_handler(
                self.build_pre_compact_handler(&config, message.user_id.as_deref())
                    .await,
            )
            .custom_tools(custom_tool_definitions())
            .followup_rx(followup_rx)
            .attachments(query_atts)
            .provider(provider)
            .cwd(config.project_root.to_string_lossy().to_string())
            .additional_directories(vec![])
            .env_blocklist(
                starpod_vault::SYSTEM_KEYS
                    .iter()
                    .map(|k| k.to_string())
                    .collect(),
            )
            .hook_dirs(vec![config.db_dir.join("hooks")])
            .include_partial_messages(true);

        // Inject proxy env vars into tool subprocesses
        #[cfg(feature = "secret-proxy")]
        if let Some(ref handle) = self.proxy_handle {
            let proxy_url = format!("http://127.0.0.1:{}", handle.port());
            builder = builder
                .env("HTTP_PROXY", &proxy_url)
                .env("HTTPS_PROXY", &proxy_url)
                .env("http_proxy", &proxy_url)
                .env("https_proxy", &proxy_url);
            if let Some(ref ca_path) = handle.ca_cert_path {
                let ca = ca_path.to_string_lossy().to_string();
                builder = builder
                    .env("SSL_CERT_FILE", &ca)
                    .env("NODE_EXTRA_CA_CERTS", &ca)
                    .env("REQUESTS_CA_BUNDLE", &ca);
            }
            // Tier 1: network namespace pre_exec hook (Linux only)
            #[cfg(all(unix, feature = "secret-proxy-netns"))]
            if let Some(hook) = handle.pre_exec_hook() {
                builder = builder.pre_exec_fn(hook);
            }
        }

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
                    match self.build_provider_for(cp, &config).await {
                        Ok(p) => {
                            builder = builder.compaction_provider(p);
                        }
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
        Ok((stream, session_id, followup_tx, out_attachments))
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
            let _ = self
                .session_mgr
                .record_usage(
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
                )
                .await;
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

        // Background memory nudge (every N user messages)
        self.maybe_nudge_memory(session_id, user_id, &config).await;
    }

    /// Increment the nudge counter for a session and spawn a background
    /// review if the interval has been reached.
    ///
    /// The model is resolved via [`resolve_background_model`] from
    /// `nudge_model` → `flush_model` → `compaction_model` → primary.
    /// Model specs in `"provider/model"` format are split so only the
    /// model name reaches the API and the correct provider is used.
    ///
    /// When `self_improve` is enabled, the nudge also includes skill tools
    /// so the background LLM can create or update skills from the conversation.
    ///
    /// Returns immediately — the nudge runs in a detached `tokio::spawn` task.
    async fn maybe_nudge_memory(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        config: &StarpodConfig,
    ) {
        let interval = config.memory.nudge_interval;
        if interval == 0 {
            return;
        }

        let count = {
            let mut counters = self.nudge_counters.write().await;
            let entry = counters
                .entry(session_id.to_string())
                .or_insert_with(|| (user_id.unwrap_or("admin").to_string(), 0));
            entry.1 += 1;
            entry.1
        };

        if count % interval != 0 {
            return;
        }

        // Time for a nudge — gather what we need and spawn in background
        let messages = match self.session_mgr.get_messages(session_id).await {
            Ok(msgs) if !msgs.is_empty() => msgs,
            _ => return,
        };

        // Resolve the model: nudge_model → flush_model → compaction_model → primary.
        let nudge_spec = config
            .memory
            .nudge_model
            .clone()
            .or_else(|| config.compaction.flush_model.clone())
            .or_else(|| config.compaction_model.clone());
        let (nudge_provider, nudge_model) = resolve_background_model(nudge_spec.as_deref(), config);

        let provider: Arc<dyn agent_sdk::LlmProvider> =
            match self.build_provider_for(&nudge_provider, config).await {
                Ok(p) => Arc::from(p),
                Err(e) => {
                    warn!(error = %e, "Failed to build provider for background nudge");
                    return;
                }
            };

        let memory = Arc::clone(&self.memory);
        let user_view: Option<Arc<UserMemoryView>> = match user_id {
            Some(uid) => {
                let user_dir = self.paths.users_dir.join(uid);
                match UserMemoryView::new(Arc::clone(&memory), user_dir).await {
                    Ok(uv) => Some(Arc::new(uv)),
                    Err(_) => None,
                }
            }
            None => None,
        };

        // When self-improve is on, pass skills to the nudge for unified review
        let skills = if config.self_improve {
            Some(Arc::clone(&self.skills))
        } else {
            None
        };

        let self_improve = config.self_improve;
        info!(session_id, count, self_improve, "Spawning background nudge");

        tokio::spawn(async move {
            nudge::run_nudge(
                provider,
                &nudge_model,
                &messages,
                &memory,
                user_view.as_deref(),
                skills.as_deref(),
            )
            .await;
        });
    }

    /// Run a final background nudge for a closing session.
    ///
    /// Called by [`export_session_to_memory`] when a session ends with
    /// un-nudged messages (e.g., a 3-message chat that never hit the nudge
    /// interval). Uses the session's user_id from metadata for per-user
    /// memory routing.
    ///
    /// Model resolution follows the same chain as [`maybe_nudge_memory`]
    /// via [`resolve_background_model`].
    async fn run_final_nudge(&self, session_id: &str, config: &StarpodConfig) {
        let messages = match self.session_mgr.get_messages(session_id).await {
            Ok(msgs) if !msgs.is_empty() => msgs,
            _ => return,
        };

        // Resolve the model: nudge_model → flush_model → compaction_model → primary.
        let nudge_spec = config
            .memory
            .nudge_model
            .clone()
            .or_else(|| config.compaction.flush_model.clone())
            .or_else(|| config.compaction_model.clone());
        let (nudge_provider, nudge_model) = resolve_background_model(nudge_spec.as_deref(), config);

        let provider: Arc<dyn agent_sdk::LlmProvider> =
            match self.build_provider_for(&nudge_provider, config).await {
                Ok(p) => Arc::from(p),
                Err(e) => {
                    warn!(error = %e, "Failed to build provider for final nudge");
                    return;
                }
            };

        // Resolve user_id from session metadata for per-user memory routing
        let user_id = match self.session_mgr.get_session(session_id).await {
            Ok(Some(meta))
                if !meta.user_id.is_empty()
                    && meta.user_id != "heartbeat"
                    && meta.user_id != "cron" =>
            {
                Some(meta.user_id)
            }
            _ => None,
        };

        let memory = Arc::clone(&self.memory);
        let user_view: Option<Arc<UserMemoryView>> = match user_id.as_deref() {
            Some(uid) => {
                let user_dir = self.paths.users_dir.join(uid);
                match UserMemoryView::new(Arc::clone(&memory), user_dir).await {
                    Ok(uv) => Some(Arc::new(uv)),
                    Err(_) => None,
                }
            }
            None => None,
        };

        let skills = if config.self_improve {
            Some(Arc::clone(&self.skills))
        } else {
            None
        };

        info!(session_id, "Spawning final nudge for closing session");

        tokio::spawn(async move {
            nudge::run_nudge(
                provider,
                &nudge_model,
                &messages,
                &memory,
                user_view.as_deref(),
                skills.as_deref(),
            )
            .await;
        });
    }

    /// Flush un-nudged sessions belonging to a user when they switch context.
    ///
    /// Scans `nudge_counters` for sessions owned by `user_id` that are NOT
    /// `current_session_id` and have un-nudged messages (count > 0, not at an
    /// interval boundary). For each, spawns a final nudge and resets the
    /// counter so it won't be nudged again.
    ///
    /// This catches short conversations that never reached the nudge interval
    /// (e.g., 3 messages in a web UI chat before starting a new one).
    async fn flush_stale_sessions(
        &self,
        current_session_id: &str,
        user_id: &str,
        config: &StarpodConfig,
    ) {
        let interval = config.memory.nudge_interval;
        if interval == 0 {
            return;
        }

        // Collect stale session IDs under a read lock
        let stale: Vec<String> = {
            let counters = self.nudge_counters.read().await;
            counters
                .iter()
                .filter(|(sid, (uid, count))| {
                    sid.as_str() != current_session_id
                        && uid == user_id
                        && *count > 0
                        && *count % interval != 0
                })
                .map(|(sid, _)| sid.clone())
                .collect()
        };

        if stale.is_empty() {
            return;
        }

        // Reset counters so these sessions won't be flushed again
        {
            let mut counters = self.nudge_counters.write().await;
            for sid in &stale {
                if let Some(entry) = counters.get_mut(sid) {
                    entry.1 = 0;
                }
            }
        }

        for sid in stale {
            debug!(session_id = %sid, user_id, "Flushing stale session for user");
            self.run_final_nudge(&sid, config).await;
        }
    }

    /// Append to daily log via user view when a user_id is present, falling back to agent-level store.
    async fn append_daily_for_user(
        &self,
        user_id: Option<&str>,
        text: &str,
    ) -> starpod_core::Result<()> {
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
    /// Also runs a final background nudge if the session had messages that
    /// never reached the nudge interval (e.g., a 3-message chat with
    /// `nudge_interval = 10`), so short conversations aren't lost.
    ///
    /// Formats all messages as markdown and writes to the memory store so they
    /// become searchable. Runs in the background to avoid blocking the chat flow.
    async fn export_session_to_memory(&self, session_id: &str) {
        // Always evict the frozen bootstrap snapshot when a session closes,
        // regardless of whether the transcript export is enabled.
        self.bootstrap_cache.write().await.remove(session_id);

        // Grab and evict the nudge counter — if it has un-nudged messages,
        // we'll run a final nudge below.
        let pending_count = self
            .nudge_counters
            .write()
            .await
            .remove(session_id)
            .map(|(_, count)| count)
            .unwrap_or(0);

        // Run a final nudge for sessions that ended before reaching the interval
        let config = self.snapshot_config();
        let interval = config.memory.nudge_interval;
        if interval > 0 && pending_count > 0 && pending_count % interval != 0 {
            self.run_final_nudge(session_id, &config).await;
        }

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
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
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
            title,
            &meta.created_at[..10.min(meta.created_at.len())],
            meta.channel,
            meta.message_count,
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
        let write_result =
            if !meta.user_id.is_empty() && meta.user_id != "heartbeat" && meta.user_id != "cron" {
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
            debug!(
                session_id,
                filename, "Exported session transcript to memory"
            );
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

    /// Get a reference to the vault (if available).
    pub fn vault(&self) -> Option<&Arc<starpod_vault::Vault>> {
        self.vault.as_ref()
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
                    starpod_cron::SessionMode::Isolated => ("scheduler".to_string(), None),
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
async fn ensure_heartbeat(agent: &StarpodAgent, store: &CronStore) -> Result<()> {
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

    info!(
        interval_minutes = interval,
        "Created __heartbeat__ cron job"
    );
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
            let key = msg
                .channel_session_key
                .clone()
                .or_else(|| msg.user_id.clone())
                .unwrap_or_else(|| "default".into());
            (Channel::Telegram, key)
        }
        "email" => {
            // Email channel: key is the sender email address.
            // All emails from the same sender continue the same session
            // until the gap timeout (24h default) expires.
            let key = msg
                .channel_session_key
                .clone()
                .unwrap_or_else(|| "unknown@sender".into());
            (Channel::Email, key)
        }
        "slack" => {
            // Slack channel: key is "{team_id}:{channel_id}:{thread_ts}"
            // so each Slack thread is a distinct, continuous session.
            // Falls back to user_id if the handler forgot to set the key.
            let key = msg
                .channel_session_key
                .clone()
                .or_else(|| msg.user_id.clone())
                .unwrap_or_else(|| "default".into());
            (Channel::Slack, key)
        }
        _ => {
            // "main", "scheduler", or any unknown → explicit Main session
            let key = msg
                .channel_session_key
                .clone()
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
        assert!(ctx.contains("Nova"));

        // Vault should work
        // Skills dir should exist
        assert!(tmp.path().join("skills").exists());

        // Core db should exist in db/
        assert!(tmp.path().join("db").join("core.db").exists());
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
            connectors_dir: agent_home.join("connectors"),
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
        assert!(ctx.contains("TestBot") || ctx.contains("Nova"));

        // DB dir should have core.db (unified sessions + cron + auth)
        assert!(db_dir.join("core.db").exists());
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
        std::fs::write(
            skill_a.join("SKILL.md"),
            "---\nname: alpha\ndescription: A\n---\nBody A",
        )
        .unwrap();
        std::fs::write(
            skill_b.join("SKILL.md"),
            "---\nname: beta\ndescription: B\n---\nBody B",
        )
        .unwrap();

        let paths = ResolvedPaths {
            mode: starpod_core::Mode::SingleAgent {
                starpod_dir: agent_home.clone(),
            },
            agent_toml: agent_home.join("agent.toml"),
            agent_home: agent_home.clone(),
            config_dir: agent_home.clone(),
            db_dir: agent_home.join("db"),
            skills_dir: skills_dir.clone(),
            connectors_dir: agent_home.join("connectors"),
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
        assert!(names.contains(&"Attach"));
        assert!(names.contains(&"VaultGet"));
        assert!(names.contains(&"VaultList"));
        assert!(names.contains(&"VaultSet"));
        assert!(names.contains(&"VaultDelete"));
        // Connector tools
        assert!(names.contains(&"ConnectorList"));
        assert!(names.contains(&"ConnectorAdd"));
        assert!(names.contains(&"ConnectorRemove"));
        assert_eq!(defs.len(), 38);
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
            vault: None,
            user_md_limit: 4_000,
            memory_md_limit: 8_000,
            attachments: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            proxy_enabled: false,
            connector_store: None,
            connectors_dir: std::path::PathBuf::new(),
            oauth_proxy_url: None,
        };

        // Test MemorySearch
        let result = handle_custom_tool(
            &ctx,
            "MemorySearch",
            &serde_json::json!({"query": "Nova", "limit": 3}),
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

        let result = handle_custom_tool(&ctx, "SkillList", &serde_json::json!({})).await;
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

        let result = handle_custom_tool(&ctx, "CronList", &serde_json::json!({})).await;
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
        let result =
            handle_custom_tool(&ctx, "CronRun", &serde_json::json!({"name": "test-job"})).await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("Manual run recorded"));

        // Test CronRun on nonexistent job
        let result =
            handle_custom_tool(&ctx, "CronRun", &serde_json::json!({"name": "nope"})).await;
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
        let result =
            handle_custom_tool(&ctx, "CronRuns", &serde_json::json!({"name": "nope"})).await;
        assert!(result.is_some());
        assert!(result.unwrap().is_error);

        // Test HeartbeatWake (no heartbeat job exists, should error)
        let result =
            handle_custom_tool(&ctx, "HeartbeatWake", &serde_json::json!({"mode": "now"})).await;
        assert!(result.is_some());
        assert!(result.unwrap().is_error); // no __heartbeat__ job yet

        // Test HeartbeatWake with mode="next" (always succeeds)
        let result =
            handle_custom_tool(&ctx, "HeartbeatWake", &serde_json::json!({"mode": "next"})).await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Test HeartbeatWake with default mode (no mode specified)
        let result = handle_custom_tool(&ctx, "HeartbeatWake", &serde_json::json!({})).await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Create a heartbeat job, then test wake "now"
        ctx.cron
            .add_job_full(
                "__heartbeat__",
                "heartbeat prompt",
                &starpod_cron::Schedule::Cron {
                    expr: "0 */30 * * * *".into(),
                },
                false,
                None,
                3,
                7200,
                starpod_cron::SessionMode::Main,
                None,
            )
            .await
            .unwrap();

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
        let hb = ctx
            .cron
            .get_job_by_name("__heartbeat__")
            .await
            .unwrap()
            .unwrap();
        let now = chrono::Utc::now().timestamp();
        assert!(hb.next_run_at.unwrap() <= now + 2);

        // Test unknown tool
        let result = handle_custom_tool(&ctx, "UnknownTool", &serde_json::json!({})).await;
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
        let attachments = vec![Attachment {
            file_name: "photo.png".into(),
            mime_type: "image/png".into(),
            data: "base64data".into(),
        }];
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
        let attachments = vec![Attachment {
            file_name: "doc.pdf".into(),
            mime_type: "application/pdf".into(),
            data: "base64data".into(),
        }];
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

        assert_eq!(agent.config().agent_name, "Nova");

        let mut new_cfg = test_config(&tmp);
        new_cfg.agent_name = "Renamed".to_string();
        agent.reload_config(new_cfg);

        assert_eq!(agent.config().agent_name, "Renamed");
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
        let user_daily = tmp
            .path()
            .join("users")
            .join("bob")
            .join("memory")
            .join(format!("{}.md", today));
        assert!(
            user_daily.exists(),
            "Pre-compact daily log should be in user dir"
        );

        let content = std::fs::read_to_string(&user_daily).unwrap();
        assert!(content.contains("Pre-compaction save"));
        assert!(content.contains("Important context"));

        // Agent-level should NOT have it
        let agent_daily = tmp.path().join("memory").join(format!("{}.md", today));
        assert!(
            !agent_daily.exists(),
            "Pre-compact log should NOT be in agent-level dir"
        );
    }

    #[tokio::test]
    async fn test_append_daily_for_user_routes_to_user_dir() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Append with a user_id — should write to users/{id}/memory/
        agent
            .append_daily_for_user(Some("alice"), "Hello from Alice")
            .await
            .unwrap();

        let user_memory_dir = tmp.path().join("users").join("alice").join("memory");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let daily_file = user_memory_dir.join(format!("{}.md", today));
        assert!(daily_file.exists(), "Daily log should be in user dir");

        let content = std::fs::read_to_string(&daily_file).unwrap();
        assert!(content.contains("Hello from Alice"));

        // Agent-level memory dir should NOT have today's file
        let agent_daily = tmp.path().join("memory").join(format!("{}.md", today));
        assert!(
            !agent_daily.exists(),
            "Daily log should NOT be in agent-level dir"
        );
    }

    #[tokio::test]
    async fn test_append_daily_for_user_fallback_no_user() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Append with no user_id — should fall back to agent-level store
        agent
            .append_daily_for_user(None, "Agent-level entry")
            .await
            .unwrap();

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        // Should be in agent-level memory (the MemoryStore root)
        let content = agent
            .memory()
            .read_file(&format!("memory/{}.md", today))
            .unwrap();
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

    #[tokio::test]
    async fn test_bootstrap_cache_frozen_per_session() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();
        let config = agent.snapshot_config();
        let session_id = "test-session-1";

        // First call computes and caches bootstrap
        let prompt1 = agent
            .build_system_prompt(session_id, &config, None, None)
            .await
            .unwrap();
        assert!(prompt1.contains("SOUL.md"));

        // Mutate the SOUL.md on disk
        let soul_path = agent.paths.config_dir.join("SOUL.md");
        std::fs::write(&soul_path, "# Soul\nModified content").unwrap();

        // Second call for the SAME session returns the frozen snapshot
        let prompt2 = agent
            .build_system_prompt(session_id, &config, None, None)
            .await
            .unwrap();

        // The bootstrap portion should be identical (frozen)
        assert!(!prompt2.contains("Modified content"));

        // A DIFFERENT session gets the fresh (modified) content
        let prompt3 = agent
            .build_system_prompt("test-session-2", &config, None, None)
            .await
            .unwrap();
        assert!(prompt3.contains("Modified content"));
    }

    #[tokio::test]
    async fn test_bootstrap_cache_evicted_on_session_export() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.export_sessions = true;
        let agent = StarpodAgent::new(cfg).await.unwrap();
        let config = agent.snapshot_config();

        // Populate the cache for a session
        let session_id = "evict-test-session";
        let _ = agent
            .build_system_prompt(session_id, &config, None, None)
            .await
            .unwrap();

        // Verify cache is populated
        assert!(agent.bootstrap_cache.read().await.contains_key(session_id));

        // Export triggers eviction (will fail to find session in DB, but
        // the cache eviction still runs)
        agent.export_session_to_memory(session_id).await;

        // Cache entry should be gone
        assert!(!agent.bootstrap_cache.read().await.contains_key(session_id));
    }

    // ── resolve_background_model tests ──────────────────────────────

    #[test]
    fn resolve_background_model_strips_provider_prefix() {
        let cfg = StarpodConfig::default();
        let (provider, model) =
            resolve_background_model(Some("anthropic/claude-haiku-4-5-20251001"), &cfg);
        assert_eq!(provider, "anthropic");
        assert_eq!(model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn resolve_background_model_handles_different_provider() {
        let cfg = StarpodConfig::default(); // default provider is "anthropic"
        let (provider, model) = resolve_background_model(Some("openai/gpt-4o"), &cfg);
        assert_eq!(provider, "openai");
        assert_eq!(model, "gpt-4o");
    }

    #[test]
    fn resolve_background_model_bare_model_uses_default_provider() {
        let cfg = StarpodConfig::default();
        let (provider, model) = resolve_background_model(Some("claude-haiku-4-5-20251001"), &cfg);
        assert_eq!(
            provider,
            cfg.provider(),
            "bare model name should use default provider"
        );
        assert_eq!(model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn resolve_background_model_none_falls_back_to_default() {
        let mut cfg = StarpodConfig::default();
        cfg.models = vec!["anthropic/claude-sonnet-4-6".to_string()];
        let (provider, model) = resolve_background_model(None, &cfg);
        assert_eq!(provider, "anthropic");
        assert_eq!(model, "claude-sonnet-4-6");
    }

    #[test]
    fn resolve_background_model_vertex_provider() {
        let cfg = StarpodConfig::default();
        let (provider, model) =
            resolve_background_model(Some("vertex/claude-haiku-4-5-20251001"), &cfg);
        assert_eq!(provider, "vertex");
        assert_eq!(model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn resolve_background_model_bedrock_provider() {
        let cfg = StarpodConfig::default();
        let (provider, model) = resolve_background_model(
            Some("bedrock/us.anthropic.claude-haiku-4-5-20251001-v1:0"),
            &cfg,
        );
        assert_eq!(provider, "bedrock");
        assert_eq!(model, "us.anthropic.claude-haiku-4-5-20251001-v1:0");
    }

    // ── nudge counter and flush_stale_sessions tests ────────────────

    #[tokio::test]
    async fn nudge_counter_stores_user_id() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Manually insert a counter entry
        agent
            .nudge_counters
            .write()
            .await
            .insert("sess-1".into(), ("alice".into(), 3));

        let counters = agent.nudge_counters.read().await;
        let (uid, count) = counters.get("sess-1").unwrap();
        assert_eq!(uid, "alice");
        assert_eq!(*count, 3);
    }

    #[tokio::test]
    async fn flush_stale_sessions_finds_stale_for_same_user() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.nudge_interval = 10;
        let agent = StarpodAgent::new(cfg).await.unwrap();

        // Populate counters: 2 sessions for alice, 1 for bob
        {
            let mut counters = agent.nudge_counters.write().await;
            counters.insert("sess-a1".into(), ("alice".into(), 3)); // stale (3 < 10)
            counters.insert("sess-a2".into(), ("alice".into(), 5)); // stale (5 < 10)
            counters.insert("sess-b1".into(), ("bob".into(), 7)); // different user
        }

        // Flush for alice, current session is sess-a2
        let config = agent.snapshot_config();
        agent
            .flush_stale_sessions("sess-a2", "alice", &config)
            .await;

        // sess-a1 should have been reset to 0 (flushed)
        // sess-a2 is current, should be untouched
        // sess-b1 belongs to bob, should be untouched
        let counters = agent.nudge_counters.read().await;
        assert_eq!(
            counters.get("sess-a1").unwrap().1,
            0,
            "sess-a1 should be reset after flush"
        );
        assert_eq!(
            counters.get("sess-a2").unwrap().1,
            5,
            "current session should be untouched"
        );
        assert_eq!(
            counters.get("sess-b1").unwrap().1,
            7,
            "other user's session should be untouched"
        );
    }

    #[tokio::test]
    async fn flush_stale_sessions_skips_sessions_at_interval_boundary() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.nudge_interval = 10;
        let agent = StarpodAgent::new(cfg).await.unwrap();

        {
            let mut counters = agent.nudge_counters.write().await;
            counters.insert("sess-1".into(), ("alice".into(), 10)); // already nudged (10 % 10 == 0)
            counters.insert("sess-2".into(), ("alice".into(), 20)); // already nudged (20 % 10 == 0)
            counters.insert("sess-3".into(), ("alice".into(), 7)); // stale
        }

        let config = agent.snapshot_config();
        agent
            .flush_stale_sessions("sess-new", "alice", &config)
            .await;

        let counters = agent.nudge_counters.read().await;
        assert_eq!(
            counters.get("sess-1").unwrap().1,
            10,
            "at interval boundary, should not flush"
        );
        assert_eq!(
            counters.get("sess-2").unwrap().1,
            20,
            "at interval boundary, should not flush"
        );
        assert_eq!(
            counters.get("sess-3").unwrap().1,
            0,
            "stale session should be flushed"
        );
    }

    #[tokio::test]
    async fn flush_stale_sessions_skips_zero_count() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.nudge_interval = 10;
        let agent = StarpodAgent::new(cfg).await.unwrap();

        {
            let mut counters = agent.nudge_counters.write().await;
            counters.insert("sess-1".into(), ("alice".into(), 0)); // already flushed
        }

        let config = agent.snapshot_config();
        agent
            .flush_stale_sessions("sess-new", "alice", &config)
            .await;

        // count 0 should remain 0 (not re-flushed)
        let counters = agent.nudge_counters.read().await;
        assert_eq!(counters.get("sess-1").unwrap().1, 0);
    }

    #[tokio::test]
    async fn flush_stale_sessions_noop_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.nudge_interval = 0; // disabled
        let agent = StarpodAgent::new(cfg).await.unwrap();

        {
            let mut counters = agent.nudge_counters.write().await;
            counters.insert("sess-1".into(), ("alice".into(), 5));
        }

        let config = agent.snapshot_config();
        agent
            .flush_stale_sessions("sess-new", "alice", &config)
            .await;

        // Should not touch anything when nudge is disabled
        let counters = agent.nudge_counters.read().await;
        assert_eq!(counters.get("sess-1").unwrap().1, 5);
    }

    #[tokio::test]
    async fn flush_stale_sessions_noop_when_no_other_sessions() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.nudge_interval = 10;
        let agent = StarpodAgent::new(cfg).await.unwrap();

        {
            let mut counters = agent.nudge_counters.write().await;
            counters.insert("sess-current".into(), ("alice".into(), 3));
        }

        let config = agent.snapshot_config();
        agent
            .flush_stale_sessions("sess-current", "alice", &config)
            .await;

        // Current session should be untouched
        let counters = agent.nudge_counters.read().await;
        assert_eq!(counters.get("sess-current").unwrap().1, 3);
    }

    #[tokio::test]
    async fn flush_stale_sessions_prevents_double_flush() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = test_config(&tmp);
        cfg.memory.nudge_interval = 10;
        let agent = StarpodAgent::new(cfg).await.unwrap();

        {
            let mut counters = agent.nudge_counters.write().await;
            counters.insert("sess-old".into(), ("alice".into(), 3));
        }

        let config = agent.snapshot_config();

        // First flush resets counter to 0
        agent
            .flush_stale_sessions("sess-new", "alice", &config)
            .await;
        assert_eq!(
            agent.nudge_counters.read().await.get("sess-old").unwrap().1,
            0
        );

        // Second flush should be a no-op (count is 0, so filter excludes it)
        agent
            .flush_stale_sessions("sess-another", "alice", &config)
            .await;
        assert_eq!(
            agent.nudge_counters.read().await.get("sess-old").unwrap().1,
            0
        );
    }

    #[tokio::test]
    async fn export_session_evicts_counter_with_user_id() {
        let tmp = TempDir::new().unwrap();
        let agent = StarpodAgent::new(test_config(&tmp)).await.unwrap();

        // Insert a counter entry
        agent
            .nudge_counters
            .write()
            .await
            .insert("sess-export".into(), ("alice".into(), 5));

        // Export session (will fail to find session in DB, but counter eviction still runs)
        agent.export_session_to_memory("sess-export").await;

        // Counter should be evicted
        assert!(
            !agent
                .nudge_counters
                .read()
                .await
                .contains_key("sess-export"),
            "Counter should be evicted after session export"
        );
    }
}
