use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::StarpodError;

/// Project directory name (created by `starpod agent init`).
const PROJECT_DIR: &str = ".starpod";
const CONFIG_FILE: &str = "config.toml";

// ── Sub-config types ─────────────────────────────────────────────────────

/// Agent identity (name, emoji, personality).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    /// Agent's display name (default: "Aster").
    pub name: Option<String>,
    /// Agent's emoji/avatar (e.g. "🤖").
    pub emoji: Option<String>,
    /// Freeform personality text injected into system prompt.
    /// Use this for custom instructions, tone, or behavior.
    pub soul: Option<String>,
}

impl IdentityConfig {
    /// The resolved agent name (falls back to "Aster").
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Aster")
    }
}

/// User profile (set during onboarding or in config).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserConfig {
    /// User's name (used in conversations).
    pub name: Option<String>,
    /// User's timezone (IANA format, e.g. "Europe/Rome").
    pub timezone: Option<String>,
}

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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// Whether this provider is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// API key (or use the corresponding env var).
    pub api_key: Option<String>,
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

/// Telegram-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    /// Bot token from @BotFather.
    pub bot_token: Option<String>,
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

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: None,
            allowed_users: Vec::new(),
            stream_mode: default_stream_mode(),
        }
    }
}

impl TelegramConfig {
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
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            half_life_days: 30.0,
            mmr_lambda: 0.7,
            vector_search: true,
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
    /// Root data directory (default: `.starpod/data` relative to project root)
    #[serde(default)]
    pub data_dir: PathBuf,

    /// Path to the SQLite database (default: `<data_dir>/memory.db`)
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

    /// Agent identity (name, emoji, personality).
    #[serde(default)]
    pub identity: IdentityConfig,

    /// User profile.
    #[serde(default)]
    pub user: UserConfig,

    /// Multi-provider configuration.
    #[serde(default)]
    pub providers: ProvidersConfig,

    /// Telegram bot configuration.
    #[serde(default)]
    pub telegram: TelegramConfig,

    /// How followup messages are handled during an active agent loop.
    /// "inject" (default) integrates them into the next loop iteration;
    /// "queue" buffers them and starts a new loop after the current one finishes.
    #[serde(default)]
    pub followup_mode: FollowupMode,

    /// Memory search tuning.
    #[serde(default)]
    pub memory: MemoryConfig,

    /// Attachment handling settings.
    #[serde(default)]
    pub attachments: AttachmentsConfig,

    /// Remote instance backend URL (e.g. "https://api.starpod.example.com").
    /// If set, `starpod instance` commands will connect to this backend.
    #[serde(default)]
    pub instance_backend_url: Option<String>,

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

impl Default for StarpodConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::new(),
            db_path: None,
            server_addr: default_server_addr(),
            provider: default_provider(),
            model: default_model(),
            max_turns: default_max_turns(),
            reasoning_effort: None,
            compaction_model: None,
            followup_mode: FollowupMode::default(),
            memory: MemoryConfig::default(),
            identity: IdentityConfig::default(),
            user: UserConfig::default(),
            providers: ProvidersConfig::default(),
            telegram: TelegramConfig::default(),
            attachments: AttachmentsConfig::default(),
            instance_backend_url: None,
            project_root: PathBuf::new(),
        }
    }
}

impl StarpodConfig {
    /// Find the `.starpod/` directory by walking up from the current directory.
    /// Returns the project root (parent of `.starpod/`).
    pub fn find_project_root() -> Option<PathBuf> {
        let mut dir = std::env::current_dir().ok()?;
        loop {
            if dir.join(PROJECT_DIR).is_dir() {
                return Some(dir);
            }
            if !dir.pop() {
                return None;
            }
        }
    }

    /// Load config from `.starpod/config.toml` in the current project.
    /// Walks up from CWD to find the project root.
    pub async fn load() -> Result<Self, StarpodError> {
        let project_root = Self::find_project_root().ok_or_else(|| {
            StarpodError::Config(
                "No .starpod/ directory found. Run `starpod agent init` to initialize a project."
                    .to_string(),
            )
        })?;

        let config_path = project_root.join(PROJECT_DIR).join(CONFIG_FILE);
        let mut config = Self::load_from(&config_path).await?;
        config.project_root = project_root;

        // Resolve data_dir relative to project root if not absolute
        if config.data_dir.as_os_str().is_empty() {
            config.data_dir = config.project_root.join(PROJECT_DIR).join("data");
        } else if config.data_dir.is_relative() {
            config.data_dir = config.project_root.join(&config.data_dir);
        }

        Ok(config)
    }

    /// Load config from a specific path.
    pub async fn load_from(path: &Path) -> Result<Self, StarpodError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            StarpodError::Config(format!("Failed to read config at {}: {}", path.display(), e))
        })?;

        let config: StarpodConfig = toml::from_str(&content)
            .map_err(|e| StarpodError::Config(format!("Invalid config TOML: {}", e)))?;

        Ok(config)
    }

    /// Resolved Anthropic API key: checks providers.anthropic.api_key,
    /// then ANTHROPIC_API_KEY env var.
    pub fn resolved_api_key(&self) -> Option<String> {
        self.providers
            .anthropic
            .as_ref()
            .and_then(|p| p.api_key.clone())
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
    }

    /// Resolved Telegram bot token: checks [telegram] section, then env var.
    pub fn resolved_telegram_token(&self) -> Option<String> {
        self.telegram
            .bot_token
            .clone()
            .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
    }

    /// Resolved Telegram allowed user IDs from [telegram] section.
    pub fn resolved_telegram_allowed_user_ids(&self) -> Vec<u64> {
        self.telegram.allowed_user_ids()
    }

    /// Resolved Telegram allowed usernames (lowercased) from [telegram] section.
    pub fn resolved_telegram_allowed_usernames(&self) -> Vec<String> {
        self.telegram.allowed_usernames()
    }

    /// Resolved database path (uses `db_path` if set, otherwise `<data_dir>/memory.db`).
    pub fn resolved_db_path(&self) -> PathBuf {
        self.db_path
            .clone()
            .unwrap_or_else(|| self.data_dir.join("memory.db"))
    }

    /// Resolved instance backend URL: checks config, then env var.
    pub fn resolved_instance_backend_url(&self) -> Option<String> {
        self.instance_backend_url
            .clone()
            .or_else(|| std::env::var("STARPOD_INSTANCE_BACKEND_URL").ok())
    }

    /// Resolved API key for any provider.
    ///
    /// Checks `providers.<name>.api_key` in config, then falls back to the
    /// conventional environment variable for that provider.
    pub fn resolved_provider_api_key(&self, provider: &str) -> Option<String> {
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

        cfg.and_then(|c| c.api_key.clone()).or_else(|| {
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
        })
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

    /// Path to the `.starpod/` directory for this project.
    pub fn starpod_dir(&self) -> PathBuf {
        self.project_root.join(PROJECT_DIR)
    }

    /// Initialize a new Starpod project in the given directory.
    /// Creates `.starpod/config.toml` and `.starpod/data/`.
    ///
    /// If `config_content` is provided, it is written as the config file.
    /// Otherwise a commented default template is used.
    pub async fn init(dir: &Path, config_content: Option<&str>) -> Result<(), StarpodError> {
        let starpod_dir = dir.join(PROJECT_DIR);

        if starpod_dir.exists() {
            return Err(StarpodError::Config(format!(
                "Already initialized: {} exists",
                starpod_dir.display()
            )));
        }

        // Create directory structure
        let data_dir = starpod_dir.join("data");
        tokio::fs::create_dir_all(&data_dir)
            .await
            .map_err(|e| StarpodError::Io(e))?;

        let content = config_content.unwrap_or(Self::DEFAULT_CONFIG);

        tokio::fs::write(starpod_dir.join(CONFIG_FILE), content)
            .await
            .map_err(|e| StarpodError::Io(e))?;

        Ok(())
    }

    /// Default config template (well-commented, all values commented out or set to defaults).
    pub const DEFAULT_CONFIG: &str = r#"# Starpod agent configuration
# See: https://github.com/gventuri/starpod-rs

# ══════════════════════════════════════════════════════════════════════════════
# GENERAL
# ══════════════════════════════════════════════════════════════════════════════

# Active LLM provider ("anthropic", "openai", etc.)
provider = "anthropic"

# Model to use
model = "claude-haiku-4-5"

# Maximum agentic turns per request
max_turns = 30

# Server bind address
server_addr = "127.0.0.1:3000"

# Reasoning effort for extended thinking: "low", "medium", "high"
# reasoning_effort = "medium"

# Model for conversation compaction summaries (defaults to primary model)
# compaction_model = "claude-haiku-4-5"

# How followup messages are handled during an active agent loop.
# "inject" (default) integrates them into the next loop iteration;
# "queue" buffers them and starts a new loop after the current one finishes.
# followup_mode = "inject"

# ══════════════════════════════════════════════════════════════════════════════
# MEMORY
# ══════════════════════════════════════════════════════════════════════════════
# Tune memory search behavior.

[memory]
# half_life_days = 30.0            # Temporal decay half-life for daily logs
# mmr_lambda = 0.7                 # 0.0 = max diversity, 1.0 = pure relevance
# vector_search = true             # Enable vector (semantic) search

# ══════════════════════════════════════════════════════════════════════════════
# AGENT IDENTITY
# ══════════════════════════════════════════════════════════════════════════════
# Customize your agent's personality.

[identity]
# name = "Aster"                  # Agent's display name
# emoji = "🤖"                    # Agent's emoji/avatar
# soul = ""                       # Freeform personality text injected into system prompt
                                  # Use this for custom instructions, tone, or behavior

# ══════════════════════════════════════════════════════════════════════════════
# USER PROFILE
# ══════════════════════════════════════════════════════════════════════════════
# Information about you.

[user]
# name = "Your Name"              # Your name (used in conversations)
# timezone = "America/New_York"   # Your timezone (IANA format)

# ══════════════════════════════════════════════════════════════════════════════
# LLM PROVIDERS
# ══════════════════════════════════════════════════════════════════════════════
# Configure API keys and settings for each LLM provider.
# Each provider supports: enabled, api_key, base_url, models

# [providers.anthropic]
# api_key = "sk-ant-..."                      # Or set ANTHROPIC_API_KEY env var
# base_url = "https://api.anthropic.com"

# [providers.openai]
# api_key = "sk-..."                          # Or set OPENAI_API_KEY env var
# models = ["gpt-4o", "gpt-4o-mini"]

# ══════════════════════════════════════════════════════════════════════════════
# TELEGRAM
# ══════════════════════════════════════════════════════════════════════════════

[telegram]
# bot_token = "123456:ABC..."     # Or set TELEGRAM_BOT_TOKEN env var
# allowed_users = [123456789, "alice"]  # User IDs or usernames (without @)
# stream_mode = "final_only"      # "final_only" or "all_messages"

# ══════════════════════════════════════════════════════════════════════════════
# ATTACHMENTS
# ══════════════════════════════════════════════════════════════════════════════
# Control file attachment handling.

[attachments]
# enabled = true                   # Set to false to disable attachments entirely
# allowed_extensions = []          # Allowed file extensions, e.g. ["jpg", "png", "pdf"]
                                   # Empty list = all extensions allowed
# max_file_size = 20971520         # Max file size in bytes (default: 20 MB)

# ══════════════════════════════════════════════════════════════════════════════
# INSTANCES
# ══════════════════════════════════════════════════════════════════════════════
# Remote instance backend for `starpod instance` commands.

# instance_backend_url = "https://api.starpod.example.com"  # Or set STARPOD_INSTANCE_BACKEND_URL env var
"#;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_users_ids_only() {
        let toml = r#"
            [telegram]
            allowed_users = [111, 222]
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.telegram.allowed_user_ids(), vec![111, 222]);
        assert!(config.telegram.allowed_usernames().is_empty());
    }

    #[test]
    fn test_allowed_users_usernames_only() {
        let toml = r#"
            [telegram]
            allowed_users = ["alice", "Bob"]
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.telegram.allowed_user_ids().is_empty());
        assert_eq!(config.telegram.allowed_usernames(), vec!["alice", "bob"]);
    }

    #[test]
    fn test_allowed_users_mixed() {
        let toml = r#"
            [telegram]
            allowed_users = [123456789, "alice", 987654321, "Bob"]
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.telegram.allowed_user_ids(), vec![123456789, 987654321]);
        assert_eq!(config.telegram.allowed_usernames(), vec!["alice", "bob"]);
    }

    #[test]
    fn test_allowed_users_empty() {
        let toml = r#"
            [telegram]
            allowed_users = []
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.telegram.allowed_user_ids().is_empty());
        assert!(config.telegram.allowed_usernames().is_empty());
    }

    #[test]
    fn test_allowed_users_default() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert!(config.telegram.allowed_users.is_empty());
    }

    #[test]
    fn test_resolved_api_key_from_config() {
        let toml = r#"
            [providers.anthropic]
            api_key = "sk-ant-from-config"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.resolved_api_key().unwrap(), "sk-ant-from-config");
    }

    #[test]
    fn test_resolved_api_key_config_takes_priority_over_env() {
        let toml = r#"
            [providers.anthropic]
            api_key = "sk-ant-from-config"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        // Even if env var is set, config value should win
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-from-env");
        let key = config.resolved_api_key().unwrap();
        std::env::remove_var("ANTHROPIC_API_KEY");
        assert_eq!(key, "sk-ant-from-config");
    }

    #[test]
    fn test_resolved_api_key_falls_back_to_env() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-from-env");
        let key = config.resolved_api_key();
        std::env::remove_var("ANTHROPIC_API_KEY");
        assert_eq!(key.unwrap(), "sk-ant-from-env");
    }

    #[test]
    fn test_resolved_api_key_none_when_neither_set() {
        let toml = "";
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        std::env::remove_var("ANTHROPIC_API_KEY");
        assert!(config.resolved_api_key().is_none());
    }

    #[test]
    fn test_resolved_api_key_empty_provider_section() {
        // Provider section exists but no api_key — should fall back to env
        let toml = r#"
            [providers.anthropic]
            base_url = "https://api.anthropic.com"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        std::env::remove_var("ANTHROPIC_API_KEY");
        assert!(config.resolved_api_key().is_none());
    }

    #[test]
    fn resolved_provider_api_key_from_config() {
        let toml = r#"
            [providers.openai]
            api_key = "sk-test-openai-key"
        "#;
        let config: StarpodConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.resolved_provider_api_key("openai"),
            Some("sk-test-openai-key".to_string())
        );
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
}
