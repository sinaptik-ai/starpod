use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

/// Warn (and ignore) any `api_key` or `bot_token` found in a raw TOML value tree.
///
/// Credentials must live in `.env`, not in config files. This function inspects
/// the parsed TOML before it is deserialized so that unknown/ignored keys still
/// trigger a user-visible warning.
pub fn warn_credentials_in_toml(value: &toml::Value, file_label: &str) {
    if let Some(table) = value.as_table() {
        // Check providers.*.api_key
        if let Some(providers) = table.get("providers").and_then(|v| v.as_table()) {
            for (name, provider) in providers {
                if let Some(ptable) = provider.as_table() {
                    if ptable.contains_key("api_key") {
                        warn!(
                            file = file_label,
                            provider = name,
                            "api_key in [providers.{name}] is ignored — \
                             set it via Settings or the vault",
                        );
                    }
                }
            }
        }
        // Check channels.telegram.bot_token
        if let Some(channels) = table.get("channels").and_then(|v| v.as_table()) {
            if let Some(telegram) = channels.get("telegram").and_then(|v| v.as_table()) {
                if telegram.contains_key("bot_token") {
                    warn!(
                        file = file_label,
                        "bot_token in [channels.telegram] is ignored — \
                         set it via Settings or the vault"
                    );
                }
            }
        }
    }
}

/// Deep-merge `overlay` into `base`. Keys in `overlay` take precedence.
pub fn deep_merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, value) in overlay_table {
                match base_table.entry(key) {
                    toml::map::Entry::Occupied(mut e) => {
                        deep_merge(e.get_mut(), value);
                    }
                    toml::map::Entry::Vacant(e) => {
                        e.insert(value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

// ── Sub-config types ─────────────────────────────────────────────────────

/// Reasoning effort level for models that support extended thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

/// How followup messages are handled when they arrive during an active agent loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FollowupMode {
    /// Inject followup messages into the next iteration of the running agent loop.
    #[default]
    Inject,
    /// Queue followup messages and start a new agent loop after the current one finishes.
    Queue,
}

/// Configuration for a single LLM provider.
///
/// **Credentials belong in `.env`, not here.** Use the conventional env var
/// for each provider (e.g. `ANTHROPIC_API_KEY`). Any `api_key` found in a
/// config file is ignored and triggers a warning at load time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// Whether this provider is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Override API endpoint.
    pub base_url: Option<String>,
    /// Preferred models shown first.
    #[serde(default)]
    pub models: Vec<String>,
    /// Provider-specific options merged into every request body.
    ///
    /// For Ollama: `keep_alive`, `num_ctx`, etc.
    #[serde(default)]
    pub options: serde_json::Map<String, serde_json::Value>,
}

fn default_true() -> bool {
    true
}

/// Multi-provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub anthropic: Option<ProviderConfig>,
    pub bedrock: Option<ProviderConfig>,
    pub vertex: Option<ProviderConfig>,
    pub openai: Option<ProviderConfig>,
    pub gemini: Option<ProviderConfig>,
    pub groq: Option<ProviderConfig>,
    pub deepseek: Option<ProviderConfig>,
    pub openrouter: Option<ProviderConfig>,
    pub ollama: Option<ProviderConfig>,
}

/// Telegram channel configuration (lives under `[channels.telegram]`).
///
/// **The bot token belongs in `.env` as `TELEGRAM_BOT_TOKEN`, not here.**
/// Any `bot_token` found in a config file is ignored and triggers a warning.
///
/// Telegram user access is now controlled via the `starpod-auth` crate
/// (database-backed user management with Telegram account linking),
/// not via config-file allow-lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramChannelConfig {
    /// Whether this channel is enabled (default: false).
    pub enabled: bool,
    /// Inactivity gap (in minutes) before auto-closing a Telegram session (default: 360 = 6h).
    #[serde(default = "default_gap_minutes")]
    pub gap_minutes: Option<i64>,
    /// Message mode: "final_only" (default) sends only the last assistant
    /// message; "all_messages" sends each assistant message as a standalone
    /// Telegram message (tool-use messages are excluded).
    #[serde(default = "default_stream_mode")]
    pub stream_mode: String,
}

fn default_gap_minutes() -> Option<i64> {
    Some(360)
}

impl Default for TelegramChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gap_minutes: default_gap_minutes(),
            stream_mode: default_stream_mode(),
        }
    }
}

/// Authentication and rate-limiting configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// Maximum requests per user per window (0 = disabled).
    pub rate_limit_requests: u32,
    /// Rate-limit window in seconds.
    pub rate_limit_window_secs: u64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            rate_limit_requests: 0, // disabled by default
            rate_limit_window_secs: 60,
        }
    }
}

/// Email channel configuration (lives under `[channels.email]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailChannelConfig {
    /// Whether this channel is enabled (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Inactivity gap (in minutes) before auto-closing an email session (default: 1440 = 24h).
    #[serde(default = "default_email_gap_minutes")]
    pub gap_minutes: Option<i64>,
}

fn default_email_gap_minutes() -> Option<i64> {
    Some(1440)
}

impl Default for EmailChannelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gap_minutes: default_email_gap_minutes(),
        }
    }
}

/// Internet access settings for web search and fetch tools.
///
/// # Example
///
/// ```toml
/// [internet]
/// enabled = true
/// ```
///
/// The Brave Search API key should be set in `.env` as `BRAVE_API_KEY`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InternetConfig {
    /// Whether web search and fetch tools are enabled.
    pub enabled: bool,
    /// Maximum response body size in bytes for WebFetch (default: 2 MiB).
    ///
    /// This is the raw HTTP response limit applied *before* any HTML processing.
    /// A larger limit gives readability extraction more content to work with.
    pub max_fetch_bytes: usize,
    /// Maximum extracted text length in characters (default: 50 000).
    ///
    /// Applied *after* readability extraction and markdown conversion.
    /// This is the final size guard before content enters the agent's context.
    pub max_text_chars: usize,
    /// Request timeout in seconds (default: 15).
    pub timeout_secs: u64,
}

impl Default for InternetConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_fetch_bytes: 2 * 1024 * 1024,
            max_text_chars: 50_000,
            timeout_secs: 15,
        }
    }
}

/// Channel configuration namespace (`[channels.*]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelsConfig {
    /// Telegram channel settings.
    pub telegram: Option<TelegramChannelConfig>,
    /// Email channel settings.
    pub email: Option<EmailChannelConfig>,
}

fn default_stream_mode() -> String {
    "final_only".to_string()
}

/// Memory search configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Half-life in days for temporal decay on daily logs (default: 30.0).
    pub half_life_days: f64,
    /// MMR lambda: 0.0 = max diversity, 1.0 = pure relevance (default: 0.7).
    pub mmr_lambda: f64,
    /// Enable vector search (requires `embeddings` feature). Default: true.
    pub vector_search: bool,
    /// Target chunk size in characters for indexing (~400 tokens ≈ 1600 chars).
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    /// Overlap in characters between chunks (~80 tokens ≈ 320 chars).
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    /// Maximum characters to include from a single file in bootstrap context.
    #[serde(default = "default_bootstrap_file_cap")]
    pub bootstrap_file_cap: usize,
    /// Export closed session transcripts to knowledge/sessions/ for long-term recall (default: true).
    #[serde(default = "default_true")]
    pub export_sessions: bool,
    /// Automatically append a truncated summary of each turn to the daily log (default: false).
    /// With memory flush enabled, this is redundant and off by default.
    #[serde(default)]
    pub auto_log: bool,
    /// Soft character limit for USER.md (default: 4000). Writes that would exceed
    /// this return a warning asking the agent to consolidate before retrying.
    #[serde(default = "default_user_md_limit")]
    pub user_md_limit: usize,
    /// Soft character limit for MEMORY.md (default: 8000).
    #[serde(default = "default_memory_md_limit")]
    pub memory_md_limit: usize,
    /// Run a background memory review every N user messages (default: 10, 0 = disabled).
    ///
    /// When enabled, a lightweight LLM call reviews the recent conversation every
    /// `nudge_interval` user messages and persists important information to memory
    /// (USER.md, MEMORY.md, or daily logs) without interrupting the main chat flow.
    #[serde(default = "default_nudge_interval")]
    pub nudge_interval: u32,
    /// Model to use for background memory nudges. Falls back to `compaction.flush_model`,
    /// then `compaction_model`, then the primary model.
    pub nudge_model: Option<String>,
}

fn default_chunk_size() -> usize {
    1600
}
fn default_chunk_overlap() -> usize {
    320
}
fn default_bootstrap_file_cap() -> usize {
    20_000
}
fn default_user_md_limit() -> usize {
    4_000
}
fn default_memory_md_limit() -> usize {
    8_000
}
fn default_nudge_interval() -> u32 {
    10
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            half_life_days: 30.0,
            mmr_lambda: 0.7,
            vector_search: true,
            chunk_size: default_chunk_size(),
            chunk_overlap: default_chunk_overlap(),
            bootstrap_file_cap: default_bootstrap_file_cap(),
            export_sessions: true,
            auto_log: false,
            user_md_limit: default_user_md_limit(),
            memory_md_limit: default_memory_md_limit(),
            nudge_interval: default_nudge_interval(),
            nudge_model: None,
        }
    }
}

/// Cron scheduling configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CronConfig {
    /// Default maximum retries for failed jobs (default: 3).
    #[serde(default = "default_cron_max_retries")]
    pub default_max_retries: u32,
    /// Default job timeout in seconds (default: 7200 = 2h).
    #[serde(default = "default_cron_timeout_secs")]
    pub default_timeout_secs: u64,
    /// Maximum concurrent job runs (default: 1).
    #[serde(default = "default_cron_max_concurrent")]
    pub max_concurrent_runs: usize,
    /// Heartbeat interval in minutes (default: 30).
    /// Controls how often the HEARTBEAT.md prompt is executed.
    #[serde(default = "default_heartbeat_interval_minutes")]
    pub heartbeat_interval_minutes: u32,
}

fn default_cron_max_retries() -> u32 {
    3
}
fn default_cron_timeout_secs() -> u64 {
    7200
}
fn default_cron_max_concurrent() -> usize {
    1
}
fn default_heartbeat_interval_minutes() -> u32 {
    30
}

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            default_max_retries: default_cron_max_retries(),
            default_timeout_secs: default_cron_timeout_secs(),
            max_concurrent_runs: default_cron_max_concurrent(),
            heartbeat_interval_minutes: default_heartbeat_interval_minutes(),
        }
    }
}

/// Conversation compaction configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactionConfig {
    /// Token budget triggering compaction (~80% of model context window).
    #[serde(default = "default_context_budget")]
    pub context_budget: u64,
    /// Max tokens for the compaction summary response.
    #[serde(default = "default_summary_max_tokens")]
    pub summary_max_tokens: u32,
    /// Minimum messages to keep at the end (never compact below this).
    #[serde(default = "default_min_keep_messages")]
    pub min_keep_messages: usize,
    /// Run a silent agentic turn before compaction to persist important memories (default: true).
    #[serde(default = "default_true")]
    pub memory_flush: bool,
    /// Model to use for the memory flush turn.
    /// Falls back to `compaction_model` then the primary model if not set.
    #[serde(default)]
    pub flush_model: Option<String>,
    /// Maximum size in bytes for any single tool result (default: 50 000).
    ///
    /// Applied to all tool results before they enter the conversation.
    /// Also strips base64 data URIs and hex blobs.
    #[serde(default = "default_max_tool_result_bytes")]
    pub max_tool_result_bytes: usize,
    /// Percentage of context_budget at which lightweight tool-result pruning triggers (default: 70).
    #[serde(default = "default_prune_threshold_pct")]
    pub prune_threshold_pct: u8,
    /// Tool results longer than this (in chars) are candidates for pruning (default: 2000).
    #[serde(default = "default_prune_tool_result_max_chars")]
    pub prune_tool_result_max_chars: usize,
}

fn default_context_budget() -> u64 {
    160_000
}
fn default_summary_max_tokens() -> u32 {
    4096
}
fn default_min_keep_messages() -> usize {
    4
}
fn default_max_tool_result_bytes() -> usize {
    50_000
}
fn default_prune_threshold_pct() -> u8 {
    70
}
fn default_prune_tool_result_max_chars() -> usize {
    2_000
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            context_budget: default_context_budget(),
            summary_max_tokens: default_summary_max_tokens(),
            min_keep_messages: default_min_keep_messages(),
            memory_flush: true,
            flush_model: None,
            max_tool_result_bytes: default_max_tool_result_bytes(),
            prune_threshold_pct: default_prune_threshold_pct(),
            prune_tool_result_max_chars: default_prune_tool_result_max_chars(),
        }
    }
}

/// Browser automation configuration (beta).
///
/// Uses Lightpanda (a lightweight headless browser) for CDP-based web
/// browsing. Currently in beta — works well for server-rendered pages but
/// does not reliably render JavaScript-heavy SPAs (Angular, React, Vue).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// Whether browser tools are enabled (default: false — beta feature).
    #[serde(default)]
    pub enabled: bool,

    /// CDP endpoint URL. When set, the agent connects to this existing
    /// CDP endpoint instead of auto-spawning Lightpanda.
    /// Example: `ws://127.0.0.1:9222`
    #[serde(default)]
    pub cdp_url: Option<String>,

    /// Timeout in seconds for waiting for the browser process to start
    /// (only used in auto-spawn mode). Default: 10.
    #[serde(default = "default_browser_startup_timeout")]
    pub startup_timeout_secs: u64,
}

fn default_browser_startup_timeout() -> u64 {
    10
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cdp_url: None,
            startup_timeout_secs: default_browser_startup_timeout(),
        }
    }
}

/// Attachment handling configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AttachmentsConfig {
    /// Whether file attachments are accepted (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Allowed file extensions (e.g. ["jpg", "png", "pdf"]).
    /// Empty list means all extensions are allowed.
    #[serde(default)]
    pub allowed_extensions: Vec<String>,

    /// Maximum file size in bytes (default: 20 MB).
    #[serde(default = "default_max_file_size")]
    pub max_file_size: usize,
}

fn default_max_file_size() -> usize {
    20 * 1024 * 1024 // 20 MB
}

impl Default for AttachmentsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_extensions: Vec::new(),
            max_file_size: default_max_file_size(),
        }
    }
}

impl AttachmentsConfig {
    /// Check whether an attachment is allowed by this config.
    /// Returns `Ok(())` if allowed, or `Err(reason)` if rejected.
    pub fn validate(&self, file_name: &str, raw_size: usize) -> Result<(), String> {
        if !self.enabled {
            return Err("Attachments are disabled".to_string());
        }

        if raw_size > self.max_file_size {
            return Err(format!(
                "File '{}' exceeds {:.1} MB limit ({:.1} MB)",
                file_name,
                self.max_file_size as f64 / 1_048_576.0,
                raw_size as f64 / 1_048_576.0,
            ));
        }

        if !self.allowed_extensions.is_empty() {
            let ext = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
            if !self
                .allowed_extensions
                .iter()
                .any(|e| e.to_lowercase() == ext)
            {
                return Err(format!(
                    "File extension '{}' is not allowed (allowed: {})",
                    ext,
                    self.allowed_extensions.join(", "),
                ));
            }
        }

        Ok(())
    }
}

// ── Main config ──────────────────────────────────────────────────────────

/// Main configuration for Starpod, loaded from `.starpod/config.toml` in the current directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarpodConfig {
    /// Database directory for SQLite DBs (default: `.starpod/db`)
    #[serde(default)]
    pub db_dir: PathBuf,

    /// Path to the SQLite database (default: `<db_dir>/memory.db`)
    #[serde(default)]
    pub db_path: Option<PathBuf>,

    /// Server bind address (default: `127.0.0.1:3000`)
    #[serde(default = "default_server_addr")]
    pub server_addr: String,

    /// Allowed models in `"provider/model"` format (e.g. `"anthropic/claude-sonnet-4-6"`).
    /// The first entry is the default. Must contain at least one entry.
    #[serde(default = "default_models")]
    pub models: Vec<String>,

    /// Maximum agentic turns per request
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Reasoning effort for extended thinking (low, medium, high).
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,

    /// Compaction model in `"provider/model"` format.
    /// Defaults to the primary model if not set.
    #[serde(default)]
    pub compaction_model: Option<String>,

    /// Agent display name (default: "Nova").
    /// Used in CLI headers, daily logs, and Telegram display.
    /// Personality and soul live in SOUL.md; user profile in USER.md.
    #[serde(default = "default_agent_name")]
    pub agent_name: String,

    /// User's timezone (IANA format, e.g. "Europe/Rome").
    /// Used for cron scheduling. User profile details live in USER.md.
    #[serde(default)]
    pub timezone: Option<String>,

    /// Multi-provider configuration.
    #[serde(default)]
    pub providers: ProvidersConfig,

    /// Channel configurations (e.g. `[channels.telegram]`).
    #[serde(default)]
    pub channels: ChannelsConfig,

    /// Maximum tokens for LLM API responses (default: 16384).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    /// How followup messages are handled during an active agent loop.
    /// "inject" (default) integrates them into the next loop iteration;
    /// "queue" buffers them and starts a new loop after the current one finishes.
    #[serde(default)]
    pub followup_mode: FollowupMode,

    /// Memory search tuning.
    #[serde(default)]
    pub memory: MemoryConfig,

    /// Cron scheduling settings.
    #[serde(default)]
    pub cron: CronConfig,

    /// Conversation compaction settings.
    #[serde(default)]
    pub compaction: CompactionConfig,

    /// Browser automation settings.
    #[serde(default)]
    pub browser: BrowserConfig,

    /// Attachment handling settings.
    #[serde(default)]
    pub attachments: AttachmentsConfig,

    /// Authentication settings.
    #[serde(default)]
    pub auth: AuthConfig,

    /// Internet access settings (web search & fetch).
    #[serde(default)]
    pub internet: InternetConfig,

    /// Self-improve mode (beta): when enabled, the agent proactively creates
    /// skills from complex tasks and updates outdated skills during use.
    #[serde(default)]
    pub self_improve: bool,

    /// The project root directory (not serialized — set at load time).
    #[serde(skip)]
    pub project_root: PathBuf,
}

/// Frontend configuration loaded from `frontend.toml`.
///
/// Controls the web UI welcome screen. Both fields are optional — if the file
/// is missing or empty, the frontend falls back to defaults (`"ready_"` greeting,
/// no prompt chips).
///
/// # Example
///
/// ```toml
/// greeting = "Hi! I'm Nova."
///
/// prompts = [
///     "What can you help me with?",
///     "What do you remember about me?",
/// ]
/// ```
///
/// The gateway reads this file on every page load and injects it into the HTML
/// as `window.__STARPOD__`, so changes take effect on the next browser refresh
/// without restarting the server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrontendConfig {
    /// Custom greeting text shown below the logo on the welcome screen.
    #[serde(default)]
    pub greeting: Option<String>,

    /// Suggested prompt chips shown on the welcome screen.
    #[serde(default)]
    pub prompts: Vec<String>,
}

impl FrontendConfig {
    /// Load from `config_dir/frontend.toml`, returning `Default` if missing or malformed.
    pub fn load(config_dir: &Path) -> Self {
        let path = config_dir.join("frontend.toml");
        match std::fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

fn default_server_addr() -> String {
    "127.0.0.1:3000".to_string()
}

fn default_models() -> Vec<String> {
    vec!["anthropic/claude-haiku-4-5".to_string()]
}

fn default_max_turns() -> u32 {
    30
}

fn default_max_tokens() -> u32 {
    16384
}

fn default_agent_name() -> String {
    "Nova".to_string()
}

impl Default for StarpodConfig {
    fn default() -> Self {
        Self {
            db_dir: PathBuf::new(),
            db_path: None,
            server_addr: default_server_addr(),
            models: default_models(),
            max_turns: default_max_turns(),
            max_tokens: default_max_tokens(),
            reasoning_effort: None,
            compaction_model: None,
            followup_mode: FollowupMode::default(),
            memory: MemoryConfig::default(),
            cron: CronConfig::default(),
            compaction: CompactionConfig::default(),
            agent_name: default_agent_name(),
            timezone: None,
            providers: ProvidersConfig::default(),
            channels: ChannelsConfig::default(),
            browser: BrowserConfig::default(),
            attachments: AttachmentsConfig::default(),
            auth: AuthConfig::default(),
            internet: InternetConfig::default(),
            self_improve: false,
            project_root: PathBuf::new(),
        }
    }
}

/// Parse a `"provider/model"` string into `(provider, model)`.
///
/// Returns `None` if the string contains no `/`.
///
/// ```
/// use starpod_core::parse_model_spec;
/// assert_eq!(parse_model_spec("anthropic/claude-sonnet-4-6"), Some(("anthropic", "claude-sonnet-4-6")));
/// assert_eq!(parse_model_spec("gpt-4o"), None);
/// ```
pub fn parse_model_spec(spec: &str) -> Option<(&str, &str)> {
    spec.split_once('/')
}

impl StarpodConfig {
    /// Default (provider, model) — first entry in `models`.
    pub fn default_model(&self) -> (&str, &str) {
        self.models
            .first()
            .and_then(|s| parse_model_spec(s))
            .unwrap_or(("anthropic", "claude-haiku-4-5"))
    }

    /// Default provider name (from the first model entry).
    pub fn provider(&self) -> &str {
        self.default_model().0
    }

    /// Default model name (from the first model entry).
    pub fn model(&self) -> &str {
        self.default_model().1
    }

    /// Resolve a model override against the allowed list.
    /// Returns `(provider, model)`. If `override_spec` is `None`, returns the default.
    /// If the override is not in the allowed list, returns an error.
    pub fn resolve_model(&self, override_spec: Option<&str>) -> Result<(String, String), String> {
        match override_spec {
            None => {
                let (p, m) = self.default_model();
                Ok((p.to_string(), m.to_string()))
            }
            Some(spec) => {
                if self.models.iter().any(|m| m == spec) {
                    match parse_model_spec(spec) {
                        Some((p, m)) => Ok((p.to_string(), m.to_string())),
                        None => Err(format!("invalid model spec: {spec}")),
                    }
                } else {
                    Err(format!("model {spec} is not in the allowed list"))
                }
            }
        }
    }

    /// Resolve the compaction model. Returns `(provider, model)`.
    /// Falls back to the primary model if not set.
    pub fn resolve_compaction_model(&self) -> (String, String) {
        if let Some(ref spec) = self.compaction_model {
            if let Some((p, m)) = parse_model_spec(spec) {
                return (p.to_string(), m.to_string());
            }
        }
        let (p, m) = self.default_model();
        (p.to_string(), m.to_string())
    }

    /// Resolved timezone: config value → system timezone fallback.
    pub fn resolved_timezone(&self) -> Option<String> {
        self.timezone
            .clone()
            .or_else(|| iana_time_zone::get_timezone().ok())
    }

    /// Resolved Anthropic API key from the `ANTHROPIC_API_KEY` env var.
    pub fn resolved_api_key(&self) -> Option<String> {
        std::env::var("ANTHROPIC_API_KEY").ok()
    }

    /// Resolved Telegram bot token from the `TELEGRAM_BOT_TOKEN` env var.
    pub fn resolved_telegram_token(&self) -> Option<String> {
        std::env::var("TELEGRAM_BOT_TOKEN").ok()
    }

    /// Get the inactivity gap (in minutes) for a channel by name.
    /// Returns `None` for channels that don't use time-gap sessions.
    pub fn channel_gap_minutes(&self, channel: &str) -> Option<i64> {
        match channel {
            "telegram" => self.channels.telegram.as_ref().and_then(|t| t.gap_minutes),
            "email" => self.channels.email.as_ref().and_then(|e| e.gap_minutes),
            _ => None,
        }
    }

    /// Resolved database path (uses `db_path` if set, otherwise `<db_dir>/memory.db`).
    pub fn resolved_db_path(&self) -> PathBuf {
        self.db_path
            .clone()
            .unwrap_or_else(|| self.db_dir.join("memory.db"))
    }

    /// Resolved API key for any provider from the conventional env var.
    ///
    /// Credentials must be set via environment variables (or `.env` file),
    /// never in config files.
    pub fn resolved_provider_api_key(&self, provider: &str) -> Option<String> {
        let env_var = match provider {
            "anthropic" => "ANTHROPIC_API_KEY",
            // Bedrock uses AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY (handled by the provider)
            "bedrock" => "AWS_ACCESS_KEY_ID",
            // Vertex uses Google ADC / service account (handled by the provider)
            "vertex" => "GOOGLE_APPLICATION_CREDENTIALS",
            "openai" => "OPENAI_API_KEY",
            "gemini" => "GEMINI_API_KEY",
            "groq" => "GROQ_API_KEY",
            "deepseek" => "DEEPSEEK_API_KEY",
            "openrouter" => "OPENROUTER_API_KEY",
            "ollama" => return Some(String::new()), // Ollama doesn't require a key
            _ => return None,
        };
        std::env::var(env_var).ok()
    }

    /// Resolved base URL for any provider.
    ///
    /// Checks `providers.<name>.base_url` in config, then returns the default
    /// endpoint for that provider.
    pub fn resolved_provider_base_url(&self, provider: &str) -> Option<String> {
        let cfg = match provider {
            "anthropic" => self.providers.anthropic.as_ref(),
            "bedrock" => self.providers.bedrock.as_ref(),
            "vertex" => self.providers.vertex.as_ref(),
            "openai" => self.providers.openai.as_ref(),
            "gemini" => self.providers.gemini.as_ref(),
            "groq" => self.providers.groq.as_ref(),
            "deepseek" => self.providers.deepseek.as_ref(),
            "openrouter" => self.providers.openrouter.as_ref(),
            "ollama" => self.providers.ollama.as_ref(),
            _ => None,
        };

        cfg.and_then(|c| c.base_url.clone()).or_else(|| {
            let default_url = match provider {
                "anthropic" => "https://api.anthropic.com/v1/messages",
                // Bedrock URL is constructed per-model by the provider; this is a sentinel
                "bedrock" => "https://bedrock-runtime.us-east-1.amazonaws.com",
                // Vertex URL is constructed per-model by the provider; this is a sentinel
                "vertex" => "https://us-central1-aiplatform.googleapis.com",
                "openai" => "https://api.openai.com/v1/chat/completions",
                "gemini" => "https://generativelanguage.googleapis.com/v1beta",
                "groq" => "https://api.groq.com/openai/v1/chat/completions",
                "deepseek" => "https://api.deepseek.com/v1/chat/completions",
                "openrouter" => "https://openrouter.ai/api/v1/chat/completions",
                "ollama" => "http://localhost:11434/v1/chat/completions",
                _ => return None,
            };
            Some(default_url.to_string())
        })
    }

    /// Return provider-specific options (empty map if none configured).
    pub fn provider_options(&self, provider: &str) -> &serde_json::Map<String, serde_json::Value> {
        static EMPTY: std::sync::LazyLock<serde_json::Map<String, serde_json::Value>> =
            std::sync::LazyLock::new(serde_json::Map::new);

        let cfg = match provider {
            "anthropic" => self.providers.anthropic.as_ref(),
            "bedrock" => self.providers.bedrock.as_ref(),
            "vertex" => self.providers.vertex.as_ref(),
            "openai" => self.providers.openai.as_ref(),
            "gemini" => self.providers.gemini.as_ref(),
            "groq" => self.providers.groq.as_ref(),
            "deepseek" => self.providers.deepseek.as_ref(),
            "openrouter" => self.providers.openrouter.as_ref(),
            "ollama" => self.providers.ollama.as_ref(),
            _ => None,
        };

        cfg.map(|c| &c.options).unwrap_or(&EMPTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegram_default() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.channels.telegram.is_none());
    }

    #[test]
    fn test_channels_telegram_enabled_and_gap_defaults() {
        let toml = r#"
            [channels.telegram]
            enabled = true
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let tg = config.channels.telegram.as_ref().unwrap();
        assert!(tg.enabled);
        assert_eq!(tg.gap_minutes, Some(360));
    }

    #[test]
    fn test_channel_gap_minutes_convenience() {
        let toml = r#"
            [channels.telegram]
            gap_minutes = 120
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.channel_gap_minutes("telegram"), Some(120));
        assert_eq!(config.channel_gap_minutes("main"), None);
    }

    // All env-var API key tests are combined into a single test to avoid
    // parallel test races — std::env::set_var/remove_var are process-global.
    #[test]
    fn test_resolved_api_key_env_only() {
        // Credentials only come from env vars, never from config files.
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-from-env");
        let config: StarpodConfig = toml::from_str("").unwrap();
        assert_eq!(config.resolved_api_key().unwrap(), "sk-ant-from-env");

        std::env::remove_var("ANTHROPIC_API_KEY");
        assert!(config.resolved_api_key().is_none());
    }

    #[test]
    fn resolved_provider_api_key_from_env() {
        std::env::set_var("OPENAI_API_KEY", "sk-test-openai-key");
        let config = StarpodConfig::default();
        assert_eq!(
            config.resolved_provider_api_key("openai"),
            Some("sk-test-openai-key".to_string())
        );
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn resolved_provider_api_key_unknown_provider_returns_none() {
        let config = StarpodConfig::default();
        assert_eq!(
            config.resolved_provider_api_key("nonexistent_provider"),
            None
        );
    }

    #[test]
    fn resolved_provider_base_url_defaults() {
        let config = StarpodConfig::default();

        assert_eq!(
            config.resolved_provider_base_url("anthropic"),
            Some("https://api.anthropic.com/v1/messages".to_string())
        );
        assert_eq!(
            config.resolved_provider_base_url("openai"),
            Some("https://api.openai.com/v1/chat/completions".to_string())
        );
        assert_eq!(
            config.resolved_provider_base_url("gemini"),
            Some("https://generativelanguage.googleapis.com/v1beta".to_string())
        );
        assert_eq!(
            config.resolved_provider_base_url("groq"),
            Some("https://api.groq.com/openai/v1/chat/completions".to_string())
        );
        assert_eq!(
            config.resolved_provider_base_url("ollama"),
            Some("http://localhost:11434/v1/chat/completions".to_string())
        );
    }

    #[test]
    fn resolved_provider_base_url_unknown_returns_none() {
        let config = StarpodConfig::default();
        assert_eq!(
            config.resolved_provider_base_url("nonexistent_provider"),
            None
        );
    }

    #[test]
    fn resolved_provider_base_url_config_override() {
        let toml = r#"
            [providers.openai]
            base_url = "https://custom.openai.example.com/v1/chat"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.resolved_provider_base_url("openai"),
            Some("https://custom.openai.example.com/v1/chat".to_string())
        );
    }

    #[test]
    fn resolved_provider_api_key_ollama_returns_empty_string() {
        let config = StarpodConfig::default();
        // Ollama doesn't require an API key, returns empty string
        assert_eq!(
            config.resolved_provider_api_key("ollama"),
            Some(String::new())
        );
    }

    // ── Credential-in-config rejection tests ─────────────────────────────

    #[test]
    fn warn_credentials_in_toml_detects_api_key() {
        let value: toml::Value = toml::from_str(
            r#"
            [providers.anthropic]
            api_key = "sk-ant-bad"
            [providers.openai]
            base_url = "https://example.com"
        "#,
        )
        .unwrap();
        // Should not panic — warning goes to tracing. We just verify the
        // function runs without error on input containing credentials.
        warn_credentials_in_toml(&value, "test.toml");
    }

    #[test]
    fn warn_credentials_in_toml_detects_bot_token() {
        let value: toml::Value = toml::from_str(
            r#"
            [channels.telegram]
            bot_token = "123:ABC"
        "#,
        )
        .unwrap();
        warn_credentials_in_toml(&value, "test.toml");
    }

    #[test]
    fn warn_credentials_in_toml_clean_config_no_panic() {
        // A config with no credentials should pass through silently.
        let value: toml::Value = toml::from_str(
            r#"
            [providers.anthropic]
            base_url = "https://api.anthropic.com"
            [channels.telegram]
            gap_minutes = 360
        "#,
        )
        .unwrap();
        warn_credentials_in_toml(&value, "clean.toml");
    }

    #[test]
    fn resolved_api_key_ignores_config_reads_env_only() {
        // Even if a provider section exists, the resolved key comes from env.
        let config: StarpodConfig = toml::from_str(
            r#"
            [providers.anthropic]
            base_url = "https://custom.example.com"
        "#,
        )
        .unwrap();
        std::env::remove_var("ANTHROPIC_API_KEY");
        assert!(config.resolved_api_key().is_none());

        std::env::set_var("ANTHROPIC_API_KEY", "sk-from-env-only");
        assert_eq!(config.resolved_api_key().unwrap(), "sk-from-env-only");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn resolved_telegram_token_reads_env_only() {
        let config = StarpodConfig::default();
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        assert!(config.resolved_telegram_token().is_none());

        std::env::set_var("TELEGRAM_BOT_TOKEN", "123:from-env");
        assert_eq!(config.resolved_telegram_token().unwrap(), "123:from-env");
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
    }

    // ── Attachments config tests ─────────────────────────────────────────

    #[test]
    fn attachments_default_allows_everything() {
        let cfg = AttachmentsConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.allowed_extensions.is_empty());
        assert_eq!(cfg.max_file_size, 20 * 1024 * 1024);
        assert!(cfg.validate("anything.zip", 1024).is_ok());
    }

    #[test]
    fn attachments_disabled_rejects_all() {
        let cfg = AttachmentsConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(cfg.validate("photo.jpg", 100).is_err());
    }

    #[test]
    fn attachments_max_file_size_enforced() {
        let cfg = AttachmentsConfig {
            max_file_size: 1000,
            ..Default::default()
        };
        assert!(cfg.validate("small.txt", 999).is_ok());
        assert!(cfg.validate("small.txt", 1000).is_ok());
        assert!(cfg.validate("big.txt", 1001).is_err());
    }

    #[test]
    fn attachments_allowed_extensions_filter() {
        let cfg = AttachmentsConfig {
            allowed_extensions: vec!["jpg".into(), "png".into(), "pdf".into()],
            ..Default::default()
        };
        assert!(cfg.validate("photo.jpg", 100).is_ok());
        assert!(cfg.validate("photo.PNG", 100).is_ok()); // case-insensitive
        assert!(cfg.validate("doc.pdf", 100).is_ok());
        assert!(cfg.validate("script.exe", 100).is_err());
        assert!(cfg.validate("noext", 100).is_err());
    }

    #[test]
    fn attachments_empty_extensions_allows_all() {
        let cfg = AttachmentsConfig {
            allowed_extensions: vec![],
            ..Default::default()
        };
        assert!(cfg.validate("anything.exe", 100).is_ok());
        assert!(cfg.validate("noext", 100).is_ok());
    }

    #[test]
    fn attachments_from_toml() {
        let toml = r#"
            [attachments]
            enabled = true
            allowed_extensions = ["jpg", "png"]
            max_file_size = 5242880
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.attachments.enabled);
        assert_eq!(config.attachments.allowed_extensions, vec!["jpg", "png"]);
        assert_eq!(config.attachments.max_file_size, 5 * 1024 * 1024);
    }

    #[test]
    fn attachments_from_toml_disabled() {
        let toml = r#"
            [attachments]
            enabled = false
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(!config.attachments.enabled);
    }

    #[test]
    fn attachments_default_when_missing_from_toml() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.attachments.enabled);
        assert!(config.attachments.allowed_extensions.is_empty());
        assert_eq!(config.attachments.max_file_size, 20 * 1024 * 1024);
    }

    // ── Memory config tests ─────────────────────────────────────────────

    #[test]
    fn memory_config_defaults() {
        let cfg = MemoryConfig::default();
        assert_eq!(cfg.half_life_days, 30.0);
        assert_eq!(cfg.mmr_lambda, 0.7);
        assert!(cfg.vector_search);
    }

    #[test]
    fn memory_config_default_when_missing_from_toml() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.memory.half_life_days, 30.0);
        assert_eq!(config.memory.mmr_lambda, 0.7);
        assert!(config.memory.vector_search);
    }

    #[test]
    fn memory_config_from_toml() {
        let toml = r#"
            [memory]
            half_life_days = 14.0
            mmr_lambda = 0.5
            vector_search = false
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.memory.half_life_days, 14.0);
        assert_eq!(config.memory.mmr_lambda, 0.5);
        assert!(!config.memory.vector_search);
    }

    #[test]
    fn memory_config_partial_from_toml() {
        // Only set half_life_days, rest should default
        let toml = r#"
            [memory]
            half_life_days = 7.0
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.memory.half_life_days, 7.0);
        assert_eq!(config.memory.mmr_lambda, 0.7); // default
        assert!(config.memory.vector_search); // default
    }

    // ── export_sessions tests ──────────────────────────────────────────

    #[test]
    fn test_export_sessions_defaults_true() {
        let cfg = MemoryConfig::default();
        assert!(
            cfg.export_sessions,
            "export_sessions should default to true"
        );
    }

    #[test]
    fn test_export_sessions_from_toml() {
        let toml = r#"
            [memory]
            export_sessions = false
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(
            !config.memory.export_sessions,
            "export_sessions should be false when set in TOML"
        );

        // Also verify true parses correctly
        let toml_true = r#"
            [memory]
            export_sessions = true
        "#;
        let config_true: StarpodConfig = toml::from_str(toml_true).unwrap();
        assert!(config_true.memory.export_sessions);
    }

    // ── nudge_interval tests ────────────────────────────────────────────

    #[test]
    fn nudge_interval_defaults_to_10() {
        let cfg = MemoryConfig::default();
        assert_eq!(cfg.nudge_interval, 10);
        assert!(cfg.nudge_model.is_none());
    }

    #[test]
    fn nudge_interval_from_toml() {
        let toml = r#"
            [memory]
            nudge_interval = 5
            nudge_model = "anthropic/claude-haiku-4-5-20251001"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.memory.nudge_interval, 5);
        assert_eq!(
            config.memory.nudge_model.as_deref(),
            Some("anthropic/claude-haiku-4-5-20251001")
        );
    }

    #[test]
    fn nudge_interval_zero_disables() {
        let toml = r#"
            [memory]
            nudge_interval = 0
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.memory.nudge_interval, 0);
    }

    #[test]
    fn nudge_interval_defaults_when_missing_from_toml() {
        let toml = r#"
            [memory]
            half_life_days = 14.0
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.memory.nudge_interval, 10,
            "nudge_interval should default to 10"
        );
        assert!(
            config.memory.nudge_model.is_none(),
            "nudge_model should default to None"
        );
    }

    #[test]
    fn nudge_model_none_when_absent_from_toml() {
        let toml = r#"
            [memory]
            nudge_interval = 5
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.memory.nudge_interval, 5);
        assert!(
            config.memory.nudge_model.is_none(),
            "nudge_model should be None when not in TOML"
        );
    }

    #[test]
    fn nudge_config_serialization_round_trip() {
        let cfg = MemoryConfig {
            nudge_interval: 20,
            nudge_model: Some("anthropic/claude-haiku-4-5-20251001".into()),
            ..MemoryConfig::default()
        };
        let serialized = toml::to_string(&cfg).unwrap();
        let deserialized: MemoryConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.nudge_interval, 20);
        assert_eq!(
            deserialized.nudge_model.as_deref(),
            Some("anthropic/claude-haiku-4-5-20251001")
        );
    }

    // ── Channel gap_minutes tests ──────────────────────────────────────

    #[test]
    fn channel_gap_minutes_default_when_telegram_missing() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        // No [channels.telegram] → None
        assert_eq!(config.channel_gap_minutes("telegram"), None);
    }

    #[test]
    fn channel_gap_minutes_default_when_telegram_present() {
        let toml = r#"
            [channels.telegram]
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.channel_gap_minutes("telegram"), Some(360));
    }

    #[test]
    fn channel_gap_minutes_custom_from_toml() {
        let toml = r#"
            [channels.telegram]
            gap_minutes = 60
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.channel_gap_minutes("telegram"), Some(60));
    }

    // ── Compaction config tests ────────────────────────────────────────

    #[test]
    fn compaction_config_defaults() {
        let cfg = CompactionConfig::default();
        assert_eq!(cfg.context_budget, 160_000);
        assert_eq!(cfg.summary_max_tokens, 4096);
        assert_eq!(cfg.min_keep_messages, 4);
        assert!(cfg.memory_flush, "memory_flush should default to true");
        assert!(
            cfg.flush_model.is_none(),
            "flush_model should default to None"
        );
        assert_eq!(cfg.max_tool_result_bytes, 50_000);
        assert_eq!(cfg.prune_threshold_pct, 70);
        assert_eq!(cfg.prune_tool_result_max_chars, 2_000);
    }

    #[test]
    fn compaction_config_default_when_missing_from_toml() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.compaction.context_budget, 160_000);
        assert_eq!(config.compaction.summary_max_tokens, 4096);
        assert_eq!(config.compaction.min_keep_messages, 4);
        assert!(config.compaction.memory_flush);
        assert!(config.compaction.flush_model.is_none());
        assert_eq!(config.compaction.max_tool_result_bytes, 50_000);
        assert_eq!(config.compaction.prune_threshold_pct, 70);
        assert_eq!(config.compaction.prune_tool_result_max_chars, 2_000);
    }

    #[test]
    fn compaction_config_from_toml() {
        let toml = r#"
            [compaction]
            context_budget = 80000
            summary_max_tokens = 2048
            min_keep_messages = 8
            max_tool_result_bytes = 75000
            prune_threshold_pct = 60
            prune_tool_result_max_chars = 5000
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.compaction.context_budget, 80_000);
        assert_eq!(config.compaction.summary_max_tokens, 2048);
        assert_eq!(config.compaction.min_keep_messages, 8);
        assert_eq!(config.compaction.max_tool_result_bytes, 75_000);
        assert_eq!(config.compaction.prune_threshold_pct, 60);
        assert_eq!(config.compaction.prune_tool_result_max_chars, 5_000);
    }

    #[test]
    fn compaction_config_partial_from_toml() {
        // Only set context_budget, rest should default
        let toml = r#"
            [compaction]
            context_budget = 100000
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.compaction.context_budget, 100_000);
        assert_eq!(config.compaction.summary_max_tokens, 4096); // default
        assert_eq!(config.compaction.min_keep_messages, 4); // default
        assert!(config.compaction.memory_flush); // default true
        assert_eq!(config.compaction.max_tool_result_bytes, 50_000); // default
        assert_eq!(config.compaction.prune_threshold_pct, 70); // default
        assert_eq!(config.compaction.prune_tool_result_max_chars, 2_000); // default
    }

    #[test]
    fn compaction_memory_flush_from_toml() {
        let toml = r#"
            [compaction]
            memory_flush = false
            flush_model = "claude-haiku-4-5-20251001"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(!config.compaction.memory_flush);
        assert_eq!(
            config.compaction.flush_model.as_deref(),
            Some("claude-haiku-4-5-20251001")
        );
    }

    #[test]
    fn memory_auto_log_defaults_false() {
        let config: StarpodConfig = toml::from_str("").unwrap();
        assert!(!config.memory.auto_log, "auto_log should default to false");
    }

    #[test]
    fn memory_auto_log_from_toml() {
        let toml = r#"
            [memory]
            auto_log = true
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.memory.auto_log);
    }

    // ── Cron config tests ──────────────────────────────────────────────

    #[test]
    fn cron_config_defaults() {
        let cfg = CronConfig::default();
        assert_eq!(cfg.default_max_retries, 3);
        assert_eq!(cfg.default_timeout_secs, 7200);
        assert_eq!(cfg.max_concurrent_runs, 1);
        assert_eq!(cfg.heartbeat_interval_minutes, 30);
    }

    #[test]
    fn cron_config_default_when_missing_from_toml() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.cron.default_max_retries, 3);
        assert_eq!(config.cron.default_timeout_secs, 7200);
        assert_eq!(config.cron.max_concurrent_runs, 1);
        assert_eq!(config.cron.heartbeat_interval_minutes, 30);
    }

    #[test]
    fn cron_config_from_toml() {
        let toml = r#"
            [cron]
            default_max_retries = 5
            default_timeout_secs = 3600
            max_concurrent_runs = 4
            heartbeat_interval_minutes = 15
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.cron.default_max_retries, 5);
        assert_eq!(config.cron.default_timeout_secs, 3600);
        assert_eq!(config.cron.max_concurrent_runs, 4);
        assert_eq!(config.cron.heartbeat_interval_minutes, 15);
    }

    #[test]
    fn cron_config_partial_from_toml() {
        // Only set default_max_retries, rest should default
        let toml = r#"
            [cron]
            default_max_retries = 10
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.cron.default_max_retries, 10);
        assert_eq!(config.cron.default_timeout_secs, 7200); // default
        assert_eq!(config.cron.max_concurrent_runs, 1); // default
        assert_eq!(config.cron.heartbeat_interval_minutes, 30); // default
    }

    // ── Instances config tests ─────────────────────────────────────────

    // ── max_tokens tests ───────────────────────────────────────────────

    #[test]
    fn max_tokens_default() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.max_tokens, 16384);
    }

    #[test]
    fn max_tokens_from_toml() {
        let toml = r#"
            max_tokens = 8192
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.max_tokens, 8192);
    }

    // ── agent_name and timezone tests ───────────────────────────────────

    #[test]
    fn agent_name_default() {
        let config: StarpodConfig = toml::from_str("").unwrap();
        assert_eq!(config.agent_name, "Nova");
    }

    #[test]
    fn agent_name_from_toml() {
        let toml = r#"agent_name = "Nova""#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.agent_name, "Nova");
    }

    #[test]
    fn timezone_default_is_none() {
        let config: StarpodConfig = toml::from_str("").unwrap();
        assert!(config.timezone.is_none());
    }

    #[test]
    fn timezone_from_toml() {
        let toml = r#"timezone = "Europe/Rome""#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.timezone.as_deref(), Some("Europe/Rome"));
    }

    // ── deep_merge tests ────────────────────────────────────────────────

    #[test]
    fn deep_merge_overlay_adds_new_keys() {
        let mut base: toml::Value = toml::from_str(r#"models = ["anthropic/haiku"]"#).unwrap();
        let overlay: toml::Value = toml::from_str(r#"agent_name = "Nova""#).unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.models, vec!["anthropic/haiku"]);
        assert_eq!(config.agent_name, "Nova");
    }

    #[test]
    fn deep_merge_overlay_overrides_existing() {
        let mut base: toml::Value = toml::from_str(r#"models = ["anthropic/haiku"]"#).unwrap();
        let overlay: toml::Value = toml::from_str(r#"models = ["anthropic/sonnet"]"#).unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.models, vec!["anthropic/sonnet"]);
    }

    #[test]
    fn deep_merge_nested_tables() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [memory]
            half_life_days = 30.0
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            [memory]
            mmr_lambda = 0.5
            [channels.telegram]
            bot_token = "test"
        "#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.memory.half_life_days, 30.0); // kept from base
        assert_eq!(config.memory.mmr_lambda, 0.5); // from overlay
        assert!(config.channels.telegram.is_some()); // from overlay
    }

    // ── old identity/user sections are silently ignored ─────────────────

    // ── deep_merge edge cases ───────────────────────────────────────

    #[test]
    fn deep_merge_overlay_replaces_scalar_with_table() {
        // Edge case: base has a scalar, overlay has a table at the same key
        let mut base: toml::Value = toml::from_str(r#"memory = "flat""#).unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            [memory]
            half_life_days = 7.0
        "#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        // The table should win
        let table = base.get("memory").unwrap().as_table().unwrap();
        assert_eq!(
            table.get("half_life_days").unwrap().as_float().unwrap(),
            7.0
        );
    }

    #[test]
    fn deep_merge_empty_overlay_preserves_base() {
        let mut base: toml::Value = toml::from_str(
            r#"
            models = ["anthropic/haiku"]
            agent_name = "Nova"
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str("").unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.models, vec!["anthropic/haiku"]);
        assert_eq!(config.agent_name, "Nova");
    }

    #[test]
    fn deep_merge_instance_overrides_model_but_keeps_other_fields() {
        let mut base: toml::Value = toml::from_str(
            r#"
            models = ["anthropic/haiku"]
            max_turns = 30
            agent_name = "Nova"
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            models = ["anthropic/sonnet"]
            [channels.telegram]
            gap_minutes = 120
        "#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.models, vec!["anthropic/sonnet"]); // overridden
        assert_eq!(config.max_turns, 30); // preserved
        assert_eq!(config.agent_name, "Nova"); // preserved
        let tg = config.channels.telegram.unwrap();
        assert_eq!(tg.gap_minutes, Some(120)); // added
    }

    // ── FrontendConfig ──────────────────────────────────────────────

    #[test]
    fn frontend_config_full() {
        let toml = r#"
            greeting = "Hi! I'm Nova."
            prompts = ["What can you do?", "Tell me a joke"]
        "#;
        let config: FrontendConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.greeting.as_deref(), Some("Hi! I'm Nova."));
        assert_eq!(config.prompts, vec!["What can you do?", "Tell me a joke"]);
    }

    #[test]
    fn frontend_config_greeting_only() {
        let toml = r#"greeting = "Hello!""#;
        let config: FrontendConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.greeting.as_deref(), Some("Hello!"));
        assert!(config.prompts.is_empty());
    }

    #[test]
    fn frontend_config_prompts_only() {
        let toml = r#"prompts = ["one", "two"]"#;
        let config: FrontendConfig = toml::from_str(toml).unwrap();
        assert!(config.greeting.is_none());
        assert_eq!(config.prompts.len(), 2);
    }

    #[test]
    fn frontend_config_empty() {
        let config: FrontendConfig = toml::from_str("").unwrap();
        assert!(config.greeting.is_none());
        assert!(config.prompts.is_empty());
    }

    #[test]
    fn frontend_config_default() {
        let config = FrontendConfig::default();
        assert!(config.greeting.is_none());
        assert!(config.prompts.is_empty());
    }

    #[test]
    fn frontend_config_load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("frontend.toml"),
            "greeting = \"ready_\"\nprompts = [\"test prompt\"]\n",
        )
        .unwrap();
        let config = FrontendConfig::load(dir.path());
        assert_eq!(config.greeting.as_deref(), Some("ready_"));
        assert_eq!(config.prompts, vec!["test prompt"]);
    }

    #[test]
    fn frontend_config_load_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = FrontendConfig::load(dir.path());
        assert!(config.greeting.is_none());
        assert!(config.prompts.is_empty());
    }

    #[test]
    fn frontend_config_load_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("frontend.toml"), "not valid { toml").unwrap();
        let config = FrontendConfig::load(dir.path());
        // Should fall back to default, not panic
        assert!(config.greeting.is_none());
        assert!(config.prompts.is_empty());
    }

    #[test]
    fn frontend_config_serializes_to_json() {
        let config = FrontendConfig {
            greeting: Some("Hello".to_string()),
            prompts: vec!["a".to_string(), "b".to_string()],
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"greeting\":\"Hello\""));
        assert!(json.contains("\"prompts\":[\"a\",\"b\"]"));
    }

    // ── InternetConfig ────────────────────────────────────────────────

    #[test]
    fn internet_config_defaults() {
        let config = InternetConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_fetch_bytes, 2 * 1024 * 1024);
        assert_eq!(config.max_text_chars, 50_000);
        assert_eq!(config.timeout_secs, 15);
    }

    #[test]
    fn internet_config_from_toml_defaults() {
        let config: InternetConfig = toml::from_str("").unwrap();
        assert!(config.enabled);
        assert_eq!(config.max_fetch_bytes, 2 * 1024 * 1024);
        assert_eq!(config.max_text_chars, 50_000);
        assert_eq!(config.timeout_secs, 15);
    }

    #[test]
    fn internet_config_from_toml_partial_override() {
        let config: InternetConfig = toml::from_str(
            r#"
            enabled = false
            timeout_secs = 30
            "#,
        )
        .unwrap();
        assert!(!config.enabled);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.max_fetch_bytes, 2 * 1024 * 1024);
        assert_eq!(config.max_text_chars, 50_000);
    }

    #[test]
    fn internet_config_from_toml_full_override() {
        let config: InternetConfig = toml::from_str(
            r#"
            enabled = true
            max_fetch_bytes = 1048576
            max_text_chars = 25000
            timeout_secs = 60
            "#,
        )
        .unwrap();
        assert!(config.enabled);
        assert_eq!(config.max_fetch_bytes, 1_048_576);
        assert_eq!(config.max_text_chars, 25_000);
        assert_eq!(config.timeout_secs, 60);
    }

    #[test]
    fn starpod_config_includes_internet_section() {
        let config: StarpodConfig = toml::from_str(
            r#"
            [internet]
            enabled = false
            "#,
        )
        .unwrap();
        assert!(!config.internet.enabled);
    }

    #[test]
    fn starpod_config_default_has_internet_enabled() {
        let config = StarpodConfig::default();
        assert!(config.internet.enabled);
    }

    // ── Email channel config tests ──────────────────────────────────────

    #[test]
    fn email_channel_default_not_configured() {
        let config: StarpodConfig = toml::from_str("").unwrap();
        assert!(config.channels.email.is_none());
    }

    #[test]
    fn email_channel_enabled_and_gap_defaults() {
        let toml = r#"
            [channels.email]
            enabled = true
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let email = config.channels.email.as_ref().unwrap();
        assert!(email.enabled);
        assert_eq!(email.gap_minutes, Some(1440)); // 24h default
    }

    #[test]
    fn email_channel_custom_gap_minutes() {
        let toml = r#"
            [channels.email]
            gap_minutes = 60
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let email = config.channels.email.as_ref().unwrap();
        assert_eq!(email.gap_minutes, Some(60));
    }

    #[test]
    fn email_channel_gap_minutes_convenience() {
        let toml = r#"
            [channels.email]
            gap_minutes = 720
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.channel_gap_minutes("email"), Some(720));
    }

    #[test]
    fn channel_gap_minutes_email_none_when_not_configured() {
        let config: StarpodConfig = toml::from_str("").unwrap();
        assert_eq!(config.channel_gap_minutes("email"), None);
    }

    #[test]
    fn email_and_telegram_channels_coexist() {
        let toml = r#"
            [channels.telegram]
            gap_minutes = 360

            [channels.email]
            gap_minutes = 1440
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.channel_gap_minutes("telegram"), Some(360));
        assert_eq!(config.channel_gap_minutes("email"), Some(1440));
    }

    // ── parse_model_spec ────────────────────────────────────────────────

    #[test]
    fn parse_model_spec_valid() {
        assert_eq!(
            parse_model_spec("anthropic/claude-sonnet-4-6"),
            Some(("anthropic", "claude-sonnet-4-6"))
        );
        assert_eq!(
            parse_model_spec("openai/gpt-4o"),
            Some(("openai", "gpt-4o"))
        );
        assert_eq!(
            parse_model_spec("ollama/llama3"),
            Some(("ollama", "llama3"))
        );
    }

    #[test]
    fn parse_model_spec_no_slash() {
        assert_eq!(parse_model_spec("gpt-4o"), None);
        assert_eq!(parse_model_spec(""), None);
    }

    #[test]
    fn parse_model_spec_multiple_slashes() {
        // Only splits on first slash
        assert_eq!(
            parse_model_spec("openrouter/openai/gpt-4o"),
            Some(("openrouter", "openai/gpt-4o"))
        );
    }

    #[test]
    fn parse_model_spec_empty_parts() {
        assert_eq!(parse_model_spec("/model"), Some(("", "model")));
        assert_eq!(parse_model_spec("provider/"), Some(("provider", "")));
    }

    // ── default_model / provider / model ────────────────────────────────

    #[test]
    fn default_model_returns_first_entry() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["openai/gpt-4o", "anthropic/claude-sonnet-4-6"]
        "#,
        )
        .unwrap();
        assert_eq!(config.default_model(), ("openai", "gpt-4o"));
        assert_eq!(config.provider(), "openai");
        assert_eq!(config.model(), "gpt-4o");
    }

    #[test]
    fn default_model_fallback_when_empty() {
        let config = StarpodConfig {
            models: vec![],
            ..StarpodConfig::default()
        };
        assert_eq!(config.default_model(), ("anthropic", "claude-haiku-4-5"));
    }

    #[test]
    fn default_model_from_default_config() {
        let config = StarpodConfig::default();
        assert_eq!(config.provider(), "anthropic");
        assert_eq!(config.model(), "claude-haiku-4-5");
    }

    // ── resolve_model ───────────────────────────────────────────────────

    #[test]
    fn resolve_model_none_returns_default() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["anthropic/claude-sonnet-4-6", "openai/gpt-4o"]
        "#,
        )
        .unwrap();
        let (p, m) = config.resolve_model(None).unwrap();
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-sonnet-4-6");
    }

    #[test]
    fn resolve_model_valid_override() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["anthropic/claude-sonnet-4-6", "openai/gpt-4o"]
        "#,
        )
        .unwrap();
        let (p, m) = config.resolve_model(Some("openai/gpt-4o")).unwrap();
        assert_eq!(p, "openai");
        assert_eq!(m, "gpt-4o");
    }

    #[test]
    fn resolve_model_override_not_in_list() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["anthropic/claude-sonnet-4-6"]
        "#,
        )
        .unwrap();
        let err = config.resolve_model(Some("openai/gpt-4o")).unwrap_err();
        assert!(err.contains("not in the allowed list"));
    }

    #[test]
    fn resolve_model_override_matches_default() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["anthropic/claude-sonnet-4-6"]
        "#,
        )
        .unwrap();
        let (p, m) = config
            .resolve_model(Some("anthropic/claude-sonnet-4-6"))
            .unwrap();
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-sonnet-4-6");
    }

    // ── resolve_compaction_model ────────────────────────────────────────

    #[test]
    fn resolve_compaction_model_none_falls_back_to_default() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["anthropic/claude-sonnet-4-6"]
        "#,
        )
        .unwrap();
        let (p, m) = config.resolve_compaction_model();
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-sonnet-4-6");
    }

    #[test]
    fn resolve_compaction_model_with_spec() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["anthropic/claude-sonnet-4-6"]
            compaction_model = "openai/gpt-4o-mini"
        "#,
        )
        .unwrap();
        let (p, m) = config.resolve_compaction_model();
        assert_eq!(p, "openai");
        assert_eq!(m, "gpt-4o-mini");
    }

    #[test]
    fn resolve_compaction_model_invalid_spec_falls_back() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["anthropic/claude-sonnet-4-6"]
            compaction_model = "no-slash"
        "#,
        )
        .unwrap();
        // Invalid spec (no slash) falls back to primary
        let (p, m) = config.resolve_compaction_model();
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-sonnet-4-6");
    }

    // ── models from TOML ────────────────────────────────────────────────

    #[test]
    fn models_from_toml_single() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = ["anthropic/claude-haiku-4-5"]
        "#,
        )
        .unwrap();
        assert_eq!(config.models, vec!["anthropic/claude-haiku-4-5"]);
    }

    #[test]
    fn models_from_toml_multiple() {
        let config: StarpodConfig = toml::from_str(
            r#"
            models = [
                "anthropic/claude-sonnet-4-6",
                "anthropic/claude-opus-4-6",
                "openai/gpt-4o",
            ]
        "#,
        )
        .unwrap();
        assert_eq!(config.models.len(), 3);
        assert_eq!(config.model(), "claude-sonnet-4-6");
        assert_eq!(config.provider(), "anthropic");
    }

    #[test]
    fn models_default_when_absent() {
        let config: StarpodConfig = toml::from_str("").unwrap();
        assert_eq!(config.models, vec!["anthropic/claude-haiku-4-5"]);
    }

    // ── Provider options tests ──────────────────────────────────────────

    #[test]
    fn provider_options_empty_by_default() {
        let config = StarpodConfig::default();
        assert!(config.provider_options("ollama").is_empty());
        assert!(config.provider_options("openai").is_empty());
    }

    #[test]
    fn provider_options_unknown_provider_returns_empty() {
        let config = StarpodConfig::default();
        assert!(config.provider_options("nonexistent").is_empty());
    }

    #[test]
    fn provider_options_parsed_from_toml() {
        let toml = r#"
            [providers.ollama.options]
            keep_alive = "30m"
            num_ctx = 32768
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let opts = config.provider_options("ollama");
        assert_eq!(opts["keep_alive"], "30m");
        assert_eq!(opts["num_ctx"], 32768);
    }

    #[test]
    fn provider_options_empty_when_section_present_but_no_options() {
        let toml = r#"
            [providers.ollama]
            base_url = "http://localhost:11434/v1/chat/completions"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.provider_options("ollama").is_empty());
    }

    #[test]
    fn provider_options_per_provider_isolation() {
        let toml = r#"
            [providers.ollama.options]
            keep_alive = "5m"

            [providers.openai.options]
            seed = 42
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.provider_options("ollama").len(), 1);
        assert_eq!(config.provider_options("ollama")["keep_alive"], "5m");
        assert_eq!(config.provider_options("openai").len(), 1);
        assert_eq!(config.provider_options("openai")["seed"], 42);
        assert!(config.provider_options("anthropic").is_empty());
    }

    // ── Bedrock provider tests ──────────────────────────────────────────

    #[test]
    fn resolved_provider_api_key_bedrock_from_env() {
        std::env::set_var("AWS_ACCESS_KEY_ID", "AKIATEST12345");
        let config = StarpodConfig::default();
        assert_eq!(
            config.resolved_provider_api_key("bedrock"),
            Some("AKIATEST12345".to_string())
        );
        std::env::remove_var("AWS_ACCESS_KEY_ID");
    }

    #[test]
    fn resolved_provider_base_url_bedrock_default() {
        let config = StarpodConfig::default();
        assert_eq!(
            config.resolved_provider_base_url("bedrock"),
            Some("https://bedrock-runtime.us-east-1.amazonaws.com".to_string())
        );
    }

    #[test]
    fn resolved_provider_base_url_bedrock_config_override() {
        let toml = r#"
            [providers.bedrock]
            base_url = "https://bedrock-runtime.eu-west-1.amazonaws.com"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.resolved_provider_base_url("bedrock"),
            Some("https://bedrock-runtime.eu-west-1.amazonaws.com".to_string())
        );
    }

    #[test]
    fn provider_options_bedrock_region() {
        let toml = r#"
            [providers.bedrock.options]
            region = "eu-west-1"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let opts = config.provider_options("bedrock");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts["region"], "eu-west-1");
    }

    #[test]
    fn bedrock_provider_config_deserialization() {
        let toml = r#"
            [providers.bedrock]
            enabled = true
            [providers.bedrock.options]
            region = "us-east-1"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let bedrock = config.providers.bedrock.as_ref().unwrap();
        assert!(bedrock.enabled);
        assert_eq!(bedrock.options["region"], "us-east-1");
    }

    // ── Vertex AI provider tests ────────────────────────────────────────

    #[test]
    fn resolved_provider_api_key_vertex_from_env() {
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", "/path/to/sa.json");
        let config = StarpodConfig::default();
        assert_eq!(
            config.resolved_provider_api_key("vertex"),
            Some("/path/to/sa.json".to_string())
        );
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
    }

    #[test]
    fn resolved_provider_base_url_vertex_default() {
        let config = StarpodConfig::default();
        assert_eq!(
            config.resolved_provider_base_url("vertex"),
            Some("https://us-central1-aiplatform.googleapis.com".to_string())
        );
    }

    #[test]
    fn resolved_provider_base_url_vertex_config_override() {
        let toml = r#"
            [providers.vertex]
            base_url = "https://europe-west1-aiplatform.googleapis.com"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.resolved_provider_base_url("vertex"),
            Some("https://europe-west1-aiplatform.googleapis.com".to_string())
        );
    }

    #[test]
    fn provider_options_vertex_project_and_region() {
        let toml = r#"
            [providers.vertex.options]
            project_id = "my-gcp-project"
            region = "europe-west1"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let opts = config.provider_options("vertex");
        assert_eq!(opts.len(), 2);
        assert_eq!(opts["project_id"], "my-gcp-project");
        assert_eq!(opts["region"], "europe-west1");
    }

    #[test]
    fn vertex_provider_config_deserialization() {
        let toml = r#"
            [providers.vertex]
            enabled = true
            [providers.vertex.options]
            project_id = "test-project"
            region = "us-east1"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let vertex = config.providers.vertex.as_ref().unwrap();
        assert!(vertex.enabled);
        assert_eq!(vertex.options["project_id"], "test-project");
        assert_eq!(vertex.options["region"], "us-east1");
    }
}
