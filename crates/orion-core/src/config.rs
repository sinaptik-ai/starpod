use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::OrionError;

/// Project directory name (created by `orion agent init`).
const PROJECT_DIR: &str = ".orion";
const CONFIG_FILE: &str = "config.toml";

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

    /// Claude model to use
    #[serde(default = "default_model")]
    pub model: String,

    /// Maximum agentic turns per request
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Anthropic API key (if not set via ANTHROPIC_API_KEY env var)
    #[serde(default)]
    pub api_key: Option<String>,

    /// Telegram bot token (from @BotFather). If set, `orion agent serve` also starts the Telegram bot.
    #[serde(default)]
    pub telegram_bot_token: Option<String>,

    /// The project root directory (not serialized — set at load time).
    #[serde(skip)]
    pub project_root: PathBuf,
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
            data_dir: PathBuf::new(),
            db_path: None,
            server_addr: default_server_addr(),
            model: default_model(),
            max_turns: default_max_turns(),
            api_key: None,
            telegram_bot_token: None,
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
    pub async fn init(dir: &Path) -> Result<(), OrionError> {
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

        // Write default config
        let config_content = r#"# Orion agent configuration
# See: https://github.com/gventuri/orion-rs

# Claude model to use
model = "claude-haiku-4-5"

# Maximum agentic turns per request
max_turns = 30

# Server bind address
server_addr = "127.0.0.1:3000"

# Anthropic API key (or set ANTHROPIC_API_KEY env var)
# api_key = ""

# Telegram bot token (or set TELEGRAM_BOT_TOKEN env var)
# telegram_bot_token = ""
"#;

        tokio::fs::write(orion_dir.join(CONFIG_FILE), config_content)
            .await
            .map_err(|e| OrionError::Io(e))?;

        Ok(())
    }
}
