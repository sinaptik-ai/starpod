//! Multi-agent workspace detection, path resolution, and config loading.
//!
//! Starpod supports two operating modes, detected automatically:
//!
//! - **Workspace** (dev): `starpod.toml` in CWD or parent walk-up, multiple
//!   agents under `agents/<name>/`, shared skills in `skills/`.
//! - **SingleAgent** (prod): `.starpod/agent.toml` in CWD, everything self-contained.
//!
//! # Directory layouts
//!
//! **Workspace mode:**
//! ```text
//! my-platform/
//! ├── starpod.toml          # workspace defaults
//! ├── .env                  # secrets (never deployed)
//! ├── agents/
//! │   ├── sales-rep/
//! │   │   ├── agent.toml    # per-agent overrides
//! │   │   ├── SOUL.md
//! │   │   ├── data/         # sqlite dbs
//! │   │   └── memory/
//! │   └── support-bot/
//! └── skills/
//!     └── email-draft/SKILL.md
//! ```
//!
//! **Single-agent mode:**
//! ```text
//! /app/.starpod/
//! ├── agent.toml
//! ├── SOUL.md
//! ├── data/
//! ├── skills/
//! └── memory/
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use starpod_core::{detect_mode, ResolvedPaths, load_agent_config};
//!
//! let mode = detect_mode(Some("my-agent")).unwrap();
//! let paths = ResolvedPaths::resolve(&mode).unwrap();
//! let config = load_agent_config(&paths).unwrap();
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::{
    deep_merge, AttachmentsConfig, ChannelsConfig, CompactionConfig, CronConfig, FollowupMode,
    MemoryConfig, ProvidersConfig, ReasoningEffort, StarpodConfig,
};
use crate::error::StarpodError;

// ── Mode detection ──────────────────────────────────────────────────────────

/// How Starpod was invoked.
#[derive(Debug, Clone)]
pub enum Mode {
    /// Production single-agent: `.starpod/agent.toml` in CWD.
    SingleAgent { starpod_dir: PathBuf },
    /// Dev workspace: `starpod.toml` found via walk-up.
    Workspace { root: PathBuf, agent_name: String },
}

/// Detect the operating mode from the current directory.
///
/// - CWD has `.starpod/agent.toml` → `SingleAgent`
/// - Walk up for `starpod.toml` → `Workspace` (requires `agent_name`)
/// - Neither → error
pub fn detect_mode(agent_name: Option<&str>) -> crate::Result<Mode> {
    let cwd = std::env::current_dir().map_err(|e| {
        StarpodError::Config(format!("Failed to get current directory: {}", e))
    })?;
    detect_mode_from(agent_name, &cwd)
}

/// Like `detect_mode` but starting from a given directory instead of CWD.
pub fn detect_mode_from(agent_name: Option<&str>, start_dir: &Path) -> crate::Result<Mode> {
    // Check single-agent mode first
    let starpod_dir = start_dir.join(".starpod");
    if starpod_dir.join("agent.toml").is_file() {
        return Ok(Mode::SingleAgent { starpod_dir });
    }

    // Walk up looking for starpod.toml
    let mut dir = start_dir.to_path_buf();
    loop {
        if dir.join("starpod.toml").is_file() {
            let name = agent_name
                .map(|s| s.to_string())
                .or_else(|| infer_agent_name_from_cwd(start_dir, &dir))
                .ok_or_else(|| {
                    StarpodError::Config(
                        "Workspace detected but no agent name provided. \
                         Use --agent <name> or cd into agents/<name>/."
                            .to_string(),
                    )
                })?;
            return Ok(Mode::Workspace {
                root: dir,
                agent_name: name,
            });
        }
        if !dir.pop() {
            break;
        }
    }

    // Also check old-style .starpod/ for helpful error message
    if starpod_dir.is_dir() {
        return Err(StarpodError::Config(
            "Found .starpod/ but no agent.toml inside. \
             Create .starpod/agent.toml to use single-agent mode."
                .to_string(),
        ));
    }

    Err(StarpodError::Config(
        "No starpod.toml or .starpod/ found. \
         Run `starpod init` to create a workspace or `starpod agent init` in a deployment directory."
            .to_string(),
    ))
}

/// If CWD is inside `<workspace>/agents/<name>/`, infer the agent name.
fn infer_agent_name_from_cwd(cwd: &Path, workspace_root: &Path) -> Option<String> {
    let relative = cwd.strip_prefix(workspace_root).ok()?;
    let mut components = relative.components();
    let first = components.next()?.as_os_str().to_str()?;
    if first == "agents" {
        let name = components.next()?.as_os_str().to_str()?;
        Some(name.to_string())
    } else {
        None
    }
}

// ── Resolved paths ──────────────────────────────────────────────────────────

/// All resolved paths for an agent, derived from the operating mode.
#[derive(Debug, Clone)]
pub struct ResolvedPaths {
    /// The detected operating mode.
    pub mode: Mode,
    /// Path to agent.toml.
    pub agent_toml: PathBuf,
    /// Agent home directory (agents/<name>/ or .starpod/).
    pub agent_home: PathBuf,
    /// Data directory for SQLite DBs (agent_home/data/).
    pub data_dir: PathBuf,
    /// Skills directory (workspace skills/ or .starpod/skills/).
    pub skills_dir: PathBuf,
    /// Project/workspace root.
    pub project_root: PathBuf,
    /// .env file path (workspace only, if it exists).
    pub env_file: Option<PathBuf>,
}

impl ResolvedPaths {
    /// Resolve all paths from a detected mode.
    pub fn resolve(mode: &Mode) -> crate::Result<Self> {
        match mode {
            Mode::SingleAgent { starpod_dir } => {
                let agent_toml = starpod_dir.join("agent.toml");
                let agent_home = starpod_dir.clone();
                let data_dir = starpod_dir.join("data");
                let skills_dir = starpod_dir.join("skills");
                let project_root = starpod_dir
                    .parent()
                    .ok_or_else(|| {
                        StarpodError::Config("Invalid .starpod/ path".to_string())
                    })?
                    .to_path_buf();

                Ok(Self {
                    mode: mode.clone(),
                    agent_toml,
                    agent_home,
                    data_dir,
                    skills_dir,
                    project_root,
                    env_file: None,
                })
            }
            Mode::Workspace { root, agent_name } => {
                let agents_dir = root.join("agents").join(agent_name);
                let agent_toml = agents_dir.join("agent.toml");
                let data_dir = agents_dir.join("data");
                let skills_dir = root.join("skills");
                let env_path = root.join(".env");

                Ok(Self {
                    mode: mode.clone(),
                    agent_toml,
                    agent_home: agents_dir,
                    data_dir,
                    skills_dir,
                    project_root: root.clone(),
                    env_file: if env_path.is_file() {
                        Some(env_path)
                    } else {
                        None
                    },
                })
            }
        }
    }
}

// ── Agent config ────────────────────────────────────────────────────────────

/// Per-agent configuration loaded from `agent.toml`.
///
/// Same fields as `StarpodConfig` minus instance-related fields,
/// plus agent-specific additions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent name (directory name in workspace mode, or from agent_name field).
    #[serde(default = "default_agent_name")]
    pub name: String,

    /// References to shared skills (empty = all available).
    #[serde(default)]
    pub skills: Vec<String>,

    /// Server bind address.
    #[serde(default = "default_server_addr")]
    pub server_addr: String,

    /// Active LLM provider.
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Model to use.
    #[serde(default = "default_model")]
    pub model: String,

    /// Maximum agentic turns per request.
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Maximum tokens for LLM responses.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    /// Reasoning effort for extended thinking.
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,

    /// Compaction model override.
    #[serde(default)]
    pub compaction_model: Option<String>,

    /// Agent display name.
    #[serde(default = "default_agent_name")]
    pub agent_name: String,

    /// Timezone (IANA format).
    #[serde(default)]
    pub timezone: Option<String>,

    /// Followup message handling.
    #[serde(default)]
    pub followup_mode: FollowupMode,

    /// Multi-provider configuration.
    #[serde(default)]
    pub providers: ProvidersConfig,

    /// Channel configurations.
    #[serde(default)]
    pub channels: ChannelsConfig,

    /// Memory settings.
    #[serde(default)]
    pub memory: MemoryConfig,

    /// Cron settings.
    #[serde(default)]
    pub cron: CronConfig,

    /// Compaction settings.
    #[serde(default)]
    pub compaction: CompactionConfig,

    /// Attachment settings.
    #[serde(default)]
    pub attachments: AttachmentsConfig,
}

fn default_agent_name() -> String {
    "Aster".to_string()
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

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
            skills: Vec::new(),
            server_addr: default_server_addr(),
            provider: default_provider(),
            model: default_model(),
            max_turns: default_max_turns(),
            max_tokens: default_max_tokens(),
            reasoning_effort: None,
            compaction_model: None,
            agent_name: default_agent_name(),
            timezone: None,
            followup_mode: FollowupMode::default(),
            providers: ProvidersConfig::default(),
            channels: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            cron: CronConfig::default(),
            compaction: CompactionConfig::default(),
            attachments: AttachmentsConfig::default(),
        }
    }
}

impl AgentConfig {
    /// Convert to `StarpodConfig` for backward compatibility with existing code.
    pub fn into_starpod_config(self, paths: &ResolvedPaths) -> StarpodConfig {
        StarpodConfig {
            data_dir: paths.data_dir.clone(),
            db_path: None,
            server_addr: self.server_addr,
            provider: self.provider,
            model: self.model,
            max_turns: self.max_turns,
            max_tokens: self.max_tokens,
            reasoning_effort: self.reasoning_effort,
            compaction_model: self.compaction_model,
            agent_name: self.agent_name,
            timezone: self.timezone,
            followup_mode: self.followup_mode,
            providers: self.providers,
            channels: self.channels,
            memory: self.memory,
            cron: self.cron,
            compaction: self.compaction,
            attachments: self.attachments,
            project_root: paths.project_root.clone(),
        }
    }
}

// ── Workspace config ────────────────────────────────────────────────────────

/// Thin workspace-level config from `starpod.toml`.
/// All fields optional — these serve as defaults for agents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub server_addr: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub compaction_model: Option<String>,
    #[serde(default)]
    pub providers: Option<ProvidersConfig>,
    #[serde(default)]
    pub memory: Option<MemoryConfig>,
    #[serde(default)]
    pub cron: Option<CronConfig>,
    #[serde(default)]
    pub compaction: Option<CompactionConfig>,
    #[serde(default)]
    pub attachments: Option<AttachmentsConfig>,
}

// ── Config loading ──────────────────────────────────────────────────────────

/// Load agent config from resolved paths.
///
/// - **Workspace**: parse `starpod.toml` as base, deep-merge `agent.toml` on top
/// - **SingleAgent**: parse `.starpod/agent.toml` directly
/// - Loads `.env` via dotenvy if present
pub fn load_agent_config(paths: &ResolvedPaths) -> crate::Result<AgentConfig> {
    // Load .env if present
    if let Some(ref env_file) = paths.env_file {
        if let Err(e) = dotenvy::from_path_override(env_file) {
            warn!(path = %env_file.display(), error = %e, "Failed to load .env file");
        }
    }

    match &paths.mode {
        Mode::SingleAgent { .. } => {
            // Direct load from agent.toml
            if !paths.agent_toml.is_file() {
                return Err(StarpodError::Config(format!(
                    "Agent config not found: {}",
                    paths.agent_toml.display()
                )));
            }
            let content = std::fs::read_to_string(&paths.agent_toml).map_err(|e| {
                StarpodError::Config(format!(
                    "Failed to read {}: {}",
                    paths.agent_toml.display(),
                    e
                ))
            })?;

            // Warn about any credentials left in the config file
            if let Ok(raw) = toml::from_str::<toml::Value>(&content) {
                crate::config::warn_credentials_in_toml(&raw, &paths.agent_toml.display().to_string());
            }

            let mut config: AgentConfig = toml::from_str(&content)
                .map_err(|e| StarpodError::Config(format!("Invalid agent.toml: {}", e)))?;

            // Use agent_name as name if name wasn't explicitly set
            if config.name == "Aster" && config.agent_name != "Aster" {
                config.name = config.agent_name.clone();
            }

            Ok(config)
        }
        Mode::Workspace { root, agent_name } => {
            // Parse starpod.toml as TOML value tree (base defaults)
            let workspace_toml = root.join("starpod.toml");
            let mut base_value = if workspace_toml.is_file() {
                let content = std::fs::read_to_string(&workspace_toml).map_err(|e| {
                    StarpodError::Config(format!(
                        "Failed to read {}: {}",
                        workspace_toml.display(),
                        e
                    ))
                })?;
                let val = toml::from_str::<toml::Value>(&content)
                    .map_err(|e| StarpodError::Config(format!("Invalid starpod.toml: {}", e)))?;
                crate::config::warn_credentials_in_toml(&val, &workspace_toml.display().to_string());
                val
            } else {
                toml::Value::Table(Default::default())
            };

            // Deep-merge agent.toml on top
            if paths.agent_toml.is_file() {
                let content = std::fs::read_to_string(&paths.agent_toml).map_err(|e| {
                    StarpodError::Config(format!(
                        "Failed to read {}: {}",
                        paths.agent_toml.display(),
                        e
                    ))
                })?;
                let agent_value: toml::Value = toml::from_str(&content)
                    .map_err(|e| StarpodError::Config(format!("Invalid agent.toml: {}", e)))?;
                crate::config::warn_credentials_in_toml(&agent_value, &paths.agent_toml.display().to_string());
                deep_merge(&mut base_value, agent_value);
            }

            // Deserialize merged config
            let mut config: AgentConfig = base_value
                .try_into()
                .map_err(|e| StarpodError::Config(format!("Invalid config: {}", e)))?;

            // Set the agent name from the directory name
            config.name = agent_name.clone();

            Ok(config)
        }
    }
}

/// Synchronous config reload for file watcher (workspace-aware).
pub fn reload_agent_config(paths: &ResolvedPaths) -> crate::Result<AgentConfig> {
    load_agent_config(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── detect_mode ─────────────────────────────────────────────────────

    #[test]
    fn detect_single_agent_mode() {
        let tmp = TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(starpod_dir.join("agent.toml"), "").unwrap();

        let mode = detect_mode_from(None, tmp.path()).unwrap();
        match mode {
            Mode::SingleAgent { starpod_dir: dir } => {
                assert!(dir.ends_with(".starpod"));
            }
            _ => panic!("Expected SingleAgent mode"),
        }
    }

    #[test]
    fn detect_workspace_mode() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("starpod.toml"), "").unwrap();
        let agent_dir = root.join("agents").join("test-bot");
        std::fs::create_dir_all(&agent_dir).unwrap();

        let mode = detect_mode_from(None, &agent_dir).unwrap();
        match mode {
            Mode::Workspace { root: detected_root, agent_name } => {
                assert_eq!(detected_root, root);
                assert_eq!(agent_name, "test-bot");
            }
            _ => panic!("Expected Workspace mode"),
        }
    }

    #[test]
    fn detect_workspace_with_explicit_name() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("starpod.toml"), "").unwrap();

        let mode = detect_mode_from(Some("sales-rep"), tmp.path()).unwrap();
        match mode {
            Mode::Workspace { agent_name, .. } => {
                assert_eq!(agent_name, "sales-rep");
            }
            _ => panic!("Expected Workspace mode"),
        }
    }

    #[test]
    fn detect_workspace_requires_agent_name() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("starpod.toml"), "").unwrap();

        let err = detect_mode_from(None, tmp.path()).unwrap_err();
        assert!(err.to_string().contains("no agent name"));
    }

    #[test]
    fn detect_no_project_errors() {
        let tmp = TempDir::new().unwrap();
        let err = detect_mode_from(None, tmp.path()).unwrap_err();
        assert!(err.to_string().contains("No starpod.toml"));
    }

    // ── ResolvedPaths ───────────────────────────────────────────────────

    #[test]
    fn resolved_paths_single_agent() {
        let starpod_dir = PathBuf::from("/app/.starpod");
        let mode = Mode::SingleAgent {
            starpod_dir: starpod_dir.clone(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();

        assert_eq!(paths.agent_toml, PathBuf::from("/app/.starpod/agent.toml"));
        assert_eq!(paths.agent_home, PathBuf::from("/app/.starpod"));
        assert_eq!(paths.data_dir, PathBuf::from("/app/.starpod/data"));
        assert_eq!(paths.skills_dir, PathBuf::from("/app/.starpod/skills"));
        assert_eq!(paths.project_root, PathBuf::from("/app"));
        assert!(paths.env_file.is_none());
    }

    #[test]
    fn resolved_paths_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        // Create .env so it gets picked up
        std::fs::write(root.join(".env"), "KEY=val").unwrap();

        let mode = Mode::Workspace {
            root: root.clone(),
            agent_name: "sales-rep".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();

        assert_eq!(
            paths.agent_toml,
            root.join("agents/sales-rep/agent.toml")
        );
        assert_eq!(paths.agent_home, root.join("agents/sales-rep"));
        assert_eq!(paths.data_dir, root.join("agents/sales-rep/data"));
        assert_eq!(paths.skills_dir, root.join("skills"));
        assert_eq!(paths.project_root, root);
        assert!(paths.env_file.is_some());
    }

    // ── load_agent_config ───────────────────────────────────────────────

    #[test]
    fn load_single_agent_config() {
        let tmp = TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(
            starpod_dir.join("agent.toml"),
            r#"
agent_name = "TestBot"
model = "claude-sonnet-4-6"
provider = "anthropic"
"#,
        )
        .unwrap();

        let mode = Mode::SingleAgent {
            starpod_dir: starpod_dir.clone(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let config = load_agent_config(&paths).unwrap();

        assert_eq!(config.agent_name, "TestBot");
        assert_eq!(config.model, "claude-sonnet-4-6");
    }

    #[test]
    fn load_workspace_config_merges() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Workspace defaults
        std::fs::write(
            root.join("starpod.toml"),
            r#"
provider = "anthropic"
model = "claude-haiku-4-5"
max_turns = 20
"#,
        )
        .unwrap();

        // Agent overrides
        let agent_dir = root.join("agents").join("my-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("agent.toml"),
            r#"
model = "claude-sonnet-4-6"
agent_name = "MyAgent"
"#,
        )
        .unwrap();

        let mode = Mode::Workspace {
            root: root.to_path_buf(),
            agent_name: "my-agent".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let config = load_agent_config(&paths).unwrap();

        // Agent override wins
        assert_eq!(config.model, "claude-sonnet-4-6");
        assert_eq!(config.agent_name, "MyAgent");
        // Workspace default applies
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.max_turns, 20);
        // Name is from directory
        assert_eq!(config.name, "my-agent");
    }

    #[test]
    fn load_workspace_config_no_agent_toml() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        std::fs::write(
            root.join("starpod.toml"),
            r#"
provider = "openai"
model = "gpt-4o"
"#,
        )
        .unwrap();

        let agent_dir = root.join("agents").join("no-config");
        std::fs::create_dir_all(&agent_dir).unwrap();

        let mode = Mode::Workspace {
            root: root.to_path_buf(),
            agent_name: "no-config".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let config = load_agent_config(&paths).unwrap();

        // Gets workspace defaults
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.name, "no-config");
    }

    #[test]
    fn agent_config_to_starpod_config_bridge() {
        let config = AgentConfig {
            agent_name: "TestBot".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            ..Default::default()
        };
        let paths = ResolvedPaths {
            mode: Mode::SingleAgent {
                starpod_dir: PathBuf::from("/app/.starpod"),
            },
            agent_toml: PathBuf::from("/app/.starpod/agent.toml"),
            agent_home: PathBuf::from("/app/.starpod"),
            data_dir: PathBuf::from("/app/.starpod/data"),
            skills_dir: PathBuf::from("/app/.starpod/skills"),
            project_root: PathBuf::from("/app"),
            env_file: None,
        };

        let starpod_config = config.into_starpod_config(&paths);
        assert_eq!(starpod_config.agent_name, "TestBot");
        assert_eq!(starpod_config.model, "claude-sonnet-4-6");
        assert_eq!(starpod_config.data_dir, PathBuf::from("/app/.starpod/data"));
        assert_eq!(starpod_config.project_root, PathBuf::from("/app"));
    }

    #[test]
    fn load_env_file_in_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        std::fs::write(root.join("starpod.toml"), "").unwrap();
        std::fs::write(root.join(".env"), "STARPOD_TEST_VAR_42=hello_workspace\n").unwrap();

        let agent_dir = root.join("agents").join("env-test");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("agent.toml"), "").unwrap();

        let mode = Mode::Workspace {
            root: root.to_path_buf(),
            agent_name: "env-test".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let _config = load_agent_config(&paths).unwrap();

        assert_eq!(
            std::env::var("STARPOD_TEST_VAR_42").ok(),
            Some("hello_workspace".to_string())
        );
        std::env::remove_var("STARPOD_TEST_VAR_42");
    }

    #[test]
    fn infer_agent_name_from_agents_subdir() {
        let workspace = PathBuf::from("/home/user/my-platform");
        let cwd = PathBuf::from("/home/user/my-platform/agents/sales-rep");
        assert_eq!(
            infer_agent_name_from_cwd(&cwd, &workspace),
            Some("sales-rep".to_string())
        );
    }

    #[test]
    fn infer_agent_name_not_in_agents() {
        let workspace = PathBuf::from("/home/user/my-platform");
        let cwd = PathBuf::from("/home/user/my-platform/skills/email-draft");
        assert_eq!(infer_agent_name_from_cwd(&cwd, &workspace), None);
    }

    #[test]
    fn infer_agent_name_at_workspace_root() {
        let workspace = PathBuf::from("/home/user/my-platform");
        let cwd = PathBuf::from("/home/user/my-platform");
        assert_eq!(infer_agent_name_from_cwd(&cwd, &workspace), None);
    }

    // ── detect_mode edge cases ──────────────────────────────────────

    #[test]
    fn detect_old_starpod_dir_without_agent_toml() {
        let tmp = TempDir::new().unwrap();
        // Old layout: .starpod/ exists but no agent.toml
        std::fs::create_dir_all(tmp.path().join(".starpod")).unwrap();

        let err = detect_mode_from(None, tmp.path()).unwrap_err();
        assert!(err.to_string().contains("no agent.toml"));
    }

    #[test]
    fn single_agent_takes_priority_over_workspace() {
        let tmp = TempDir::new().unwrap();
        // Both modes present: single-agent should win
        std::fs::write(tmp.path().join("starpod.toml"), "").unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(starpod_dir.join("agent.toml"), "").unwrap();

        let mode = detect_mode_from(None, tmp.path()).unwrap();
        assert!(matches!(mode, Mode::SingleAgent { .. }));
    }

    // ── reload_agent_config ─────────────────────────────────────────

    #[test]
    fn reload_agent_config_returns_updated_values() {
        let tmp = TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(
            starpod_dir.join("agent.toml"),
            "model = \"claude-haiku-4-5\"\n",
        ).unwrap();

        let mode = Mode::SingleAgent { starpod_dir: starpod_dir.clone() };
        let paths = ResolvedPaths::resolve(&mode).unwrap();

        let config1 = reload_agent_config(&paths).unwrap();
        assert_eq!(config1.model, "claude-haiku-4-5");

        // Update config on disk
        std::fs::write(
            starpod_dir.join("agent.toml"),
            "model = \"claude-sonnet-4-6\"\n",
        ).unwrap();

        let config2 = reload_agent_config(&paths).unwrap();
        assert_eq!(config2.model, "claude-sonnet-4-6");
    }

    // ── AgentConfig defaults ────────────────────────────────────────

    #[test]
    fn agent_config_defaults_are_sane() {
        let config = AgentConfig::default();
        assert_eq!(config.name, "Aster");
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.model, "claude-haiku-4-5");
        assert_eq!(config.max_turns, 30);
        assert_eq!(config.max_tokens, 16384);
        assert!(config.skills.is_empty());
    }

    #[test]
    fn agent_config_deserializes_skills_filter() {
        let toml = r#"
skills = ["email-draft", "code-review"]
model = "gpt-4o"
"#;
        let config: AgentConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.skills, vec!["email-draft", "code-review"]);
        assert_eq!(config.model, "gpt-4o");
    }

    // ── WorkspaceConfig ─────────────────────────────────────────────

    #[test]
    fn workspace_config_all_optional() {
        let config: WorkspaceConfig = toml::from_str("").unwrap();
        assert!(config.provider.is_none());
        assert!(config.model.is_none());
        assert!(config.max_turns.is_none());
    }

    #[test]
    fn workspace_config_parses_partial() {
        let toml = r#"
provider = "anthropic"
model = "claude-sonnet-4-6"
"#;
        let config: WorkspaceConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.provider.as_deref(), Some("anthropic"));
        assert_eq!(config.model.as_deref(), Some("claude-sonnet-4-6"));
        assert!(config.max_turns.is_none());
    }

    // ── load_agent_config edge cases ────────────────────────────────

    #[test]
    fn load_single_agent_missing_file_errors() {
        let tmp = TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        // No agent.toml written

        let mode = Mode::SingleAgent { starpod_dir };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let err = load_agent_config(&paths).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn load_single_agent_name_from_agent_name() {
        let tmp = TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(
            starpod_dir.join("agent.toml"),
            "agent_name = \"CustomBot\"\n",
        ).unwrap();

        let mode = Mode::SingleAgent { starpod_dir };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let config = load_agent_config(&paths).unwrap();

        // name should be derived from agent_name when default
        assert_eq!(config.name, "CustomBot");
        assert_eq!(config.agent_name, "CustomBot");
    }

    // ── Credential-in-config backward compat ────────────────────────

    #[test]
    fn load_config_with_legacy_api_key_still_parses() {
        // Old configs that still contain api_key / bot_token must load
        // without error — the keys are silently dropped and a warning
        // is emitted via tracing.
        let tmp = TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(
            starpod_dir.join("agent.toml"),
            r#"
            [providers.anthropic]
            api_key = "sk-ant-should-be-ignored"
            base_url = "https://custom.example.com"

            [channels.telegram]
            bot_token = "123:ABC"
            gap_minutes = 120
            "#,
        ).unwrap();

        let mode = Mode::SingleAgent { starpod_dir };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let config = load_agent_config(&paths).unwrap();

        // Credential fields no longer exist on the structs, but the
        // config loaded without error (backward compat).
        let p = config.providers.anthropic.as_ref().unwrap();
        assert_eq!(p.base_url.as_deref(), Some("https://custom.example.com"));
        let tg = config.channels.telegram.as_ref().unwrap();
        assert_eq!(tg.gap_minutes, Some(120));
    }

    #[test]
    fn load_workspace_config_with_legacy_credentials_still_parses() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::write(
            root.join("starpod.toml"),
            r#"
            provider = "anthropic"
            [providers.anthropic]
            api_key = "sk-ant-legacy"
            "#,
        ).unwrap();
        let agent_dir = root.join("agents").join("test-bot");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("agent.toml"),
            r#"
            [channels.telegram]
            bot_token = "123:legacy"
            "#,
        ).unwrap();

        let mode = Mode::Workspace { root, agent_name: "test-bot".to_string() };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let config = load_agent_config(&paths).unwrap();

        // Both files had legacy credential keys — config still loads fine.
        assert_eq!(config.provider, "anthropic");
        assert!(config.channels.telegram.is_some());
    }
}
