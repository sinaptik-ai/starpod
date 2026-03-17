use std::path::PathBuf;

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
                             move it to .env as {}_API_KEY",
                            name.to_uppercase()
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
                         move it to .env as TELEGRAM_BOT_TOKEN"
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
}

fn default_true() -> bool {
    true
}

/// Multi-provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub anthropic: Option<ProviderConfig>,
    pub openai: Option<ProviderConfig>,
    pub gemini: Option<ProviderConfig>,
    pub groq: Option<ProviderConfig>,
    pub deepseek: Option<ProviderConfig>,
    pub openrouter: Option<ProviderConfig>,
    pub ollama: Option<ProviderConfig>,
}

/// A single entry in the Telegram allow-list: either a numeric user ID or a username string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowedUser {
    Id(u64),
    Username(String),
}

/// Telegram channel configuration (lives under `[channels.telegram]`).
///
/// **The bot token belongs in `.env` as `TELEGRAM_BOT_TOKEN`, not here.**
/// Any `bot_token` found in a config file is ignored and triggers a warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramChannelConfig {
    /// Whether this channel is enabled (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Inactivity gap (in minutes) before auto-closing a Telegram session (default: 360 = 6h).
    #[serde(default = "default_gap_minutes")]
    pub gap_minutes: Option<i64>,
    /// Users allowed to interact with the bot — can be numeric IDs or
    /// usernames (without @). Example: `[123456789, "alice"]`.
    /// If empty, no one can chat (only /start works to show user ID/username).
    #[serde(default)]
    pub allowed_users: Vec<AllowedUser>,
    /// Message mode: "final_only" (default) sends only the last assistant
    /// message; "all_messages" sends each assistant message as a standalone
    /// Telegram message (tool-use messages are excluded).
    #[serde(default = "default_stream_mode")]
    pub stream_mode: String,
}

fn default_gap_minutes() -> Option<i64> { Some(360) }

impl Default for TelegramChannelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gap_minutes: default_gap_minutes(),
            allowed_users: Vec::new(),
            stream_mode: default_stream_mode(),
        }
    }
}

impl TelegramChannelConfig {
    /// Extract the numeric user IDs from the allow-list.
    pub fn allowed_user_ids(&self) -> Vec<u64> {
        self.allowed_users
            .iter()
            .filter_map(|u| match u {
                AllowedUser::Id(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Extract the usernames (lowercased) from the allow-list.
    pub fn allowed_usernames(&self) -> Vec<String> {
        self.allowed_users
            .iter()
            .filter_map(|u| match u {
                AllowedUser::Username(name) => Some(name.to_lowercase()),
                _ => None,
            })
            .collect()
    }
}

/// Channel configuration namespace (`[channels.*]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelsConfig {
    /// Telegram channel settings.
    pub telegram: Option<TelegramChannelConfig>,
    // future: discord, whatsapp, etc.
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
}

fn default_chunk_size() -> usize { 1600 }
fn default_chunk_overlap() -> usize { 320 }
fn default_bootstrap_file_cap() -> usize { 20_000 }

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
}

fn default_cron_max_retries() -> u32 { 3 }
fn default_cron_timeout_secs() -> u64 { 7200 }
fn default_cron_max_concurrent() -> usize { 1 }

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            default_max_retries: default_cron_max_retries(),
            default_timeout_secs: default_cron_timeout_secs(),
            max_concurrent_runs: default_cron_max_concurrent(),
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
}

fn default_context_budget() -> u64 { 160_000 }
fn default_summary_max_tokens() -> u32 { 4096 }
fn default_min_keep_messages() -> usize { 4 }

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            context_budget: default_context_budget(),
            summary_max_tokens: default_summary_max_tokens(),
            min_keep_messages: default_min_keep_messages(),
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
            let ext = file_name
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_lowercase();
            if !self.allowed_extensions.iter().any(|e| e.to_lowercase() == ext) {
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
    #[serde(default, alias = "data_dir")]
    pub db_dir: PathBuf,

    /// Path to the SQLite database (default: `<db_dir>/memory.db`)
    #[serde(default)]
    pub db_path: Option<PathBuf>,

    /// Server bind address (default: `127.0.0.1:3000`)
    #[serde(default = "default_server_addr")]
    pub server_addr: String,

    /// Active LLM provider (default: "anthropic").
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Claude model to use
    #[serde(default = "default_model")]
    pub model: String,

    /// Maximum agentic turns per request
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Reasoning effort for extended thinking (low, medium, high).
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,

    /// Model used for conversation compaction summaries.
    /// Defaults to the primary model if not set.
    #[serde(default)]
    pub compaction_model: Option<String>,

    /// Agent display name (default: "Aster").
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

    /// Attachment handling settings.
    #[serde(default)]
    pub attachments: AttachmentsConfig,

    /// The project root directory (not serialized — set at load time).
    #[serde(skip)]
    pub project_root: PathBuf,
}

fn default_server_addr() -> String {
    "127.0.0.1:3000".to_string()
}

fn default_provider() -> String {
    "anthropic".to_string()
}

fn default_model() -> String {
    "claude-haiku-4-5".to_string()
}

fn default_max_turns() -> u32 {
    30
}

fn default_max_tokens() -> u32 {
    16384
}

fn default_agent_name() -> String {
    "Aster".to_string()
}

impl Default for StarpodConfig {
    fn default() -> Self {
        Self {
            db_dir: PathBuf::new(),
            db_path: None,
            server_addr: default_server_addr(),
            provider: default_provider(),
            model: default_model(),
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
            attachments: AttachmentsConfig::default(),
            project_root: PathBuf::new(),
        }
    }
}

impl StarpodConfig {
    /// Resolved Anthropic API key from the `ANTHROPIC_API_KEY` env var.
    pub fn resolved_api_key(&self) -> Option<String> {
        std::env::var("ANTHROPIC_API_KEY").ok()
    }

    /// Resolved Telegram bot token from the `TELEGRAM_BOT_TOKEN` env var.
    pub fn resolved_telegram_token(&self) -> Option<String> {
        std::env::var("TELEGRAM_BOT_TOKEN").ok()
    }

    /// Resolved Telegram allowed user IDs from [channels.telegram] section.
    pub fn resolved_telegram_allowed_user_ids(&self) -> Vec<u64> {
        self.channels
            .telegram
            .as_ref()
            .map(|t| t.allowed_user_ids())
            .unwrap_or_default()
    }

    /// Resolved Telegram allowed usernames (lowercased) from [channels.telegram] section.
    pub fn resolved_telegram_allowed_usernames(&self) -> Vec<String> {
        self.channels
            .telegram
            .as_ref()
            .map(|t| t.allowed_usernames())
            .unwrap_or_default()
    }

    /// Get the inactivity gap (in minutes) for a channel by name.
    /// Returns `None` for channels that don't use time-gap sessions.
    pub fn channel_gap_minutes(&self, channel: &str) -> Option<i64> {
        match channel {
            "telegram" => self.channels.telegram.as_ref().and_then(|t| t.gap_minutes),
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

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_users_ids_only() {
        let toml = r#"
            [channels.telegram]
            allowed_users = [111, 222]
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let tg = config.channels.telegram.as_ref().unwrap();
        assert_eq!(tg.allowed_user_ids(), vec![111, 222]);
        assert!(tg.allowed_usernames().is_empty());
    }

    #[test]
    fn test_allowed_users_usernames_only() {
        let toml = r#"
            [channels.telegram]
            allowed_users = ["alice", "Bob"]
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let tg = config.channels.telegram.as_ref().unwrap();
        assert!(tg.allowed_user_ids().is_empty());
        assert_eq!(tg.allowed_usernames(), vec!["alice", "bob"]);
    }

    #[test]
    fn test_allowed_users_mixed() {
        let toml = r#"
            [channels.telegram]
            allowed_users = [123456789, "alice", 987654321, "Bob"]
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let tg = config.channels.telegram.as_ref().unwrap();
        assert_eq!(tg.allowed_user_ids(), vec![123456789, 987654321]);
        assert_eq!(tg.allowed_usernames(), vec!["alice", "bob"]);
    }

    #[test]
    fn test_allowed_users_empty() {
        let toml = r#"
            [channels.telegram]
            allowed_users = []
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let tg = config.channels.telegram.as_ref().unwrap();
        assert!(tg.allowed_user_ids().is_empty());
        assert!(tg.allowed_usernames().is_empty());
    }

    #[test]
    fn test_allowed_users_default() {
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
        assert_eq!(config.resolved_provider_api_key("nonexistent_provider"), None);
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
        assert_eq!(config.resolved_provider_base_url("nonexistent_provider"), None);
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
        assert_eq!(config.resolved_provider_api_key("ollama"), Some(String::new()));
    }

    // ── Credential-in-config rejection tests ─────────────────────────────

    #[test]
    fn api_key_in_provider_config_is_silently_ignored() {
        // Old configs that still have api_key in [providers.*] must parse
        // without error — the field is simply dropped by serde.
        let toml = r#"
            [providers.anthropic]
            api_key = "sk-ant-should-be-ignored"
            base_url = "https://custom.example.com"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let p = config.providers.anthropic.as_ref().unwrap();
        assert_eq!(p.base_url.as_deref(), Some("https://custom.example.com"));
        // No api_key field on ProviderConfig — credential is dropped.
    }

    #[test]
    fn bot_token_in_telegram_config_is_silently_ignored() {
        // Old configs with bot_token in [channels.telegram] must still parse.
        let toml = r#"
            [channels.telegram]
            bot_token = "123:ABC"
            gap_minutes = 60
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        let tg = config.channels.telegram.as_ref().unwrap();
        assert_eq!(tg.gap_minutes, Some(60));
        // No bot_token field on TelegramChannelConfig — credential is dropped.
    }

    #[test]
    fn warn_credentials_in_toml_detects_api_key() {
        let value: toml::Value = toml::from_str(r#"
            [providers.anthropic]
            api_key = "sk-ant-bad"
            [providers.openai]
            base_url = "https://example.com"
        "#).unwrap();
        // Should not panic — warning goes to tracing. We just verify the
        // function runs without error on input containing credentials.
        warn_credentials_in_toml(&value, "test.toml");
    }

    #[test]
    fn warn_credentials_in_toml_detects_bot_token() {
        let value: toml::Value = toml::from_str(r#"
            [channels.telegram]
            bot_token = "123:ABC"
        "#).unwrap();
        warn_credentials_in_toml(&value, "test.toml");
    }

    #[test]
    fn warn_credentials_in_toml_clean_config_no_panic() {
        // A config with no credentials should pass through silently.
        let value: toml::Value = toml::from_str(r#"
            [providers.anthropic]
            base_url = "https://api.anthropic.com"
            [channels.telegram]
            gap_minutes = 360
        "#).unwrap();
        warn_credentials_in_toml(&value, "clean.toml");
    }

    #[test]
    fn resolved_api_key_ignores_config_reads_env_only() {
        // Even if a provider section exists, the resolved key comes from env.
        let config: StarpodConfig = toml::from_str(r#"
            [providers.anthropic]
            base_url = "https://custom.example.com"
        "#).unwrap();
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
        assert!(cfg.export_sessions, "export_sessions should default to true");
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
    }

    #[test]
    fn compaction_config_default_when_missing_from_toml() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.compaction.context_budget, 160_000);
        assert_eq!(config.compaction.summary_max_tokens, 4096);
        assert_eq!(config.compaction.min_keep_messages, 4);
    }

    #[test]
    fn compaction_config_from_toml() {
        let toml = r#"
            [compaction]
            context_budget = 80000
            summary_max_tokens = 2048
            min_keep_messages = 8
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.compaction.context_budget, 80_000);
        assert_eq!(config.compaction.summary_max_tokens, 2048);
        assert_eq!(config.compaction.min_keep_messages, 8);
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
    }

    // ── Cron config tests ──────────────────────────────────────────────

    #[test]
    fn cron_config_defaults() {
        let cfg = CronConfig::default();
        assert_eq!(cfg.default_max_retries, 3);
        assert_eq!(cfg.default_timeout_secs, 7200);
        assert_eq!(cfg.max_concurrent_runs, 1);
    }

    #[test]
    fn cron_config_default_when_missing_from_toml() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.cron.default_max_retries, 3);
        assert_eq!(config.cron.default_timeout_secs, 7200);
        assert_eq!(config.cron.max_concurrent_runs, 1);
    }

    #[test]
    fn cron_config_from_toml() {
        let toml = r#"
            [cron]
            default_max_retries = 5
            default_timeout_secs = 3600
            max_concurrent_runs = 4
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.cron.default_max_retries, 5);
        assert_eq!(config.cron.default_timeout_secs, 3600);
        assert_eq!(config.cron.max_concurrent_runs, 4);
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
        assert_eq!(config.agent_name, "Aster");
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
        let mut base: toml::Value = toml::from_str(r#"model = "haiku""#).unwrap();
        let overlay: toml::Value = toml::from_str(r#"agent_name = "Nova""#).unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.model, "haiku");
        assert_eq!(config.agent_name, "Nova");
    }

    #[test]
    fn deep_merge_overlay_overrides_existing() {
        let mut base: toml::Value = toml::from_str(r#"model = "haiku""#).unwrap();
        let overlay: toml::Value = toml::from_str(r#"model = "sonnet""#).unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.model, "sonnet");
    }

    #[test]
    fn deep_merge_nested_tables() {
        let mut base: toml::Value = toml::from_str(r#"
            [memory]
            half_life_days = 30.0
        "#).unwrap();
        let overlay: toml::Value = toml::from_str(r#"
            [memory]
            mmr_lambda = 0.5
            [channels.telegram]
            bot_token = "test"
        "#).unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.memory.half_life_days, 30.0); // kept from base
        assert_eq!(config.memory.mmr_lambda, 0.5); // from overlay
        assert!(config.channels.telegram.is_some()); // from overlay
    }

    // ── old identity/user sections are silently ignored ─────────────────

    #[test]
    fn old_identity_section_silently_ignored() {
        let toml = r#"
            [identity]
            name = "OldName"
        "#;
        // serde(default) + no deny_unknown_fields → old sections are just ignored
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.agent_name, "Aster"); // default, not "OldName"
    }

    #[test]
    fn old_user_section_silently_ignored() {
        let toml = r#"
            [user]
            name = "OldUser"
            timezone = "UTC"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.timezone.is_none()); // not read from [user]
    }

    // ── deep_merge edge cases ───────────────────────────────────────

    #[test]
    fn deep_merge_overlay_replaces_scalar_with_table() {
        // Edge case: base has a scalar, overlay has a table at the same key
        let mut base: toml::Value = toml::from_str(r#"memory = "flat""#).unwrap();
        let overlay: toml::Value = toml::from_str(r#"
            [memory]
            half_life_days = 7.0
        "#).unwrap();
        deep_merge(&mut base, overlay);
        // The table should win
        let table = base.get("memory").unwrap().as_table().unwrap();
        assert_eq!(table.get("half_life_days").unwrap().as_float().unwrap(), 7.0);
    }

    #[test]
    fn deep_merge_empty_overlay_preserves_base() {
        let mut base: toml::Value = toml::from_str(r#"
            model = "haiku"
            agent_name = "Aster"
        "#).unwrap();
        let overlay: toml::Value = toml::from_str("").unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.model, "haiku");
        assert_eq!(config.agent_name, "Aster");
    }

    #[test]
    fn deep_merge_instance_overrides_model_but_keeps_other_fields() {
        let mut base: toml::Value = toml::from_str(r#"
            model = "haiku"
            max_turns = 30
            agent_name = "Aster"
        "#).unwrap();
        let overlay: toml::Value = toml::from_str(r#"
            model = "sonnet"
            [channels.telegram]
            gap_minutes = 120
        "#).unwrap();
        deep_merge(&mut base, overlay);
        let config: StarpodConfig = base.try_into().unwrap();
        assert_eq!(config.model, "sonnet"); // overridden
        assert_eq!(config.max_turns, 30); // preserved
        assert_eq!(config.agent_name, "Aster"); // preserved
        let tg = config.channels.telegram.unwrap();
        assert_eq!(tg.gap_minutes, Some(120)); // added
    }
}
