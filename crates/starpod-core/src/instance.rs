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
//! internal `.starpod/` directory structure while preserving existing runtime
//! data (databases, memory files, user directories).

use std::path::Path;

use tracing::{debug, info};

use crate::error::StarpodError;

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
/// │   ├── agent.toml           ← copied from blueprint
/// │   ├── SOUL.md              ← copied from blueprint
/// │   ├── .env                 ← from workspace .env.dev (Dev) or .env (Prod)
/// │   ├── HEARTBEAT.md         ← agent-level heartbeat (empty default)
/// │   ├── BOOT.md              ← startup instructions (empty default)
/// │   ├── BOOTSTRAP.md         ← one-time init (empty default)
/// │   ├── db/                  ← SQLite databases
/// │   └── users/
/// │       └── admin/           ← auto-created default user
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
/// - `agent.toml` and `SOUL.md` are always refreshed from the blueprint.
/// - `.env` / `.env.dev` are read from `workspace_dir` (top level), not from the blueprint.
pub fn apply_blueprint(
    blueprint_dir: &Path,
    instance_dir: &Path,
    workspace_dir: &Path,
    env_source: EnvSource,
) -> crate::Result<()> {
    let starpod_dir = instance_dir.join(".starpod");

    // 1. Create directory structure
    std::fs::create_dir_all(&starpod_dir)
        .map_err(|e| StarpodError::Config(format!(
            "Failed to create .starpod/ in {}: {}", instance_dir.display(), e
        )))?;
    std::fs::create_dir_all(starpod_dir.join("db"))
        .map_err(StarpodError::Io)?;
    std::fs::create_dir_all(starpod_dir.join("users"))
        .map_err(StarpodError::Io)?;

    // Seed lifecycle files at .starpod/ root (empty defaults, only if not present)
    for name in &["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md"] {
        let path = starpod_dir.join(name);
        if !path.exists() {
            std::fs::write(&path, "").map_err(StarpodError::Io)?;
        }
    }

    // 2. Copy agent.toml (always refresh)
    let src_toml = blueprint_dir.join("agent.toml");
    if src_toml.is_file() {
        std::fs::copy(&src_toml, starpod_dir.join("agent.toml"))
            .map_err(|e| StarpodError::Config(format!(
                "Failed to copy agent.toml: {}", e
            )))?;
        debug!("Copied agent.toml from blueprint");
    }

    // 3. Copy SOUL.md (always refresh)
    let src_soul = blueprint_dir.join("SOUL.md");
    if src_soul.is_file() {
        std::fs::copy(&src_soul, starpod_dir.join("SOUL.md"))
            .map_err(|e| StarpodError::Config(format!(
                "Failed to copy SOUL.md: {}", e
            )))?;
        debug!("Copied SOUL.md from blueprint");
    }

    // 4. Copy .env file from workspace root based on env_source
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

    // 5. Sync files/ → instance root (excluding .starpod/)
    let files_dir = blueprint_dir.join("files");
    if files_dir.is_dir() {
        sync_files(&files_dir, instance_dir)?;
        debug!("Synced template files from blueprint");
    }

    // 6. Create default admin user if not exists
    let admin_dir = starpod_dir.join("users").join("admin");
    if !admin_dir.exists() {
        std::fs::create_dir_all(&admin_dir)
            .map_err(StarpodError::Io)?;
        std::fs::create_dir_all(admin_dir.join("memory"))
            .map_err(StarpodError::Io)?;

        // Seed USER.md and MEMORY.md with defaults
        if !admin_dir.join("USER.md").exists() {
            std::fs::write(
                admin_dir.join("USER.md"),
                "# User Profile\n\nTell me about yourself and I'll remember.\n",
            ).map_err(StarpodError::Io)?;
        }
        if !admin_dir.join("MEMORY.md").exists() {
            std::fs::write(
                admin_dir.join("MEMORY.md"),
                "# Memory Index\n\nImportant facts and links to memory files.\n",
            ).map_err(StarpodError::Io)?;
        }
        info!("Created default admin user directory");
    }

    info!(
        blueprint = %blueprint_dir.display(),
        instance = %instance_dir.display(),
        "Blueprint applied to instance"
    );

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
            "agent_name = \"TestBot\"\nmodel = \"claude-sonnet-4-6\"\n",
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
        assert!(sp.join("agent.toml").is_file());
        assert!(sp.join("SOUL.md").is_file());
        assert!(sp.join(".env").is_file());
        assert!(sp.join("db").is_dir());
        assert!(sp.join("HEARTBEAT.md").exists());
        assert!(sp.join("BOOT.md").exists());
        assert!(sp.join("BOOTSTRAP.md").exists());
        assert!(sp.join("users").is_dir());
        assert!(sp.join("users").join("admin").is_dir());
        assert!(sp.join("users").join("admin").join("USER.md").is_file());
        assert!(sp.join("users").join("admin").join("MEMORY.md").is_file());
        assert!(sp.join("users").join("admin").join("memory").is_dir());
    }

    #[test]
    fn apply_blueprint_dev_env() {
        let tmp = TempDir::new().unwrap();
        let blueprint = setup_blueprint(&tmp);
        let instance = tmp.path().join(".instances").join("test-bot");

        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

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

        // Re-apply — agent.toml should be refreshed
        apply_blueprint(&blueprint, &instance, tmp.path(), EnvSource::Dev).unwrap();

        let content = std::fs::read_to_string(
            instance.join(".starpod").join("agent.toml")
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

        // Instance should be created but no .env file
        assert!(instance.join(".starpod").join("agent.toml").is_file());
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
}
