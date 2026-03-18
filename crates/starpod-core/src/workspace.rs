//! Multi-agent workspace detection, path resolution, and config loading.
//!
//! Starpod supports three operating modes, detected automatically:
//!
//! - **Workspace** (dev): `starpod.toml` in CWD or parent walk-up, multiple
//!   agents under `agents/<name>/`, skills in `skills/` (copied to instances).
//!   `starpod.toml` is **scaffolding only** — it provides defaults when
//!   creating new agents (`starpod agent new`), but is NOT read at runtime.
//!   Each `agent.toml` is self-contained.
//! - **Instance** (dev runtime): CWD inside `.instances/<name>/`, workspace
//!   root identified by sibling `starpod.toml`.
//! - **SingleAgent** (prod): `.starpod/agent.toml` in CWD, everything self-contained.
//!
//! # Directory layouts
//!
//! **Workspace mode (blueprints):**
//! ```text
//! workspace/
//! +-- starpod.toml                    # scaffolding template (git-tracked, not read at runtime)
//! +-- .env                            # production secrets (gitignored)
//! +-- .env.dev                        # development overrides (gitignored)
//! +-- skills/                         # shared skills (git-tracked)
//! +-- agents/                         # BLUEPRINTS (git-tracked)
//! |   +-- aster/
//! |       +-- agent.toml
//! |       +-- SOUL.md
//! |       +-- files/                  # template filesystem
//! +-- .instances/                     # RUNTIME (gitignored)
//!     +-- aster/                      # agent's filesystem root
//!         +-- .starpod/
//!         |   +-- agent.toml          # copied from blueprint
//!         |   +-- SOUL.md
//!         |   +-- .env                # single file (from workspace .env.dev or .env)
//!         |   +-- HEARTBEAT.md        # agent-level heartbeat (optional)
//!         |   +-- BOOT.md             # startup instructions (optional)
//!         |   +-- BOOTSTRAP.md        # one-time init (self-destructing)
//!         |   +-- db/                 # SQLite databases
//!         |   +-- users/admin/        # per-user data
//!         |   +-- skills/
//!         +-- reports/                # agent creates freely
//! ```
//!
//! **Single-agent mode (prod):**
//! ```text
//! /srv/aster/                         # agent's filesystem root
//! +-- .starpod/
//! |   +-- agent.toml
//! |   +-- SOUL.md
//! |   +-- .env
//! |   +-- HEARTBEAT.md
//! |   +-- BOOT.md
//! |   +-- BOOTSTRAP.md
//! |   +-- db/                         # SQLite databases
//! |   +-- users/admin/
//! |   +-- skills/
//! +-- reports/                        # agent-produced files
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
    AttachmentsConfig, ChannelsConfig, CompactionConfig, CronConfig, FollowupMode,
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
    /// Instance mode: CWD is inside `.instances/<name>/`.
    Instance { instance_root: PathBuf, agent_name: String },
}

/// Detect the operating mode from the current directory.
///
/// - CWD has `.starpod/agent.toml` -> `SingleAgent`
/// - CWD inside `.instances/<name>/` with `starpod.toml` sibling -> `Instance`
/// - Walk up for `starpod.toml` -> `Workspace` (requires `agent_name`)
/// - Neither -> error
pub fn detect_mode(agent_name: Option<&str>) -> crate::Result<Mode> {
    let cwd = std::env::current_dir().map_err(|e| {
        StarpodError::Config(format!("Failed to get current directory: {}", e))
    })?;
    detect_mode_from(agent_name, &cwd)
}

/// Like `detect_mode` but starting from a given directory instead of CWD.
pub fn detect_mode_from(agent_name: Option<&str>, start_dir: &Path) -> crate::Result<Mode> {
    // Check single-agent mode first: .starpod/agent.toml in start_dir
    let starpod_dir = start_dir.join(".starpod");
    if starpod_dir.join("agent.toml").is_file() {
        return Ok(Mode::SingleAgent { starpod_dir });
    }

    // Check instance mode: walk up looking for `.instances/` parent with `starpod.toml` sibling
    if let Some(instance_mode) = detect_instance_mode(start_dir) {
        return Ok(instance_mode);
    }

    // Walk up looking for starpod.toml (workspace mode)
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

/// Detect Instance mode by walking up from `start_dir` looking for a `.instances/`
/// parent directory that has a sibling `starpod.toml` (i.e. the workspace root).
fn detect_instance_mode(start_dir: &Path) -> Option<Mode> {
    let mut dir = start_dir.to_path_buf();
    loop {
        // Check if `dir` is `.instances/<name>/` by examining its parent structure
        if let Some(parent) = dir.parent() {
            if parent.file_name().and_then(|n| n.to_str()) == Some(".instances") {
                // parent is `.instances/`, grandparent should have `starpod.toml`
                if let Some(workspace_root) = parent.parent() {
                    if workspace_root.join("starpod.toml").is_file() {
                        let agent_name = dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                            .to_string();
                        // instance_root is the deepest `.instances/<name>/` ancestor
                        // of start_dir (or start_dir itself)
                        return Some(Mode::Instance {
                            instance_root: dir,
                            agent_name,
                        });
                    }
                }
            }
        }
        if !dir.pop() {
            break;
        }
        // Don't walk past filesystem root
        if dir == Path::new("/") || dir == Path::new("") {
            break;
        }
    }
    None
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
    /// Agent home directory (.starpod/ dir — internal state).
    pub agent_home: PathBuf,
    /// Database directory for SQLite DBs (agent_home/db/).
    pub db_dir: PathBuf,
    /// Skills directory (.starpod/skills/ — instance-local).
    pub skills_dir: PathBuf,
    /// Project/workspace root.
    pub project_root: PathBuf,
    /// Agent's filesystem sandbox root (where agent can freely create files).
    pub instance_root: PathBuf,
    /// Per-user data directory (.starpod/users/).
    pub users_dir: PathBuf,
    /// .env file path (if it exists).
    pub env_file: Option<PathBuf>,
}

impl ResolvedPaths {
    /// Resolve all paths from a detected mode.
    ///
    /// Automatically migrates old `data/` layout to `db/` + root-level lifecycle files.
    pub fn resolve(mode: &Mode) -> crate::Result<Self> {
        match mode {
            Mode::SingleAgent { starpod_dir } => {
                let agent_toml = starpod_dir.join("agent.toml");
                let agent_home = starpod_dir.clone();
                let db_dir = starpod_dir.join("db");
                let skills_dir = starpod_dir.join("skills");
                let users_dir = starpod_dir.join("users");
                let instance_root = starpod_dir
                    .parent()
                    .ok_or_else(|| {
                        StarpodError::Config("Invalid .starpod/ path".to_string())
                    })?
                    .to_path_buf();
                let project_root = instance_root.clone();
                let env_path = starpod_dir.join(".env");

                Ok(Self {
                    mode: mode.clone(),
                    agent_toml,
                    agent_home,
                    db_dir,
                    skills_dir,
                    project_root,
                    instance_root,
                    users_dir,
                    env_file: if env_path.is_file() {
                        Some(env_path)
                    } else {
                        None
                    },
                })
            }
            Mode::Instance { instance_root, agent_name: _ } => {
                let agent_home = instance_root.join(".starpod");
                let agent_toml = agent_home.join("agent.toml");
                let db_dir = agent_home.join("db");
                let users_dir = agent_home.join("users");
                let env_path = agent_home.join(".env");
                // Skills always live at instance level (.starpod/skills/).
                // Workspace skills are copied into the instance during blueprint application.
                let skills_dir = agent_home.join("skills");
                // The agent's sandbox is the instance directory, not the
                // workspace root.  project_root controls cwd, system-prompt
                // paths, and file-tool boundaries.
                let project_root = instance_root.clone();

                Ok(Self {
                    mode: mode.clone(),
                    agent_toml,
                    agent_home,
                    db_dir,
                    skills_dir,
                    project_root,
                    instance_root: instance_root.clone(),
                    users_dir,
                    env_file: if env_path.is_file() {
                        Some(env_path)
                    } else {
                        None
                    },
                })
            }
            Mode::Workspace { root, agent_name } => {
                // Check if an instance exists — if so, use it
                let instance_dir = root.join(".instances").join(agent_name);
                if instance_dir.join(".starpod").join("agent.toml").is_file() {
                    // Instance exists, resolve as instance
                    let instance_mode = Mode::Instance {
                        instance_root: instance_dir,
                        agent_name: agent_name.clone(),
                    };
                    return Self::resolve(&instance_mode);
                }

                // Fall back to old workspace layout (backward compat)
                let agents_dir = root.join("agents").join(agent_name);
                let agent_toml = agents_dir.join("agent.toml");
                let db_dir = agents_dir.join("db");
                let skills_dir = root.join("skills");
                let users_dir = agents_dir.join("users");
                let env_path = root.join(".env");

                Ok(Self {
                    mode: mode.clone(),
                    agent_toml,
                    agent_home: agents_dir.clone(),
                    db_dir,
                    skills_dir,
                    project_root: root.clone(),
                    instance_root: agents_dir,
                    users_dir,
                    env_file: if env_path.is_file() {
                        Some(env_path)
                    } else {
                        None
                    },
                })
            }
        }
    }

    /// Migrate old `data/` layout to `db/` + root-level lifecycle files.
    ///
    /// If `agent_home/data/` exists and `agent_home/db/` does not, performs:
    /// 1. Move `data/*.db` → `db/`
    /// 2. Move `data/HEARTBEAT.md`, `data/BOOT.md`, `data/BOOTSTRAP.md` → `agent_home/`
    /// 3. Move `data/USER.md`, `data/MEMORY.md`, `data/memory/` → `users/admin/` (if not already there)
    /// 4. Remove `data/SOUL.md` (duplicate of `agent_home/SOUL.md`)
    /// 5. Remove `data/knowledge/` (no longer used)
    /// 6. Clean up empty `data/`
    pub fn migrate_if_needed(&self) {
        let data_dir = self.agent_home.join("data");
        if !data_dir.is_dir() || self.db_dir.is_dir() {
            return; // Nothing to migrate
        }

        tracing::info!("Migrating old data/ layout to db/ + root-level files");

        // 1. Create db/ and move databases
        if let Err(e) = std::fs::create_dir_all(&self.db_dir) {
            tracing::error!(error = %e, "Migration: failed to create db/");
            return;
        }
        for db_name in &["memory.db", "session.db", "cron.db", "vault.db"] {
            let src = data_dir.join(db_name);
            let dst = self.db_dir.join(db_name);
            if src.is_file() && !dst.exists() {
                if let Err(e) = std::fs::rename(&src, &dst) {
                    tracing::warn!(file = %db_name, error = %e, "Migration: failed to move DB file");
                }
            }
        }
        // Also move WAL/SHM files for SQLite
        for suffix in &["-wal", "-shm"] {
            for db_name in &["memory.db", "session.db", "cron.db", "vault.db"] {
                let name = format!("{}{}", db_name, suffix);
                let src = data_dir.join(&name);
                let dst = self.db_dir.join(&name);
                if src.is_file() && !dst.exists() {
                    let _ = std::fs::rename(&src, &dst);
                }
            }
        }

        // 2. Move lifecycle files to agent_home root
        for name in &["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md"] {
            let src = data_dir.join(name);
            let dst = self.agent_home.join(name);
            if src.is_file() && !dst.exists() {
                if let Err(e) = std::fs::rename(&src, &dst) {
                    tracing::warn!(file = %name, error = %e, "Migration: failed to move lifecycle file");
                }
            }
        }

        // 3. Move user files to users/admin/ (if not already there)
        let admin_dir = self.users_dir.join("admin");
        let _ = std::fs::create_dir_all(&admin_dir);
        for name in &["USER.md", "MEMORY.md"] {
            let src = data_dir.join(name);
            let dst = admin_dir.join(name);
            if src.is_file() && !dst.exists() {
                let _ = std::fs::rename(&src, &dst);
            }
        }
        let src_memory = data_dir.join("memory");
        let dst_memory = admin_dir.join("memory");
        if src_memory.is_dir() && !dst_memory.exists() {
            let _ = std::fs::rename(&src_memory, &dst_memory);
        }

        // 4. Remove data/SOUL.md (duplicate)
        let _ = std::fs::remove_file(data_dir.join("SOUL.md"));

        // 5. Remove data/knowledge/ (no longer used)
        let _ = std::fs::remove_dir_all(data_dir.join("knowledge"));

        // 6. Try to remove empty data/
        let _ = std::fs::remove_dir(&data_dir);

        tracing::info!("Migration complete: data/ → db/ + root-level files");
    }

    /// Build a `UserContext` for a specific user ID.
    pub fn user_context(&self, user_id: &str) -> UserContext {
        let user_dir = self.users_dir.join(user_id);
        UserContext {
            user_id: user_id.to_string(),
            user_dir: user_dir.clone(),
            user_md: user_dir.join("USER.md"),
            memory_md: user_dir.join("MEMORY.md"),
            memory_dir: user_dir.join("memory"),
            env_file: {
                let p = user_dir.join(".env");
                if p.is_file() { Some(p) } else { None }
            },
        }
    }
}

/// Per-user context derived from resolved paths.
#[derive(Debug, Clone)]
pub struct UserContext {
    /// User identifier.
    pub user_id: String,
    /// User's data directory (.starpod/users/<id>/).
    pub user_dir: PathBuf,
    /// Path to USER.md.
    pub user_md: PathBuf,
    /// Path to MEMORY.md.
    pub memory_md: PathBuf,
    /// Path to memory/ subdirectory.
    pub memory_dir: PathBuf,
    /// Per-user .env file (overrides instance .env).
    pub env_file: Option<PathBuf>,
}

// ── .env loading ────────────────────────────────────────────────────────────

/// Load .env files with hierarchical override:
/// 1. Instance-level .env (from agent_home)
/// 2. Per-user .env (overrides instance)
pub fn load_env(agent_home: &Path, user_id: Option<&str>) {
    // Instance-level .env
    let env = agent_home.join(".env");
    if env.is_file() {
        if let Err(e) = dotenvy::from_path_override(&env) {
            warn!(path = %env.display(), error = %e, "Failed to load .env file");
        }
    }
    // Per-user .env (overrides instance)
    if let Some(uid) = user_id {
        let user_env = agent_home.join("users").join(uid).join(".env");
        if user_env.is_file() {
            if let Err(e) = dotenvy::from_path_override(&user_env) {
                warn!(path = %user_env.display(), error = %e, "Failed to load user .env file");
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
            db_dir: paths.db_dir.clone(),
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
/// All modes load `agent.toml` directly — each agent is self-contained.
/// `starpod.toml` is scaffolding only (used when creating agents, not at runtime).
/// Loads `.env` via dotenvy if present.
pub fn load_agent_config(paths: &ResolvedPaths) -> crate::Result<AgentConfig> {
    // Load .env hierarchy
    match &paths.mode {
        Mode::Instance { instance_root, .. } => {
            // For instances: load workspace .env first (base), then instance .env (override)
            if let Some(workspace_root) = instance_root.parent().and_then(|p| p.parent()) {
                let workspace_env = workspace_root.join(".env");
                if workspace_env.is_file() {
                    if let Err(e) = dotenvy::from_path_override(&workspace_env) {
                        warn!(path = %workspace_env.display(), error = %e, "Failed to load workspace .env");
                    }
                }
            }
            // Instance .env overrides workspace .env
            load_env(&paths.agent_home, None);
        }
        Mode::SingleAgent { .. } => {
            load_env(&paths.agent_home, None);
        }
        Mode::Workspace { .. } => {
            // Load workspace .env
            if let Some(ref env_file) = paths.env_file {
                if let Err(e) = dotenvy::from_path_override(env_file) {
                    warn!(path = %env_file.display(), error = %e, "Failed to load .env file");
                }
            }
        }
    }

    // All modes: direct load from agent.toml (no merge with starpod.toml)
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

    // For Instance mode, set name from the agent_name in the mode
    if let Mode::Instance { agent_name, .. } = &paths.mode {
        if config.name == "Aster" {
            config.name = agent_name.clone();
        }
    }

    // For Workspace mode, set name from directory
    if let Mode::Workspace { agent_name, .. } = &paths.mode {
        config.name = agent_name.clone();
    }

    Ok(config)
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

    #[test]
    fn detect_instance_mode() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("starpod.toml"), "").unwrap();
        let instance_dir = root.join(".instances").join("aster");
        let starpod_dir = instance_dir.join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();

        let mode = detect_mode_from(None, &instance_dir).unwrap();
        match mode {
            Mode::Instance { instance_root, agent_name } => {
                assert_eq!(instance_root, instance_dir);
                assert_eq!(agent_name, "aster");
            }
            _ => panic!("Expected Instance mode, got {:?}", mode),
        }
    }

    #[test]
    fn detect_instance_mode_from_subdirectory() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("starpod.toml"), "").unwrap();
        let instance_dir = root.join(".instances").join("aster");
        std::fs::create_dir_all(&instance_dir).unwrap();
        let subdir = instance_dir.join("reports").join("weekly");
        std::fs::create_dir_all(&subdir).unwrap();

        let mode = detect_mode_from(None, &subdir).unwrap();
        match mode {
            Mode::Instance { instance_root, agent_name } => {
                assert_eq!(instance_root, instance_dir);
                assert_eq!(agent_name, "aster");
            }
            _ => panic!("Expected Instance mode from subdirectory"),
        }
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
        assert_eq!(paths.db_dir, PathBuf::from("/app/.starpod/db"));
        assert_eq!(paths.skills_dir, PathBuf::from("/app/.starpod/skills"));
        assert_eq!(paths.project_root, PathBuf::from("/app"));
        assert_eq!(paths.instance_root, PathBuf::from("/app"));
        assert_eq!(paths.users_dir, PathBuf::from("/app/.starpod/users"));
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
        assert_eq!(paths.db_dir, root.join("agents/sales-rep/db"));
        assert_eq!(paths.skills_dir, root.join("skills"));
        assert_eq!(paths.project_root, root);
        assert_eq!(paths.instance_root, root.join("agents/sales-rep"));
        assert_eq!(paths.users_dir, root.join("agents/sales-rep/users"));
        assert!(paths.env_file.is_some());
    }

    #[test]
    fn resolved_paths_instance() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("starpod.toml"), "").unwrap();
        let instance_dir = root.join(".instances").join("aster");
        let starpod_dir = instance_dir.join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();

        let mode = Mode::Instance {
            instance_root: instance_dir.clone(),
            agent_name: "aster".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();

        assert_eq!(paths.agent_toml, starpod_dir.join("agent.toml"));
        assert_eq!(paths.agent_home, starpod_dir);
        assert_eq!(paths.db_dir, instance_dir.join(".starpod/db"));
        assert_eq!(paths.skills_dir, starpod_dir.join("skills"));
        assert_eq!(paths.project_root, instance_dir);
        assert_eq!(paths.instance_root, instance_dir);
        assert_eq!(paths.users_dir, instance_dir.join(".starpod/users"));
    }

    #[test]
    fn resolved_paths_workspace_with_instance() {
        // When a workspace has an active instance, resolve should use it
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("starpod.toml"), "").unwrap();
        let instance_dir = root.join(".instances").join("bot");
        let starpod_dir = instance_dir.join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(starpod_dir.join("agent.toml"), "").unwrap();

        let mode = Mode::Workspace {
            root: root.to_path_buf(),
            agent_name: "bot".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();

        // Should resolve as instance since .instances/bot/.starpod/agent.toml exists
        assert_eq!(paths.instance_root, instance_dir);
        assert_eq!(paths.agent_home, starpod_dir);
    }

    // ── UserContext ─────────────────────────────────────────────────────

    #[test]
    fn user_context_paths() {
        let starpod_dir = PathBuf::from("/app/.starpod");
        let mode = Mode::SingleAgent {
            starpod_dir: starpod_dir.clone(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let ctx = paths.user_context("admin");

        assert_eq!(ctx.user_id, "admin");
        assert_eq!(ctx.user_dir, PathBuf::from("/app/.starpod/users/admin"));
        assert_eq!(ctx.user_md, PathBuf::from("/app/.starpod/users/admin/USER.md"));
        assert_eq!(ctx.memory_md, PathBuf::from("/app/.starpod/users/admin/MEMORY.md"));
        assert_eq!(ctx.memory_dir, PathBuf::from("/app/.starpod/users/admin/memory"));
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
    fn load_workspace_config_self_contained() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // starpod.toml exists but is NOT read at runtime
        std::fs::write(
            root.join("starpod.toml"),
            r#"
provider = "openai"
model = "gpt-4o"
max_turns = 99
"#,
        )
        .unwrap();

        // Agent is self-contained — has all its own values
        let agent_dir = root.join("agents").join("my-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("agent.toml"),
            r#"
provider = "anthropic"
model = "claude-sonnet-4-6"
agent_name = "MyAgent"
max_turns = 30
"#,
        )
        .unwrap();

        let mode = Mode::Workspace {
            root: root.to_path_buf(),
            agent_name: "my-agent".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let config = load_agent_config(&paths).unwrap();

        // All values from agent.toml, NOT starpod.toml
        assert_eq!(config.model, "claude-sonnet-4-6");
        assert_eq!(config.agent_name, "MyAgent");
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.max_turns, 30);
        // Name is from directory
        assert_eq!(config.name, "my-agent");
    }

    #[test]
    fn load_workspace_config_no_agent_toml_errors() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        std::fs::write(root.join("starpod.toml"), "").unwrap();

        let agent_dir = root.join("agents").join("no-config");
        std::fs::create_dir_all(&agent_dir).unwrap();
        // No agent.toml — should error since agents must be self-contained

        let mode = Mode::Workspace {
            root: root.to_path_buf(),
            agent_name: "no-config".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let result = load_agent_config(&paths);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Agent config not found"));
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
            db_dir: PathBuf::from("/app/.starpod/db"),
            skills_dir: PathBuf::from("/app/.starpod/skills"),
            project_root: PathBuf::from("/app"),
            instance_root: PathBuf::from("/app"),
            users_dir: PathBuf::from("/app/.starpod/users"),
            env_file: None,
        };

        let starpod_config = config.into_starpod_config(&paths);
        assert_eq!(starpod_config.agent_name, "TestBot");
        assert_eq!(starpod_config.model, "claude-sonnet-4-6");
        assert_eq!(starpod_config.db_dir, PathBuf::from("/app/.starpod/db"));
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
        std::fs::write(agent_dir.join("agent.toml"), "provider = \"anthropic\"\n").unwrap();

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

        assert_eq!(config.provider, "anthropic");
        assert!(config.channels.telegram.is_some());
    }

    // ── Instance config loading ─────────────────────────────────────

    #[test]
    fn load_instance_config() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("starpod.toml"), "").unwrap();
        let instance_dir = root.join(".instances").join("aster");
        let starpod_dir = instance_dir.join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(
            starpod_dir.join("agent.toml"),
            r#"
agent_name = "Aster"
model = "claude-sonnet-4-6"
"#,
        ).unwrap();

        let mode = Mode::Instance {
            instance_root: instance_dir,
            agent_name: "aster".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let config = load_agent_config(&paths).unwrap();

        assert_eq!(config.model, "claude-sonnet-4-6");
        assert_eq!(config.name, "aster");
    }

    // ── load_env hierarchical ───────────────────────────────────────

    #[test]
    fn load_env_loads_instance_env() {
        let tmp = TempDir::new().unwrap();
        let agent_home = tmp.path();
        std::fs::write(
            agent_home.join(".env"),
            "STARPOD_INSTANCE_ENV_TEST=from_instance\n",
        ).unwrap();

        load_env(agent_home, None);
        assert_eq!(
            std::env::var("STARPOD_INSTANCE_ENV_TEST").ok(),
            Some("from_instance".to_string())
        );
        std::env::remove_var("STARPOD_INSTANCE_ENV_TEST");
    }

    #[test]
    fn load_env_user_overrides_instance() {
        let tmp = TempDir::new().unwrap();
        let agent_home = tmp.path();
        std::fs::write(
            agent_home.join(".env"),
            "STARPOD_OVERRIDE_TEST=instance_val\n",
        ).unwrap();

        let user_dir = agent_home.join("users").join("alice");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(
            user_dir.join(".env"),
            "STARPOD_OVERRIDE_TEST=alice_val\n",
        ).unwrap();

        load_env(agent_home, Some("alice"));
        assert_eq!(
            std::env::var("STARPOD_OVERRIDE_TEST").ok(),
            Some("alice_val".to_string())
        );
        std::env::remove_var("STARPOD_OVERRIDE_TEST");
    }

    // ── Instance .env loads workspace .env as base ──────────────────

    #[test]
    fn instance_config_loads_workspace_env() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Workspace .env with API key
        std::fs::write(root.join("starpod.toml"), "").unwrap();
        std::fs::write(root.join(".env"), "STARPOD_INST_ENV_TEST=from_workspace\n").unwrap();

        // Instance with agent.toml but NO .env
        let instance_dir = root.join(".instances").join("bot");
        let starpod_dir = instance_dir.join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(starpod_dir.join("agent.toml"), "").unwrap();

        let mode = Mode::Instance {
            instance_root: instance_dir,
            agent_name: "bot".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let _config = load_agent_config(&paths).unwrap();

        // Workspace .env should have been loaded
        assert_eq!(
            std::env::var("STARPOD_INST_ENV_TEST").ok(),
            Some("from_workspace".to_string()),
            "Instance mode should load workspace .env as base"
        );
        std::env::remove_var("STARPOD_INST_ENV_TEST");
    }

    #[test]
    fn instance_env_overrides_workspace_env() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Workspace .env
        std::fs::write(root.join("starpod.toml"), "").unwrap();
        std::fs::write(root.join(".env"), "STARPOD_INST_OVERRIDE_TEST=workspace_val\n").unwrap();

        // Instance .env overrides
        let instance_dir = root.join(".instances").join("bot");
        let starpod_dir = instance_dir.join(".starpod");
        std::fs::create_dir_all(&starpod_dir).unwrap();
        std::fs::write(starpod_dir.join("agent.toml"), "").unwrap();
        std::fs::write(starpod_dir.join(".env"), "STARPOD_INST_OVERRIDE_TEST=instance_val\n").unwrap();

        let mode = Mode::Instance {
            instance_root: instance_dir,
            agent_name: "bot".to_string(),
        };
        let paths = ResolvedPaths::resolve(&mode).unwrap();
        let _config = load_agent_config(&paths).unwrap();

        // Instance .env should override workspace .env
        assert_eq!(
            std::env::var("STARPOD_INST_OVERRIDE_TEST").ok(),
            Some("instance_val".to_string()),
            "Instance .env should override workspace .env"
        );
        std::env::remove_var("STARPOD_INST_OVERRIDE_TEST");
    }
}
