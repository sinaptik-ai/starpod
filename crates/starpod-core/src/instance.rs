//! Blueprint application — copy agent blueprints into runtime instances.
//!
//! An agent **blueprint** lives in `agents/<name>/` (git-tracked) and contains
//! configuration, personality files, and template filesystem content.
//!
//! A runtime **instance** lives in `.instances/<name>/` (gitignored) and holds
//! the actual agent state: databases, memory, user data, and any files the
//! agent creates at runtime.
//!
//! [`apply_blueprint`] copies the blueprint into the instance, creating the
//! internal `.starpod/` directory structure:
//! - `config/` — blueprint-managed files (overwritten on every build)
//! - `skills/` — merged: blueprint skills overwrite by filename, user additions preserved
//! - `db/`, `users/` — runtime data (never touched by build)
//! - `.env` — environment secrets at starpod root (not in config/)

use std::path::Path;

use tracing::{debug, info};

use crate::error::StarpodError;

/// Default USER.md content seeded for new users.
const DEFAULT_USER: &str = "\
# User Profile

<!-- The agent reads this file at the start of every conversation to personalize responses. -->
<!-- Fill in what's relevant — leave sections blank or remove them if not needed. -->

## Name
<!-- Your name or how you'd like to be addressed. -->

## Role
<!-- e.g. software engineer, student, researcher, founder -->

## Expertise
<!-- What you're good at — helps the agent calibrate explanations. -->
<!-- e.g. \"senior Rust developer\", \"new to programming\", \"data scientist\" -->

## Preferences
<!-- Communication style, formatting, language, or workflow preferences. -->
<!-- e.g. \"be concise\", \"prefer code examples over explanations\", \"reply in Italian\" -->

## Context
<!-- Anything else the agent should know: current projects, goals, constraints. -->
";

/// Default user IDs seeded at build time.
const DEFAULT_USERS: &[&str] = &["admin", "user"];

/// Create a user directory with default USER.md and MEMORY.md if it doesn't exist.
fn ensure_user_dir(users_dir: &Path, user_id: &str) -> crate::Result<()> {
    let user_dir = users_dir.join(user_id);
    if !user_dir.exists() {
        std::fs::create_dir_all(&user_dir)
            .map_err(StarpodError::Io)?;
        std::fs::create_dir_all(user_dir.join("memory"))
            .map_err(StarpodError::Io)?;

        if !user_dir.join("USER.md").exists() {
            std::fs::write(
                user_dir.join("USER.md"),
                DEFAULT_USER,
            ).map_err(StarpodError::Io)?;
        }
        if !user_dir.join("MEMORY.md").exists() {
            std::fs::write(
                user_dir.join("MEMORY.md"),
                "# Memory Index\n\nImportant facts and links to memory files.\n",
            ).map_err(StarpodError::Io)?;
        }
        info!("Created default {user_id} user directory");
    }
    Ok(())
}

/// Which `.env` file to copy from the workspace root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvSource {
    /// Use `.env.dev` (development overrides).
    Dev,
    /// Use `.env` (production secrets).
    Prod,
}

/// Apply a blueprint directory to an instance directory.
///
/// # Layout created
///
/// ```text
/// .instances/<name>/           ← instance_dir (agent's filesystem root)
/// ├── .starpod/                ← internal state
/// │   ├── .env                 ← from workspace .env.dev (Dev) or .env (Prod)
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
/// └── ...                      ← agent-created files
/// ```
///
/// # Preservation rules
///
/// - Existing user directories are never overwritten.
/// - Existing databases (db/*.db) are preserved.
/// - Blueprint `files/*` are synced to instance root, but never overwrite `.starpod/`.
/// - `config/` is always refreshed from the blueprint (agent.toml, SOUL.md, lifecycle files).
/// - `skills/` are merged: blueprint skills overwrite by filename, user-created skills preserved.
/// - `.env` is read from `workspace_dir` (top level), not from the blueprint.
pub fn apply_blueprint(
    blueprint_dir: &Path,
    instance_dir: &Path,
    workspace_dir: &Path,
    env_source: EnvSource,
) -> crate::Result<()> {
    let starpod_dir = instance_dir.join(".starpod");
    let config_dir = starpod_dir.join("config");

    // 1. Create directory structure
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| StarpodError::Config(format!(
            "Failed to create .starpod/config/ in {}: {}", instance_dir.display(), e
        )))?;
    std::fs::create_dir_all(starpod_dir.join("db"))
        .map_err(StarpodError::Io)?;
    std::fs::create_dir_all(starpod_dir.join("users"))
        .map_err(StarpodError::Io)?;

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
            .map_err(|e| StarpodError::Config(format!(
                "Failed to copy agent.toml: {}", e
            )))?;
        debug!("Copied agent.toml from blueprint");
    }

    // SOUL.md (always refresh)
    let src_soul = blueprint_dir.join("SOUL.md");
    if src_soul.is_file() {
        std::fs::copy(&src_soul, config_dir.join("SOUL.md"))
            .map_err(|e| StarpodError::Config(format!(
                "Failed to copy SOUL.md: {}", e
            )))?;
        debug!("Copied SOUL.md from blueprint");
    }

    // 3. Copy .env file from workspace root to starpod root (not config/)
    let env_src = match env_source {
        EnvSource::Dev => {
            let dev = workspace_dir.join(".env.dev");
            if dev.is_file() {
                Some(dev)
            } else {
                // Fall back to .env if .env.dev doesn't exist
                let prod = workspace_dir.join(".env");
                if prod.is_file() { Some(prod) } else { None }
            }
        }
        EnvSource::Prod => {
            let prod = workspace_dir.join(".env");
            if prod.is_file() { Some(prod) } else { None }
        }
    };
    if let Some(src) = env_src {
        std::fs::copy(&src, starpod_dir.join(".env"))
            .map_err(|e| StarpodError::Config(format!(
                "Failed to copy .env: {}", e
            )))?;
        debug!(source = %src.display(), "Copied .env from workspace");
    }

    // 4. Sync files/ → instance root (excluding .starpod/)
    let files_dir = blueprint_dir.join("files");
    if files_dir.is_dir() {
        sync_files(&files_dir, instance_dir)?;
        debug!("Synced template files from blueprint");
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

    // 6. Create default users if not exist
    let users_dir = starpod_dir.join("users");
    for user_id in DEFAULT_USERS {
        ensure_user_dir(&users_dir, user_id)?;
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
/// - `env_file`: optional path to `.env` file to include
pub fn build_standalone(
    blueprint_dir: &Path,
    output_dir: &Path,
    skills_dir: Option<&Path>,
    env_file: Option<&Path>,
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
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| StarpodError::Config(format!(
            "Failed to create .starpod/config/ in {}: {}", output_dir.display(), e
        )))?;
    std::fs::create_dir_all(starpod_dir.join("db"))
        .map_err(StarpodError::Io)?;
    std::fs::create_dir_all(starpod_dir.join("users"))
        .map_err(StarpodError::Io)?;

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
        .map_err(|e| StarpodError::Config(format!(
            "Failed to copy agent.toml: {}", e
        )))?;
    debug!("Copied agent.toml from blueprint");

    // SOUL.md (always refresh)
    let src_soul = blueprint_dir.join("SOUL.md");
    if src_soul.is_file() {
        std::fs::copy(&src_soul, config_dir.join("SOUL.md"))
            .map_err(|e| StarpodError::Config(format!(
                "Failed to copy SOUL.md: {}", e
            )))?;
        debug!("Copied SOUL.md from blueprint");
    }

    // 3. Copy .env file to starpod root (not config/) — only if not already present
    if let Some(env_src) = env_file {
        if env_src.is_file() {
            std::fs::copy(env_src, starpod_dir.join(".env"))
                .map_err(|e| StarpodError::Config(format!(
                    "Failed to copy .env from {}: {}", env_src.display(), e
                )))?;
            debug!(source = %env_src.display(), "Copied .env");
        } else {
            return Err(StarpodError::Config(format!(
                "Env file not found: {}", env_src.display()
            )));
        }
    }

    // 4. Sync files/ → output root (excluding .starpod/)
    let files_dir = blueprint_dir.join("files");
    if files_dir.is_dir() {
        sync_files(&files_dir, output_dir)?;
        debug!("Synced template files from blueprint");
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

    // 6. Create default users if not exist
    let users_dir = starpod_dir.join("users");
    for user_id in DEFAULT_USERS {
        ensure_user_dir(&users_dir, user_id)?;
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
    let entries = std::fs::read_dir(src)
        .map_err(StarpodError::Io)?;

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
                    StarpodError::Config(format!(
                        "Failed to sync {}: {}", src_path.display(), e
                    ))
                })?;
            }
        }
    }

    Ok(())
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
        ).unwrap();
        std::fs::write(
            blueprint.join("SOUL.md"),
            "# Soul\n\nYou are TestBot.\n",
        ).unwrap();

        // .env files live at workspace root (tmp root), not in the blueprint
        std::fs::write(tmp.path().join(".env"), "PROD_KEY=secret\n").unwrap();
        std::fs::write(tmp.path().join(".env.dev"), "DEV_KEY=dev_secret\n").unwrap();

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
        assert!(sp.join(".env").is_file());
        assert!(sp.join("db").is_dir());
        assert!(cfg.join("HEARTBEAT.md").exists());
        assert!(cfg.join("BOOT.md").exists());
        assert!(cfg.join("BOOTSTRAP.md").exists());
        assert!(sp.join("users").is_dir());
        for uid in &["admin", "user"] {
            assert!(sp.join("users").join(uid).is_dir());
            assert!(sp.join("users").join(uid).join("USER.md").is_file());
            assert!(sp.join("users").join(uid).join("MEMORY.md").is_file());
            assert!(sp.join("users").join(uid).join("memory").is_dir());
        }
    }

    #[test]
    fn apply_blueprint_dev_env() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // .env stays at starpod root, not in config/
        let env_content = std::fs::read_to_string(instance.join(".starpod").join(".env")).unwrap();
        assert!(env_content.contains("DEV_KEY=dev_secret"));
    }

    #[test]
    fn apply_blueprint_prod_env() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Prod).unwrap();

        let env_content = std::fs::read_to_string(instance.join(".starpod").join(".env")).unwrap();
        assert!(env_content.contains("PROD_KEY=secret"));
    }

    #[test]
    fn apply_blueprint_syncs_template_files() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        assert!(instance.join("templates").join("report.md").is_file());
    }

    #[test]
    fn apply_blueprint_preserves_existing_user_data() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        // First apply
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Modify admin USER.md
        let admin_user = instance.join(".starpod").join("users").join("admin").join("USER.md");
        std::fs::write(&admin_user, "# User\nCustom content\n").unwrap();

        // Second apply — should NOT overwrite
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let content = std::fs::read_to_string(&admin_user).unwrap();
        assert!(content.contains("Custom content"), "User data should be preserved");
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
        ).unwrap();

        // Re-apply — agent.toml should be refreshed in config/
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let content = std::fs::read_to_string(
            instance.join(".starpod").join("config").join("agent.toml")
        ).unwrap();
        assert!(content.contains("UpdatedBot"));
    }

    #[test]
    fn apply_blueprint_dev_falls_back_to_prod_env() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("agents").join("no-dev-env");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();
        // .env at workspace root, no .env.dev
        std::fs::write(tmp.path().join(".env"), "ONLY_PROD=yes\n").unwrap();

        let instance = tmp.path().join(".instances").join("no-dev-env");
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let env = std::fs::read_to_string(instance.join(".starpod").join(".env")).unwrap();
        assert!(env.contains("ONLY_PROD=yes"), "Dev should fall back to prod .env");
    }

    #[test]
    fn apply_blueprint_no_env_files() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("agents").join("bare");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();
        // No .env or .env.dev at workspace root

        let instance = tmp.path().join(".instances").join("bare");
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Instance should be created with config/agent.toml but no .env file
        assert!(instance.join(".starpod").join("config").join("agent.toml").is_file());
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
        ).unwrap();
        std::fs::create_dir_all(ws_skills.join("summarize")).unwrap();
        std::fs::write(
            ws_skills.join("summarize").join("SKILL.md"),
            "---\nname: summarize\ndescription: Summarize text.\n---\n\nBe concise.",
        ).unwrap();

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let inst_skills = instance.join(".starpod").join("skills");
        assert!(inst_skills.is_dir());
        assert!(inst_skills.join("code-review").join("SKILL.md").is_file());
        assert!(inst_skills.join("summarize").join("SKILL.md").is_file());

        // Verify content was copied correctly
        let content = std::fs::read_to_string(
            inst_skills.join("code-review").join("SKILL.md")
        ).unwrap();
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
        ).unwrap();

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Agent creates a skill at runtime
        let inst_skills = instance.join(".starpod").join("skills");
        std::fs::create_dir_all(inst_skills.join("agent-skill")).unwrap();
        std::fs::write(
            inst_skills.join("agent-skill").join("SKILL.md"),
            "---\nname: agent-skill\ndescription: Created by agent.\n---\n\nRuntime skill.",
        ).unwrap();

        // Update the workspace skill (new version)
        std::fs::write(
            ws_skills.join("ws-skill").join("SKILL.md"),
            "---\nname: ws-skill\ndescription: Updated.\n---\n\nModified.",
        ).unwrap();

        // Re-apply blueprint
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        // Agent-created skill should be preserved (not in blueprint)
        assert!(inst_skills.join("agent-skill").join("SKILL.md").is_file());

        // Blueprint skill should be overwritten with new version
        let content = std::fs::read_to_string(
            inst_skills.join("ws-skill").join("SKILL.md")
        ).unwrap();
        assert!(content.contains("Modified"), "Blueprint skill should be overwritten on re-apply");
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
    fn build_standalone_creates_structure() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(
            blueprint.join("agent.toml"),
            "agent_name = \"TestBot\"\nmodel = \"claude-haiku-4-5\"\n",
        ).unwrap();
        std::fs::write(
            blueprint.join("SOUL.md"),
            "# Soul\n\nYou are TestBot.\n",
        ).unwrap();

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
        for uid in &["admin", "user"] {
            assert!(sp.join("users").join(uid).is_dir());
            assert!(sp.join("users").join(uid).join("USER.md").is_file());
            assert!(sp.join("users").join(uid).join("MEMORY.md").is_file());
            assert!(sp.join("users").join(uid).join("memory").is_dir());
        }
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

        // .env should be at starpod root, not in config/
        let content = std::fs::read_to_string(output.join(".starpod").join(".env")).unwrap();
        assert!(content.contains("SECRET=hunter2"));
    }

    #[test]
    fn build_standalone_fails_with_missing_env_file() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        let err = build_standalone(
            &blueprint,
            &output,
            None,
            Some(Path::new("/nonexistent/.env")),
            false,
        ).unwrap_err();
        assert!(err.to_string().contains("Env file not found"));
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
        ).unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        build_standalone(&blueprint, &output, Some(&skills), None, false).unwrap();

        // Skills stay at starpod root, not in config/
        assert!(output.join(".starpod").join("skills").join("code-review").join("SKILL.md").is_file());
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

        assert!(output.join("templates").join("report.md").is_file());
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

        let content = std::fs::read_to_string(output.join(".starpod").join("config").join("BOOT.md")).unwrap();
        assert!(content.contains("Run migrations on startup"));
    }

    #[test]
    fn build_standalone_preserves_existing_user_data() {
        let tmp = TempDir::new().unwrap();
        let blueprint = tmp.path().join("my-agent");
        std::fs::create_dir_all(&blueprint).unwrap();
        std::fs::write(blueprint.join("agent.toml"), "").unwrap();

        let output = tmp.path().join("deploy");
        std::fs::create_dir_all(&output).unwrap();

        // First build
        build_standalone(&blueprint, &output, None, None, false).unwrap();

        // Modify admin USER.md
        let admin_user = output.join(".starpod").join("users").join("admin").join("USER.md");
        std::fs::write(&admin_user, "# Custom user data\n").unwrap();

        // Second build with --force — should NOT overwrite user data
        build_standalone(&blueprint, &output, None, None, true).unwrap();

        let content = std::fs::read_to_string(&admin_user).unwrap();
        assert!(content.contains("Custom user data"), "User data should be preserved on re-build");
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
        assert!(output.join(".starpod").join("config").join("agent.toml").is_file());
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
        std::fs::write(skills.join("greet").join("SKILL.md"), "---\nname: greet\n---\nHi.").unwrap();

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

        // .env → starpod root (NOT config/)
        assert!(sp.join(".env").is_file());
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
        let admin_user = sp.join("users").join("admin").join("USER.md");
        std::fs::write(&admin_user, "# Custom user\n").unwrap();

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
        let user = std::fs::read_to_string(&admin_user).unwrap();
        assert!(user.contains("Custom user"), "User data should be preserved");
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
        ).unwrap();

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
        ).unwrap();

        // Update blueprint skill
        std::fs::write(
            skills.join("bp-skill").join("SKILL.md"),
            "---\nname: bp-skill\n---\nBlueprint v2.",
        ).unwrap();

        // Second build with --force
        build_standalone(&blueprint, &output, Some(&skills), None, true).unwrap();

        // Blueprint skill should be updated
        let bp = std::fs::read_to_string(inst_skills.join("bp-skill").join("SKILL.md")).unwrap();
        assert!(bp.contains("Blueprint v2"), "Blueprint skill should be overwritten");

        // User skill should be preserved
        assert!(inst_skills.join("user-skill").join("SKILL.md").is_file());
        let us = std::fs::read_to_string(inst_skills.join("user-skill").join("SKILL.md")).unwrap();
        assert!(us.contains("Created by agent"), "User-created skill should be preserved");
    }

    #[test]
    fn apply_blueprint_env_at_root_not_config() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let sp = instance.join(".starpod");
        // .env at starpod root
        assert!(sp.join(".env").is_file());
        // NOT in config/
        assert!(!sp.join("config").join(".env").exists());
    }
}
