use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::OrionError;

/// Project directory name (created by `orion agent init`).
const PROJECT_DIR: &str = ".orion";
const CONFIG_FILE: &str = "config.toml";

// ── Sub-config types ─────────────────────────────────────────────────────

/// Agent identity (name, emoji, personality).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    /// Agent's display name (default: "Orion").
    pub name: Option<String>,
    /// Agent's emoji/avatar (e.g. "🤖").
    pub emoji: Option<String>,
    /// Freeform personality text injected into system prompt.
    /// Use this for custom instructions, tone, or behavior.
    pub soul: Option<String>,
}

impl IdentityConfig {
    /// The resolved agent name (falls back to "Orion").
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Orion")
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

/// Telegram-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    /// Bot token from @BotFather.
    pub bot_token: Option<String>,
    /// User IDs allowed to interact with the bot.
    /// If empty, no one can chat (only /start works to show the user their ID).
    #[serde(default)]
    pub allowed_users: Vec<u64>,
    /// Streaming mode: "edit_in_place" or "off" (default: "off").
    #[serde(default = "default_stream_mode")]
    pub stream_mode: String,
    /// Minimum interval between edit-in-place updates (ms).
    #[serde(default = "default_edit_throttle_ms")]
    pub edit_throttle_ms: u64,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: None,
            allowed_users: Vec::new(),
            stream_mode: default_stream_mode(),
            edit_throttle_ms: default_edit_throttle_ms(),
        }
    }
}

fn default_stream_mode() -> String {
    "off".to_string()
}

fn default_edit_throttle_ms() -> u64 {
    300
}

// ── Main config ──────────────────────────────────────────────────────────

/// Main configuration for Orion, loaded from `.orion/config.toml` in the current directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrionConfig {
    /// Root data directory (default: `.orion/data` relative to project root)
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

impl Default for OrionConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::new(),
            db_path: None,
            server_addr: default_server_addr(),
            provider: default_provider(),
            model: default_model(),
            max_turns: default_max_turns(),
            reasoning_effort: None,
            identity: IdentityConfig::default(),
            user: UserConfig::default(),
            providers: ProvidersConfig::default(),
            telegram: TelegramConfig::default(),
            project_root: PathBuf::new(),
        }
    }
}

impl OrionConfig {
    /// Find the `.orion/` directory by walking up from the current directory.
    /// Returns the project root (parent of `.orion/`).
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

    /// Load config from `.orion/config.toml` in the current project.
    /// Walks up from CWD to find the project root.
    pub async fn load() -> Result<Self, OrionError> {
        let project_root = Self::find_project_root().ok_or_else(|| {
            OrionError::Config(
                "No .orion/ directory found. Run `orion agent init` to initialize a project."
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
    pub async fn load_from(path: &Path) -> Result<Self, OrionError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            OrionError::Config(format!("Failed to read config at {}: {}", path.display(), e))
        })?;

        let config: OrionConfig = toml::from_str(&content)
            .map_err(|e| OrionError::Config(format!("Invalid config TOML: {}", e)))?;

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

    /// Resolved Telegram allowed users from [telegram] section.
    pub fn resolved_telegram_allowed_users(&self) -> &[u64] {
        &self.telegram.allowed_users
    }

    /// Resolved database path (uses `db_path` if set, otherwise `<data_dir>/memory.db`).
    pub fn resolved_db_path(&self) -> PathBuf {
        self.db_path
            .clone()
            .unwrap_or_else(|| self.data_dir.join("memory.db"))
    }

    /// Path to the `.orion/` directory for this project.
    pub fn orion_dir(&self) -> PathBuf {
        self.project_root.join(PROJECT_DIR)
    }

    /// Initialize a new Orion project in the given directory.
    /// Creates `.orion/config.toml` and `.orion/data/`.
    ///
    /// If `config_content` is provided, it is written as the config file.
    /// Otherwise a commented default template is used.
    pub async fn init(dir: &Path, config_content: Option<&str>) -> Result<(), OrionError> {
        let orion_dir = dir.join(PROJECT_DIR);

        if orion_dir.exists() {
            return Err(OrionError::Config(format!(
                "Already initialized: {} exists",
                orion_dir.display()
            )));
        }

        // Create directory structure
        let data_dir = orion_dir.join("data");
        tokio::fs::create_dir_all(&data_dir)
            .await
            .map_err(|e| OrionError::Io(e))?;

        let content = config_content.unwrap_or(Self::DEFAULT_CONFIG);

        tokio::fs::write(orion_dir.join(CONFIG_FILE), content)
            .await
            .map_err(|e| OrionError::Io(e))?;

        Ok(())
    }

    /// Default config template (well-commented, all values commented out or set to defaults).
    pub const DEFAULT_CONFIG: &str = r#"# Orion agent configuration
# See: https://github.com/gventuri/orion-rs

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

# ══════════════════════════════════════════════════════════════════════════════
# AGENT IDENTITY
# ══════════════════════════════════════════════════════════════════════════════
# Customize your agent's personality.

[identity]
# name = "Orion"                  # Agent's display name
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
# allowed_users = [123456789]     # User IDs allowed to chat (empty = no one)
# stream_mode = "off"             # "edit_in_place" or "off"
# edit_throttle_ms = 300          # Min interval between streaming edits
"#;
}
