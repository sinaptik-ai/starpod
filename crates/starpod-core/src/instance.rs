//! Blueprint application and instance lifecycle.
//!
//! An agent **blueprint** lives in `agents/<name>/` (git-tracked) and contains
//! configuration, personality files, and template filesystem content.
//!
//! A runtime **instance** lives in `.instances/<name>/` (gitignored) and holds
//! the actual agent state: databases, memory, user data, and any files the
//! agent creates at runtime.
//!
//! ## Functions
//!
//! - [`apply_blueprint`] — copies a blueprint into an instance, creating the
//!   `.starpod/` directory structure. Idempotent: callers should check whether
//!   the instance already exists before calling, and only re-apply when the
//!   user explicitly requests a rebuild (e.g. `--build` flag).
//! - [`build_standalone`] — creates a self-contained `.starpod/` for CI/CD
//!   from explicit paths (no workspace required).
//! - [`create_ephemeral_instance`] — creates a throwaway instance in a temp
//!   directory for one-off commands like `starpod chat`. The returned
//!   [`tempfile::TempDir`] guard auto-deletes on drop.
//!
//! ## Directory layout
//!
//! - `config/` — blueprint-managed files (overwritten on every build)
//! - `skills/` — merged: blueprint skills overwrite by filename, user additions preserved
//! - `db/`, `users/` — runtime data (never touched by build)
//! - `.env` — environment secrets at starpod root (not in config/)

use std::path::Path;

use tracing::{debug, info};

use crate::error::StarpodError;

/// Legacy env source selector (no longer used — vault handles secrets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvSource {
    /// Development mode.
    Dev,
    /// Production mode.
    Prod,
}

/// Apply a blueprint directory to an instance directory.
///
/// # Layout created
///
/// ```text
/// .instances/<name>/           ← instance_dir (agent's filesystem root)
/// ├── .starpod/                ← internal state
/// │   ├── db/vault.db           ← encrypted secrets (populated at serve time from workspace .env)
/// │   ├── config/              ← blueprint-managed (overwritten on build)
/// │   │   ├── agent.toml
/// │   │   ├── SOUL.md
/// │   │   ├── HEARTBEAT.md
/// │   │   ├── BOOT.md
/// │   │   └── BOOTSTRAP.md
/// │   ├── skills/              ← merged (blueprint overrides, user additions preserved)
/// │   ├── db/                  ← SQLite databases (runtime)
/// │   └── users/
/// │       ├── admin/           ← auto-created default users
/// │       └── user/
/// │           ├── USER.md
/// │           ├── MEMORY.md
/// │           └── memory/      ← daily logs
/// ├── home/                    ← agent's visible filesystem (sandbox)
/// │   ├── desktop/
/// │   ├── documents/
/// │   ├── projects/
/// │   └── downloads/
/// └── ...
/// ```
///
/// # Preservation rules
///
/// - Existing user directories are never overwritten.
/// - Existing databases (db/*.db) are preserved.
/// - Blueprint `files/*` are synced to `home/`, the agent's visible filesystem.
/// - `config/` is always refreshed from the blueprint (agent.toml, SOUL.md, lifecycle files).
/// - `skills/` are merged: blueprint skills overwrite by filename, user-created skills preserved.
/// - `.env` is read from `workspace_dir` (top level), not from the blueprint.
pub fn apply_blueprint(
    blueprint_dir: &Path,
    instance_dir: &Path,
    workspace_dir: &Path,
    _env_source: EnvSource,
) -> crate::Result<()> {
    let starpod_dir = instance_dir.join(".starpod");
    let config_dir = starpod_dir.join("config");

    // 1. Create directory structure
    std::fs::create_dir_all(&config_dir).map_err(|e| {
        StarpodError::Config(format!(
            "Failed to create .starpod/config/ in {}: {}",
            instance_dir.display(),
            e
        ))
    })?;
    std::fs::create_dir_all(starpod_dir.join("db")).map_err(StarpodError::Io)?;
    std::fs::create_dir_all(starpod_dir.join("users")).map_err(StarpodError::Io)?;

    // 1b. Create home/ directory with placeholder subdirs
    let home_dir = instance_dir.join("home");
    for sub in &["desktop", "documents", "projects", "downloads"] {
        std::fs::create_dir_all(home_dir.join(sub)).map_err(StarpodError::Io)?;
    }

    // 2. Copy blueprint-managed files into config/ (always refresh)
    // Lifecycle files from blueprint
    for name in &["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md", "frontend.toml"] {
        let src = blueprint_dir.join(name);
        let dst = config_dir.join(name);
        if src.is_file() {
            std::fs::copy(&src, &dst).map_err(StarpodError::Io)?;
        } else if !dst.exists() {
            std::fs::write(&dst, "").map_err(StarpodError::Io)?;
        }
    }

    // agent.toml (always refresh)
    let src_toml = blueprint_dir.join("agent.toml");
    if src_toml.is_file() {
        std::fs::copy(&src_toml, config_dir.join("agent.toml"))
            .map_err(|e| StarpodError::Config(format!("Failed to copy agent.toml: {}", e)))?;
        debug!("Copied agent.toml from blueprint");
    }

    // SOUL.md (always refresh)
    let src_soul = blueprint_dir.join("SOUL.md");
    if src_soul.is_file() {
        std::fs::copy(&src_soul, config_dir.join("SOUL.md"))
            .map_err(|e| StarpodError::Config(format!("Failed to copy SOUL.md: {}", e)))?;
        debug!("Copied SOUL.md from blueprint");
    }

    // 3. .env is NOT copied into the instance — secrets are populated into
    //    vault.db at build time from the workspace .env. The vault is the
    //    sealed source of truth at serve time.

    // 4. Sync files/ → home/ directory
    let files_dir = blueprint_dir.join("files");
    if files_dir.is_dir() {
        sync_files(&files_dir, &home_dir)?;
        debug!("Synced template files from blueprint into home/");
    }

    // 5. Merge workspace skills/ → instance .starpod/skills/
    // Blueprint skills overwrite by filename, user-created skills preserved.
    let instance_skills = starpod_dir.join("skills");
    std::fs::create_dir_all(&instance_skills).map_err(StarpodError::Io)?;
    let workspace_skills = workspace_dir.join("skills");
    if workspace_skills.is_dir() {
        sync_skills(&workspace_skills, &instance_skills)?;
        debug!("Synced workspace skills to instance");
    }

    info!(
        blueprint = %blueprint_dir.display(),
        instance = %instance_dir.display(),
        "Blueprint applied to instance"
    );

    Ok(())
}

/// Build a standalone `.starpod/` from an agent blueprint directory.
///
/// Unlike [`apply_blueprint`] which operates within a workspace,
/// this function takes explicit paths to all inputs and creates a
/// self-contained `.starpod/` at `output_dir/.starpod/`.
///
/// # Arguments
///
/// - `blueprint_dir`: path to agent blueprint folder (must contain `agent.toml`)
/// - `output_dir`: where to create the `.starpod/` directory
/// - `skills_dir`: optional path to skills folder to include
/// - `env_file`: optional path to `.env` file — not copied to the output;
///   the caller (CLI) is responsible for populating `vault.db` from this file
///   after `build_standalone` returns.
pub fn build_standalone(
    blueprint_dir: &Path,
    output_dir: &Path,
    skills_dir: Option<&Path>,
    _env_file: Option<&Path>,
    force: bool,
) -> crate::Result<()> {
    let starpod_dir = output_dir.join(".starpod");
    let config_dir = starpod_dir.join("config");

    // Refuse to overwrite an existing instance unless --force is passed
    if starpod_dir.exists() && !force {
        return Err(StarpodError::Config(format!(
            ".starpod/ already exists in {}. Use --force to overwrite blueprint files.",
            output_dir.display()
        )));
    }

    // Validate blueprint has agent.toml
    let src_toml = blueprint_dir.join("agent.toml");
    if !src_toml.is_file() {
        return Err(StarpodError::Config(format!(
            "Blueprint directory {} does not contain agent.toml",
            blueprint_dir.display()
        )));
    }

    // 1. Create directory structure
    std::fs::create_dir_all(&config_dir).map_err(|e| {
        StarpodError::Config(format!(
            "Failed to create .starpod/config/ in {}: {}",
            output_dir.display(),
            e
        ))
    })?;
    std::fs::create_dir_all(starpod_dir.join("db")).map_err(StarpodError::Io)?;
    std::fs::create_dir_all(starpod_dir.join("users")).map_err(StarpodError::Io)?;

    // 1b. Create home/ directory with placeholder subdirs
    let home_dir = output_dir.join("home");
    for sub in &["desktop", "documents", "projects", "downloads"] {
        std::fs::create_dir_all(home_dir.join(sub)).map_err(StarpodError::Io)?;
    }

    // 2. Copy blueprint-managed files into config/ (always refresh)
    for name in &["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md", "frontend.toml"] {
        let src = blueprint_dir.join(name);
        let dst = config_dir.join(name);
        if src.is_file() {
            std::fs::copy(&src, &dst).map_err(StarpodError::Io)?;
        } else if !dst.exists() {
            std::fs::write(&dst, "").map_err(StarpodError::Io)?;
        }
    }

    // agent.toml (always refresh)
    std::fs::copy(&src_toml, config_dir.join("agent.toml"))
        .map_err(|e| StarpodError::Config(format!("Failed to copy agent.toml: {}", e)))?;
    debug!("Copied agent.toml from blueprint");

    // SOUL.md (always refresh)
    let src_soul = blueprint_dir.join("SOUL.md");
    if src_soul.is_file() {
        std::fs::copy(&src_soul, config_dir.join("SOUL.md"))
            .map_err(|e| StarpodError::Config(format!("Failed to copy SOUL.md: {}", e)))?;
        debug!("Copied SOUL.md from blueprint");
    }

    // deploy.toml (always refresh — needed by serve for vault population)
    let src_deploy = blueprint_dir.join("deploy.toml");
    if src_deploy.is_file() {
        std::fs::copy(&src_deploy, config_dir.join("deploy.toml"))
            .map_err(|e| StarpodError::Config(format!("Failed to copy deploy.toml: {}", e)))?;
        debug!("Copied deploy.toml from blueprint");
    }

    // 3. .env is NOT copied into the instance — secrets are sealed into
    //    vault.db at build time by the CLI. The vault is the source of truth.

    // 4. Sync files/ → home/ directory
    let files_dir = blueprint_dir.join("files");
    if files_dir.is_dir() {
        sync_files(&files_dir, &home_dir)?;
        debug!("Synced template files from blueprint into home/");
    }

    // 5. Merge skills: blueprint overrides by filename, user additions preserved
    let instance_skills = starpod_dir.join("skills");
    std::fs::create_dir_all(&instance_skills).map_err(StarpodError::Io)?;
    if let Some(skills_src) = skills_dir {
        if skills_src.is_dir() {
            sync_skills(skills_src, &instance_skills)?;
            debug!("Synced skills to instance");
        }
    }

    info!(
        blueprint = %blueprint_dir.display(),
        output = %output_dir.display(),
        "Standalone build complete"
    );

    Ok(())
}

/// Merge skill directories from source `skills/` into instance `.starpod/skills/`.
///
/// Each skill is a directory containing `SKILL.md` and optional resource subdirs.
/// Blueprint skills overwrite existing skills with the same name (shipping a new version).
/// User/agent-created skills that don't exist in the source are preserved.
fn sync_skills(source_skills: &Path, instance_skills: &Path) -> crate::Result<()> {
    let entries = std::fs::read_dir(source_skills).map_err(StarpodError::Io)?;
    for entry in entries {
        let entry = entry.map_err(StarpodError::Io)?;
        let src_path = entry.path();
        if !src_path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let dst_path = instance_skills.join(&name);
        // Always overwrite — blueprint skills win by filename
        if dst_path.exists() {
            std::fs::remove_dir_all(&dst_path).map_err(StarpodError::Io)?;
        }
        copy_dir_recursive(&src_path, &dst_path)?;
    }
    Ok(())
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> crate::Result<()> {
    std::fs::create_dir_all(dst).map_err(StarpodError::Io)?;
    let entries = std::fs::read_dir(src).map_err(StarpodError::Io)?;
    for entry in entries {
        let entry = entry.map_err(StarpodError::Io)?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if src_path.is_file() {
            std::fs::copy(&src_path, &dst_path).map_err(StarpodError::Io)?;
        }
    }
    Ok(())
}

/// Recursively copy files from `src` to `dst`, skipping `.starpod/` in dst.
fn sync_files(src: &Path, dst: &Path) -> crate::Result<()> {
    let entries = std::fs::read_dir(src).map_err(StarpodError::Io)?;

    for entry in entries {
        let entry = entry.map_err(StarpodError::Io)?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Never write into .starpod/
        if name_str == ".starpod" {
            continue;
        }

        let src_path = entry.path();
        let dst_path = dst.join(&name);

        if src_path.is_dir() {
            std::fs::create_dir_all(&dst_path).map_err(StarpodError::Io)?;
            sync_files(&src_path, &dst_path)?;
        } else if src_path.is_file() {
            // Only copy if destination doesn't exist (preserve agent-created files)
            if !dst_path.exists() {
                std::fs::copy(&src_path, &dst_path).map_err(|e| {
                    StarpodError::Config(format!("Failed to sync {}: {}", src_path.display(), e))
                })?;
            }
        }
    }

    Ok(())
}

/// Create an ephemeral single-agent instance in a temp directory.
///
/// Returns the [`TempDir`](tempfile::TempDir) guard (caller must hold it to
/// keep the directory alive) and [`ResolvedPaths`](crate::ResolvedPaths) for
/// loading config and constructing an agent.
///
/// The instance uses all defaults from [`AgentConfig`](crate::AgentConfig)
/// (model: `anthropic/claude-haiku-4-5`, max_turns: 30, etc.) and resolves
/// as [`Mode::SingleAgent`](crate::Mode::SingleAgent).
///
/// Dropping the returned `TempDir` deletes the entire instance (databases,
/// sessions, and all). This is intentional for one-off commands like
/// `starpod chat` where no state needs to persist.
pub fn create_ephemeral_instance() -> crate::Result<(tempfile::TempDir, crate::ResolvedPaths)> {
    let tmp = tempfile::TempDir::new().map_err(StarpodError::Io)?;
    let starpod_dir = tmp.path().join(".starpod");
    let config_dir = starpod_dir.join("config");
    std::fs::create_dir_all(&config_dir).map_err(StarpodError::Io)?;
    std::fs::create_dir_all(starpod_dir.join("db")).map_err(StarpodError::Io)?;
    std::fs::create_dir_all(starpod_dir.join("users")).map_err(StarpodError::Io)?;
    std::fs::create_dir_all(starpod_dir.join("skills")).map_err(StarpodError::Io)?;

    // Minimal agent.toml — all fields default via serde
    std::fs::write(config_dir.join("agent.toml"), "agent_name = \"Starpod\"\n")
        .map_err(StarpodError::Io)?;

    // Empty SOUL.md
    std::fs::write(config_dir.join("SOUL.md"), "").map_err(StarpodError::Io)?;

    let mode = crate::Mode::SingleAgent {
        starpod_dir: starpod_dir.clone(),
    };
    let paths = crate::ResolvedPaths::resolve(&mode)?;
    Ok((tmp, paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_blueprint(tmp: &TempDir) -> std::path::PathBuf {
        let blueprint = tmp.path().join("agents").join("test-bot");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(
            blueprint.join("agent.toml"),
            "agent_name = \"TestBot\"\nmodel = \"claude-haiku-4-5\"\n",
        )
        .unwrap();
        std::fs::write(blueprint.join("SOUL.md"), "# Soul\n\nYou are TestBot.\n").unwrap();

        // .env lives at workspace root (tmp root), not in the blueprint
        std::fs::write(tmp.path().join(".env"), "PROD_KEY=secret\n").unwrap();

        // Template files
        let files = blueprint.join("files");
        std::fs::create_dir_all(files.join("templates")).unwrap();
        std::fs::write(files.join("templates").join("report.md"), "# Report\n").unwrap();

        blueprint
    }

    #[test]
    fn apply_blueprint_creates_structure() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let sp = instance.join(".starpod");
        let cfg = sp.join("config");
        assert!(cfg.join("agent.toml").is_file());
        assert!(cfg.join("SOUL.md").is_file());
        assert!(
            !sp.join(".env").exists(),
            ".env should not be copied — vault handles secrets"
        );
        assert!(sp.join("db").is_dir());
        assert!(cfg.join("HEARTBEAT.md").exists());
        assert!(cfg.join("BOOT.md").exists());
        assert!(cfg.join("BOOTSTRAP.md").exists());
    }

    #[test]
    fn apply_blueprint_dev_env_not_copied() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // .env is NOT copied — vault handles secrets at build time
        assert!(!instance.join(".starpod").join(".env").exists());
    }

    #[test]
    fn apply_blueprint_prod_env_not_copied() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Prod).unwrap();

        assert!(!instance.join(".starpod").join(".env").exists());
    }

    #[test]
    fn apply_blueprint_syncs_template_files() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        assert!(instance
            .join("home")
            .join("templates")
            .join("report.md")
            .is_file());
    }

    #[test]
    fn apply_blueprint_creates_home_dirs() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let home = instance.join("home");
        assert!(home.join("desktop").is_dir());
        assert!(home.join("documents").is_dir());
        assert!(home.join("projects").is_dir());
        assert!(home.join("downloads").is_dir());
    }

    #[test]
    fn apply_blueprint_preserves_home_files_on_reapply() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Simulate agent creating files in home/
        let home = instance.join("home");
        std::fs::write(home.join("documents").join("notes.md"), "My notes").unwrap();
        std::fs::create_dir_all(home.join("projects").join("my-app")).unwrap();
        std::fs::write(
            home.join("projects").join("my-app").join("main.rs"),
            "fn main() {}",
        )
        .unwrap();

        // Re-apply blueprint
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Agent-created files should be preserved
        assert_eq!(
            std::fs::read_to_string(home.join("documents").join("notes.md")).unwrap(),
            "My notes"
        );
        assert_eq!(
            std::fs::read_to_string(home.join("projects").join("my-app").join("main.rs")).unwrap(),
            "fn main() {}"
        );
    }

    #[test]
    fn apply_blueprint_refreshes_agent_toml() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        // First apply
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Update blueprint
        std::fs::write(
            blueprint.join("agent.toml"),
            "agent_name = \"UpdatedBot\"\nmodel = \"gpt-4o\"\n",
        )
        .unwrap();

        // Re-apply — agent.toml should be refreshed in config/
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let content =
            std::fs::read_to_string(instance.join(".starpod").join("config").join("agent.toml"))
                .unwrap();
        assert!(content.contains("UpdatedBot"));
    }

    #[test]
    fn apply_blueprint_dev_no_env_copied() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("agents").join("no-dev-env");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();
        std::fs::write(tmp.path().join(".env"), "ONLY_PROD=yes\n").unwrap();

        let instance = tmp.path().join(".instances").join("no-dev-env");
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // .env not copied — vault reads from workspace .env directly at build time
        assert!(!instance.join(".starpod").join(".env").exists());
    }

    #[test]
    fn apply_blueprint_no_env_files() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("agents").join("bare");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();
        // No .env at workspace root

        let instance = tmp.path().join(".instances").join("bare");
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Instance should be created with config/agent.toml but no .env file
        assert!(instance
            .join(".starpod")
            .join("config")
            .join("agent.toml")
            .is_file());
        assert!(!instance.join(".starpod").join(".env").exists());
    }

    #[test]
    fn sync_files_skips_starpod_dir() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src_files");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();

        // Create a .starpod dir in src — should be skipped
        std::fs::create_dir_all(src.join(".starpod")).unwrap();
        std::fs::write(src.join(".starpod").join("bad.toml"), "evil").unwrap();
        std::fs::write(src.join("good.txt"), "hello").unwrap();

        sync_files(&src, &dst).unwrap();

        assert!(dst.join("good.txt").is_file());
        assert!(!dst.join(".starpod").exists());
    }

    #[test]
    fn apply_blueprint_copies_workspace_skills() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        // Create workspace skills
        let ws_skills = tmp.path().join("skills");
        std::fs::create_dir_all(ws_skills.join("code-review")).unwrap();
        std::fs::write(
            ws_skills.join("code-review").join("SKILL.md"),
            "---\nname: code-review\ndescription: Review code.\n---\n\nCheck for bugs.",
        )
        .unwrap();
        std::fs::create_dir_all(ws_skills.join("summarize")).unwrap();
        std::fs::write(
            ws_skills.join("summarize").join("SKILL.md"),
            "---\nname: summarize\ndescription: Summarize text.\n---\n\nBe concise.",
        )
        .unwrap();

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let inst_skills = instance.join(".starpod").join("skills");
        assert!(inst_skills.is_dir());
        assert!(inst_skills.join("code-review").join("SKILL.md").is_file());
        assert!(inst_skills.join("summarize").join("SKILL.md").is_file());

        // Verify content was copied correctly
        let content =
            std::fs::read_to_string(inst_skills.join("code-review").join("SKILL.md")).unwrap();
        assert!(content.contains("Check for bugs"));
    }

    #[test]
    fn apply_blueprint_overwrites_blueprint_skills_preserves_agent_skills() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        // First apply with workspace skill
        let ws_skills = tmp.path().join("skills");
        std::fs::create_dir_all(ws_skills.join("ws-skill")).unwrap();
        std::fs::write(
            ws_skills.join("ws-skill").join("SKILL.md"),
            "---\nname: ws-skill\ndescription: From workspace.\n---\n\nOriginal.",
        )
        .unwrap();

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Agent creates a skill at runtime
        let inst_skills = instance.join(".starpod").join("skills");
        std::fs::create_dir_all(inst_skills.join("agent-skill")).unwrap();
        std::fs::write(
            inst_skills.join("agent-skill").join("SKILL.md"),
            "---\nname: agent-skill\ndescription: Created by agent.\n---\n\nRuntime skill.",
        )
        .unwrap();

        // Update the workspace skill (new version)
        std::fs::write(
            ws_skills.join("ws-skill").join("SKILL.md"),
            "---\nname: ws-skill\ndescription: Updated.\n---\n\nModified.",
        )
        .unwrap();

        // Re-apply blueprint
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Agent-created skill should be preserved (not in blueprint)
        assert!(inst_skills.join("agent-skill").join("SKILL.md").is_file());

        // Blueprint skill should be overwritten with new version
        let content =
            std::fs::read_to_string(inst_skills.join("ws-skill").join("SKILL.md")).unwrap();
        assert!(
            content.contains("Modified"),
            "Blueprint skill should be overwritten on re-apply"
        );
    }

    #[test]
    fn apply_blueprint_creates_skills_dir_even_without_workspace_skills() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        // No workspace skills/ directory
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Instance skills dir should still be created (empty, ready for agent to create skills)
        assert!(instance.join(".starpod").join("skills").is_dir());
    }

    // ── build_standalone tests ──────────────────────────────────────────

    #[test]
    fn build_standalone_copies_deploy_toml() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(
            blueprint.join("agent.toml"),
            "agent_name = \"TestBot\"\nmodel = \"claude-haiku-4-5\"\n",
        )
        .unwrap();
        std::fs::write(
            blueprint.join("deploy.toml"),
            "version = 1\n\n[agent.secrets.ANTHROPIC_API_KEY]\nrequired = true\ndescription = \"key\"\n",
        ).unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, None, None, false).unwrap();

        let deployed = output.join(".starpod").join("config").join("deploy.toml");
        assert!(
            deployed.is_file(),
            "deploy.toml should be copied to .starpod/config/"
        );
        let content = std::fs::read_to_string(&deployed).unwrap();
        assert!(content.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn build_standalone_creates_structure() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(
            blueprint.join("agent.toml"),
            "agent_name = \"TestBot\"\nmodel = \"claude-haiku-4-5\"\n",
        )
        .unwrap();
        std::fs::write(blueprint.join("SOUL.md"), "# Soul\n\nYou are TestBot.\n").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, None, None, false).unwrap();

        let sp = output.join(".starpod");
        let cfg = sp.join("config");
        assert!(cfg.join("agent.toml").is_file());
        assert!(cfg.join("SOUL.md").is_file());
        assert!(sp.join("db").is_dir());
        assert!(sp.join("skills").is_dir());
        assert!(cfg.join("HEARTBEAT.md").exists());
        assert!(cfg.join("BOOT.md").exists());
        assert!(cfg.join("BOOTSTRAP.md").exists());
    }

    #[test]
    fn build_standalone_fails_without_agent_toml() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("bad-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        // No agent.toml

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        let err = build_standalone(&blueprint, &output, None, None, false).unwrap_err();
        assert!(err.to_string().contains("agent.toml"));
    }

    #[test]
    fn build_standalone_fails_if_starpod_exists_without_force() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        // First build succeeds
        build_standalone(&blueprint, &output, None, None, false).unwrap();

        // Second build without --force should fail
        let err = build_standalone(&blueprint, &output, None, None, false).unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert!(err.to_string().contains("--force"));

        // With --force should succeed
        build_standalone(&blueprint, &output, None, None, true).unwrap();
    }

    #[test]
    fn build_standalone_copies_env_file() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let env_file = tmp.path().join("my.env");
        std::fs::write(&env_file, "SECRET=hunter2\n").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, None, Some(&env_file), false).unwrap();

        // .env is NOT copied — vault handles secrets at build time
        assert!(!output.join(".starpod").join(".env").exists());
    }

    #[test]
    fn build_standalone_with_missing_env_file_succeeds() {
        // env_file param is ignored now — no copy, no error
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(
            &blueprint,
            &output,
            None,
            Some(Path::new("/nonexistent/.env")),
            false,
        )
        .unwrap(); // should not error
    }

    #[test]
    fn build_standalone_copies_skills() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let skills = tmp.path().join("skills");
        std::fs::create_dir_all(skills.join("code-review")).unwrap();
        std::fs::write(
            skills.join("code-review").join("SKILL.md"),
            "---\nname: code-review\n---\nReview code.",
        )
        .unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, Some(&skills), None, false).unwrap();

        // Skills stay at starpod root, not in config/
        assert!(output
            .join(".starpod")
            .join("skills")
            .join("code-review")
            .join("SKILL.md")
            .is_file());
    }

    #[test]
    fn build_standalone_syncs_template_files() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let files = blueprint.join("files");
        std::fs::create_dir_all(files.join("templates")).unwrap();
        std::fs::write(files.join("templates").join("report.md"), "# Report\n").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, None, None, false).unwrap();

        assert!(output
            .join("home")
            .join("templates")
            .join("report.md")
            .is_file());
    }

    #[test]
    fn build_standalone_creates_home_dirs() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, None, None, false).unwrap();

        let home = output.join("home");
        assert!(home.join("desktop").is_dir());
        assert!(home.join("documents").is_dir());
        assert!(home.join("projects").is_dir());
        assert!(home.join("downloads").is_dir());
    }

    #[test]
    fn build_standalone_copies_lifecycle_files_from_blueprint() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();
        std::fs::write(blueprint.join("BOOT.md"), "Run migrations on startup.").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, None, None, false).unwrap();

        let content =
            std::fs::read_to_string(output.join(".starpod").join("config").join("BOOT.md"))
                .unwrap();
        assert!(content.contains("Run migrations on startup"));
    }

    #[test]
    fn build_standalone_no_env_when_not_provided() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, None, None, false).unwrap();

        assert!(!output.join(".starpod").join(".env").exists());
        // But config/ should exist with agent.toml
        assert!(output
            .join(".starpod")
            .join("config")
            .join("agent.toml")
            .is_file());
    }

    // ── Config/runtime separation tests ──────────────────────────

    #[test]
    fn build_standalone_config_and_runtime_separation() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "agent_name = \"Bot\"\n").unwrap();
        std::fs::write(blueprint.join("SOUL.md"), "# Soul\nTest.").unwrap();
        std::fs::write(blueprint.join("BOOT.md"), "Boot up.").unwrap();

        let env_file = tmp.path().join("my.env");
        std::fs::write(&env_file, "API_KEY=test\n").unwrap();

        let skills = tmp.path().join("skills");
        std::fs::create_dir_all(skills.join("greet")).unwrap();
        std::fs::write(
            skills.join("greet").join("SKILL.md"),
            "---\nname: greet\n---\nHi.",
        )
        .unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, Some(&skills), Some(&env_file), false).unwrap();

        let sp = output.join(".starpod");
        let cfg = sp.join("config");

        // Blueprint-managed files → config/
        assert!(cfg.join("agent.toml").is_file());
        assert!(cfg.join("SOUL.md").is_file());
        assert!(cfg.join("BOOT.md").is_file());
        assert!(cfg.join("HEARTBEAT.md").exists());
        assert!(cfg.join("BOOTSTRAP.md").exists());

        // .env not copied (vault handles secrets)
        assert!(!sp.join(".env").exists());
        assert!(!cfg.join(".env").exists());

        // Skills → starpod root (NOT config/)
        assert!(sp.join("skills").join("greet").join("SKILL.md").is_file());
        assert!(!cfg.join("skills").exists());

        // Runtime dirs → starpod root
        assert!(sp.join("db").is_dir());
        assert!(sp.join("users").is_dir());
    }

    #[test]
    fn build_standalone_overwrites_config_preserves_runtime() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "model = \"v1\"\n").unwrap();
        std::fs::write(blueprint.join("SOUL.md"), "# Soul\nVersion 1.").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        // First build
        build_standalone(&blueprint, &output, None, None, false).unwrap();

        let sp = output.join(".starpod");

        // Simulate runtime data
        std::fs::write(sp.join(".env"), "RUNTIME_SECRET=val\n").unwrap();

        // Update blueprint
        std::fs::write(blueprint.join("agent.toml"), "model = \"v2\"\n").unwrap();
        std::fs::write(blueprint.join("SOUL.md"), "# Soul\nVersion 2.").unwrap();

        // Second build with --force
        build_standalone(&blueprint, &output, None, None, true).unwrap();

        // Config should be updated
        let toml = std::fs::read_to_string(sp.join("config").join("agent.toml")).unwrap();
        assert!(toml.contains("v2"), "agent.toml should be refreshed to v2");
        let soul = std::fs::read_to_string(sp.join("config").join("SOUL.md")).unwrap();
        assert!(soul.contains("Version 2"), "SOUL.md should be refreshed");

        // Runtime should be preserved
        let env = std::fs::read_to_string(sp.join(".env")).unwrap();
        assert!(env.contains("RUNTIME_SECRET"), ".env should be preserved");
    }

    #[test]
    fn build_standalone_skills_merge_preserves_user_skills() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let skills = tmp.path().join("skills");
        std::fs::create_dir_all(skills.join("bp-skill")).unwrap();
        std::fs::write(
            skills.join("bp-skill").join("SKILL.md"),
            "---\nname: bp-skill\n---\nBlueprint v1.",
        )
        .unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        // First build
        build_standalone(&blueprint, &output, Some(&skills), None, false).unwrap();

        // Agent creates a skill at runtime
        let inst_skills = output.join(".starpod").join("skills");
        std::fs::create_dir_all(inst_skills.join("user-skill")).unwrap();
        std::fs::write(
            inst_skills.join("user-skill").join("SKILL.md"),
            "---\nname: user-skill\n---\nCreated by agent.",
        )
        .unwrap();

        // Update blueprint skill
        std::fs::write(
            skills.join("bp-skill").join("SKILL.md"),
            "---\nname: bp-skill\n---\nBlueprint v2.",
        )
        .unwrap();

        // Second build with --force
        build_standalone(&blueprint, &output, Some(&skills), None, true).unwrap();

        // Blueprint skill should be updated
        let bp = std::fs::read_to_string(inst_skills.join("bp-skill").join("SKILL.md")).unwrap();
        assert!(
            bp.contains("Blueprint v2"),
            "Blueprint skill should be overwritten"
        );

        // User skill should be preserved
        assert!(inst_skills.join("user-skill").join("SKILL.md").is_file());
        let us = std::fs::read_to_string(inst_skills.join("user-skill").join("SKILL.md")).unwrap();
        assert!(
            us.contains("Created by agent"),
            "User-created skill should be preserved"
        );
    }

    #[test]
    fn apply_blueprint_env_at_root_not_config() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let sp = instance.join(".starpod");
        // .env should NOT be in the instance (vault handles secrets)
        assert!(!sp.join(".env").exists());
        assert!(!sp.join("config").join(".env").exists());
    }

    // ── create_ephemeral_instance tests ─────────────────────────────

    #[test]
    fn ephemeral_instance_creates_valid_structure() {
        let (tmp, paths) = create_ephemeral_instance().unwrap();
        let sp = tmp.path().join(".starpod");

        // Directory structure
        assert!(sp.join("config").is_dir());
        assert!(sp.join("db").is_dir());
        assert!(sp.join("users").is_dir());
        assert!(sp.join("skills").is_dir());

        // Config files
        assert!(sp.join("config").join("agent.toml").is_file());
        assert!(sp.join("config").join("SOUL.md").is_file());

        // Paths resolve correctly
        assert_eq!(paths.agent_toml, sp.join("config").join("agent.toml"));
        assert_eq!(paths.config_dir, sp.join("config"));
        assert_eq!(paths.db_dir, sp.join("db"));
    }

    #[test]
    fn ephemeral_instance_has_default_agent_name() {
        let (_tmp, paths) = create_ephemeral_instance().unwrap();
        let content = std::fs::read_to_string(&paths.agent_toml).unwrap();
        assert!(content.contains("agent_name = \"Starpod\""));
    }

    #[test]
    fn ephemeral_instance_config_is_loadable() {
        let (_tmp, paths) = create_ephemeral_instance().unwrap();
        // The generated agent.toml must parse into a valid AgentConfig
        let config = crate::load_agent_config(&paths).unwrap();
        assert_eq!(config.agent_name, "Starpod");
    }

    #[test]
    fn ephemeral_instance_is_single_agent_mode() {
        let (_tmp, paths) = create_ephemeral_instance().unwrap();
        assert!(
            matches!(paths.mode, crate::Mode::SingleAgent { .. }),
            "Ephemeral instance should resolve as SingleAgent mode"
        );
    }

    #[test]
    fn ephemeral_instance_cleanup_on_drop() {
        let dir_path;
        {
            let (tmp, _paths) = create_ephemeral_instance().unwrap();
            dir_path = tmp.path().to_path_buf();
            assert!(
                dir_path.exists(),
                "Temp dir should exist while guard is held"
            );
        }
        // TempDir dropped — directory should be cleaned up
        assert!(
            !dir_path.exists(),
            "Temp dir should be removed after guard drops"
        );
    }

    #[test]
    fn ephemeral_instance_unique_per_call() {
        let (tmp1, _) = create_ephemeral_instance().unwrap();
        let (tmp2, _) = create_ephemeral_instance().unwrap();
        assert_ne!(
            tmp1.path(),
            tmp2.path(),
            "Each ephemeral instance should have a unique temp directory"
        );
    }
}
