use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::OrionError;

/// Main configuration for Orion, loaded from `~/.orion/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrionConfig {
    /// Root data directory (default: `~/.orion/orion_data`)
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// Path to the SQLite database (default: `<data_dir>/memory.db`)
    #[serde(default)]
    pub db_path: Option<PathBuf>,

    /// Server bind address (default: `127.0.0.1:3000`)
    #[serde(default = "default_server_addr")]
    pub server_addr: String,

    /// Claude model to use
    #[serde(default = "default_model")]
    pub model: String,

    /// Maximum agentic turns per request
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Anthropic API key (if not set via ANTHROPIC_API_KEY env var)
    #[serde(default)]
    pub api_key: Option<String>,

    /// Telegram bot token (from @BotFather). If set, `orion serve` also starts the Telegram bot.
    #[serde(default)]
    pub telegram_bot_token: Option<String>,
}

fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".orion")
        .join("orion_data")
}

fn default_server_addr() -> String {
    "127.0.0.1:3000".to_string()
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
            data_dir: default_data_dir(),
            db_path: None,
            server_addr: default_server_addr(),
            model: default_model(),
            max_turns: default_max_turns(),
            api_key: None,
            telegram_bot_token: None,
        }
    }
}

impl OrionConfig {
    /// Load config from `~/.orion/config.toml`, falling back to defaults.
    pub async fn load() -> Result<Self, OrionError> {
        let config_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".orion")
            .join("config.toml");

        Self::load_from(&config_path).await
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

    /// Resolved database path (uses `db_path` if set, otherwise `<data_dir>/memory.db`).
    pub fn resolved_db_path(&self) -> PathBuf {
        self.db_path
            .clone()
            .unwrap_or_else(|| self.data_dir.join("memory.db"))
    }
}
