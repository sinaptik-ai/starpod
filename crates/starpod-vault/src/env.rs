//! Deploy.toml ↔ vault env resolution.
//!
//! At **build time**, [`populate_vault`] reads `.env` + `deploy.toml` declarations
//! and encrypts all secrets and variables into the instance vault. It validates
//! that all required secrets are present.
//!
//! At **serve time**, [`inject_env_from_vault`] decrypts vault entries and injects
//! them into the process environment so the agent and its tools can read them
//! via `std::env::var()`.

use std::collections::HashMap;
use std::path::Path;

use tracing::{debug, warn};

use starpod_core::deploy_manifest::DeployManifest;
use starpod_core::{Result, StarpodError};

use crate::Vault;

// ── .env parser ─────────────────────────────────────────────────────────────

/// Parse a `.env` file into a key-value map.
/// Handles `KEY=VALUE` lines, ignoring comments and empty lines.
/// Strips surrounding double quotes from values.
fn parse_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| StarpodError::Config(format!("Failed to read {}: {}", path.display(), e)))?;

    let mut env = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value)
                .to_string();
            if !key.is_empty() {
                env.insert(key, value);
            }
        }
    }
    Ok(env)
}

// ── Build-time: .env + deploy.toml → vault ──────────────────────────────────

/// Result of populating the vault from .env + deploy.toml.
#[derive(Debug)]
pub struct PopulateResult {
    /// Number of secrets written to vault.
    pub secrets_count: usize,
    /// Number of variables written to vault.
    pub variables_count: usize,
    /// Warnings for missing optional secrets.
    pub warnings: Vec<String>,
}

/// Validate that `.env` has all required secrets declared in `deploy.toml`.
///
/// Dry check — no vault writes. Returns an error if required secrets are missing.
/// Returns warnings for missing optional secrets.
pub fn validate_env(
    deploy_toml_path: &Path,
    env_file: Option<&Path>,
) -> Result<Vec<String>> {
    let manifest = match DeployManifest::load(deploy_toml_path)? {
        Some(m) => m,
        None => return Ok(vec![]),
    };

    let env_map = match env_file {
        Some(path) if path.exists() => parse_env_file(path)?,
        _ => HashMap::new(),
    };

    let mut warnings = Vec::new();
    let mut missing_required = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (key, entry) in &manifest.agent.secrets {
        if seen.insert(key.as_str()) && !env_map.contains_key(key.as_str()) {
            if entry.required {
                missing_required.push(format!("{} — {}", key, entry.description));
            } else {
                warnings.push(format!("{} (optional) — {}", key, entry.description));
            }
        }
    }
    for section in manifest.skills.values() {
        for (key, entry) in &section.secrets {
            if seen.insert(key.as_str()) && !env_map.contains_key(key.as_str()) {
                if entry.required {
                    missing_required.push(format!("{} — {}", key, entry.description));
                } else {
                    warnings.push(format!("{} (optional) — {}", key, entry.description));
                }
            }
        }
    }

    if !missing_required.is_empty() {
        return Err(StarpodError::Config(format!(
            "Missing required secrets in .env:\n  {}",
            missing_required.join("\n  ")
        )));
    }

    Ok(warnings)
}

/// Populate the vault from `.env` values matched against `deploy.toml` declarations.
///
/// For each declared secret: looks up the key in `.env`, encrypts into vault.
/// For each declared variable: uses `.env` value if present, else `deploy.toml` default.
///
/// Returns an error if any **required** secrets are missing from `.env`.
/// Returns warnings for missing optional secrets.
pub async fn populate_vault(
    deploy_toml_path: &Path,
    env_file: Option<&Path>,
    vault: &Vault,
) -> Result<PopulateResult> {
    let manifest = match DeployManifest::load(deploy_toml_path)? {
        Some(m) => m,
        None => {
            debug!("No deploy.toml found, skipping vault population");
            return Ok(PopulateResult {
                secrets_count: 0,
                variables_count: 0,
                warnings: vec![],
            });
        }
    };

    let env_map = match env_file {
        Some(path) if path.exists() => parse_env_file(path)?,
        _ => HashMap::new(),
    };

    let mut warnings = Vec::new();
    let mut missing_required = Vec::new();

    // Collect all secret declarations (agent + skills), deduplicated
    let mut all_secrets: Vec<(&str, &starpod_core::deploy_manifest::SecretEntry)> = Vec::new();
    let mut seen_secrets = std::collections::HashSet::new();
    for (key, entry) in &manifest.agent.secrets {
        if seen_secrets.insert(key.as_str()) {
            all_secrets.push((key.as_str(), entry));
        }
    }
    for (_skill_name, section) in &manifest.skills {
        for (key, entry) in &section.secrets {
            if seen_secrets.insert(key.as_str()) {
                all_secrets.push((key.as_str(), entry));
            }
        }
    }

    // ── Phase 1: Validate — fail fast before writing anything ────
    for (key, entry) in &all_secrets {
        if !env_map.contains_key(*key) {
            if entry.required {
                missing_required.push(format!("{} — {}", key, entry.description));
            } else {
                warnings.push(format!("{} (optional) — {}", key, entry.description));
            }
        }
    }

    if !missing_required.is_empty() {
        return Err(StarpodError::Config(format!(
            "Missing required secrets in .env:\n  {}",
            missing_required.join("\n  ")
        )));
    }

    // ── Phase 2: Write — only after validation passes ────────────
    let mut secrets_count = 0;
    for (key, _entry) in &all_secrets {
        if let Some(value) = env_map.get(*key) {
            vault.set(key, value).await?;
            secrets_count += 1;
        }
    }

    // Collect all variable declarations (agent + skills), deduplicated
    let mut all_variables: Vec<(&str, &starpod_core::deploy_manifest::VariableEntry)> = Vec::new();
    let mut seen_variables = std::collections::HashSet::new();
    for (key, entry) in &manifest.agent.variables {
        if seen_variables.insert(key.as_str()) {
            all_variables.push((key.as_str(), entry));
        }
    }
    for (_skill_name, section) in &manifest.skills {
        for (key, entry) in &section.variables {
            if seen_variables.insert(key.as_str()) {
                all_variables.push((key.as_str(), entry));
            }
        }
    }

    let mut variables_count = 0;
    for (key, entry) in &all_variables {
        // .env value takes precedence over deploy.toml default
        if let Some(value) = env_map.get(*key) {
            vault.set(key, value).await?;
            variables_count += 1;
        } else if let Some(ref default) = entry.default {
            vault.set(key, default).await?;
            variables_count += 1;
        }
        // No .env value and no default → skip silently
    }

    Ok(PopulateResult {
        secrets_count,
        variables_count,
        warnings,
    })
}

// ── Serve-time: vault → process env ─────────────────────────────────────────

/// Inject all declared env vars from the vault into the process environment.
///
/// Reads `deploy.toml` to know which keys to look up, then decrypts each
/// from the vault and calls `std::env::set_var()`.
///
/// Returns the number of variables injected.
pub async fn inject_env_from_vault(
    deploy_toml_path: &Path,
    vault: &Vault,
) -> Result<usize> {
    let manifest = match DeployManifest::load(deploy_toml_path)? {
        Some(m) => m,
        None => {
            debug!("No deploy.toml found, skipping env injection");
            return Ok(0);
        }
    };

    let mut count = 0;

    // Collect all declared keys (secrets + variables, agent + skills)
    let mut all_keys = std::collections::HashSet::new();
    for key in manifest.agent.secrets.keys() {
        all_keys.insert(key.as_str());
    }
    for key in manifest.agent.variables.keys() {
        all_keys.insert(key.as_str());
    }
    for section in manifest.skills.values() {
        for key in section.secrets.keys() {
            all_keys.insert(key.as_str());
        }
        for key in section.variables.keys() {
            all_keys.insert(key.as_str());
        }
    }

    for key in all_keys {
        match vault.get(key).await? {
            Some(value) => {
                // SAFETY: set_var is unsafe in Rust 2024 edition but we're
                // calling it before any multithreaded work starts.
                #[allow(unused_unsafe)]
                unsafe {
                    std::env::set_var(key, &value);
                }
                count += 1;
            }
            None => {
                warn!(key = %key, "Declared env var not found in vault — was build run?");
            }
        }
    }

    debug!(count = count, "Injected env vars from vault");
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn test_vault(tmp: &TempDir) -> Vault {
        let key = [0xAB; 32];
        Vault::new(&tmp.path().join("vault.db"), &key).await.unwrap()
    }

    fn write_env(dir: &Path, content: &str) -> std::path::PathBuf {
        let path = dir.join(".env");
        std::fs::write(&path, content).unwrap();
        path
    }

    fn write_deploy_toml(dir: &Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("deploy.toml");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[tokio::test]
    async fn test_populate_no_deploy_toml() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;
        let result = populate_vault(
            &tmp.path().join("nonexistent.toml"),
            None,
            &vault,
        ).await.unwrap();
        assert_eq!(result.secrets_count, 0);
        assert_eq!(result.variables_count, 0);
    }

    #[tokio::test]
    async fn test_populate_secrets_from_env() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        let env_path = write_env(tmp.path(), "ANTHROPIC_API_KEY=sk-ant-xxx\nGITHUB_TOKEN=ghp_yyy\n");
        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[agent.secrets.ANTHROPIC_API_KEY]
secret = "ANTHROPIC_API_KEY"
required = true
description = "Anthropic key"

[skills.my-skill.secrets.GITHUB_TOKEN]
secret = "GITHUB_TOKEN"
required = true
description = "GitHub PAT"
"#);

        let result = populate_vault(&deploy_path, Some(&env_path), &vault).await.unwrap();
        assert_eq!(result.secrets_count, 2);
        assert_eq!(vault.get("ANTHROPIC_API_KEY").await.unwrap().as_deref(), Some("sk-ant-xxx"));
        assert_eq!(vault.get("GITHUB_TOKEN").await.unwrap().as_deref(), Some("ghp_yyy"));
    }

    #[tokio::test]
    async fn test_populate_missing_required_secret_fails() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        let env_path = write_env(tmp.path(), ""); // empty .env
        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[agent.secrets.ANTHROPIC_API_KEY]
secret = "ANTHROPIC_API_KEY"
required = true
description = "Anthropic key"
"#);

        let result = populate_vault(&deploy_path, Some(&env_path), &vault).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("ANTHROPIC_API_KEY"));
    }

    #[tokio::test]
    async fn test_populate_missing_optional_secret_warns() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        let env_path = write_env(tmp.path(), "ANTHROPIC_API_KEY=sk-xxx\n");
        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[agent.secrets.ANTHROPIC_API_KEY]
secret = "ANTHROPIC_API_KEY"
required = true
description = "Anthropic key"

[skills.my-skill.secrets.OPTIONAL_KEY]
secret = "OPTIONAL_KEY"
required = false
description = "Not critical"
"#);

        let result = populate_vault(&deploy_path, Some(&env_path), &vault).await.unwrap();
        assert_eq!(result.secrets_count, 1);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("OPTIONAL_KEY"));
    }

    #[tokio::test]
    async fn test_populate_variables_from_env_overrides_default() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        let env_path = write_env(tmp.path(), "CITY=Milan\n");
        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[skills.weather.variables.CITY]
default = "Rome"
description = "Default city"
"#);

        let result = populate_vault(&deploy_path, Some(&env_path), &vault).await.unwrap();
        assert_eq!(result.variables_count, 1);
        // .env wins over default
        assert_eq!(vault.get("CITY").await.unwrap().as_deref(), Some("Milan"));
    }

    #[tokio::test]
    async fn test_populate_variables_uses_default_when_no_env() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        let env_path = write_env(tmp.path(), ""); // no CITY in .env
        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[skills.weather.variables.CITY]
default = "Rome"
description = "Default city"
"#);

        let result = populate_vault(&deploy_path, Some(&env_path), &vault).await.unwrap();
        assert_eq!(result.variables_count, 1);
        assert_eq!(vault.get("CITY").await.unwrap().as_deref(), Some("Rome"));
    }

    #[tokio::test]
    async fn test_populate_variable_no_default_no_env_skipped() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[skills.my-skill.variables.REGION]
description = "Cloud region"
"#);

        let result = populate_vault(&deploy_path, None, &vault).await.unwrap();
        assert_eq!(result.variables_count, 0);
        assert!(vault.get("REGION").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_inject_env_from_vault() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        // Pre-populate vault
        vault.set("MY_SECRET", "secret_value").await.unwrap();
        vault.set("MY_VAR", "var_value").await.unwrap();

        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[agent.secrets.MY_SECRET]
secret = "MY_SECRET"
required = true
description = "A secret"

[agent.variables.MY_VAR]
default = "unused"
description = "A var"
"#);

        // Use unique env var names to avoid test pollution
        let count = inject_env_from_vault(&deploy_path, &vault).await.unwrap();
        assert_eq!(count, 2);
        assert_eq!(std::env::var("MY_SECRET").unwrap(), "secret_value");
        assert_eq!(std::env::var("MY_VAR").unwrap(), "var_value");

        // Cleanup
        std::env::remove_var("MY_SECRET");
        std::env::remove_var("MY_VAR");
    }

    #[tokio::test]
    async fn test_inject_no_deploy_toml() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;
        let count = inject_env_from_vault(&tmp.path().join("nope.toml"), &vault).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_inject_missing_vault_key_warns() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;
        // Vault is empty, deploy.toml declares a key
        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[agent.secrets.MISSING_KEY]
secret = "MISSING_KEY"
required = true
description = "Not in vault"
"#);

        let count = inject_env_from_vault(&deploy_path, &vault).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_populate_deduplicates_same_key_across_skills() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        let env_path = write_env(tmp.path(), "SHARED_TOKEN=shared_value\n");
        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[skills.skill-a.secrets.SHARED_TOKEN]
secret = "SHARED_TOKEN"
required = true
description = "Shared token (skill-a)"

[skills.skill-b.secrets.SHARED_TOKEN]
secret = "SHARED_TOKEN"
required = true
description = "Shared token (skill-b)"
"#);

        let result = populate_vault(&deploy_path, Some(&env_path), &vault).await.unwrap();
        // Should only be counted once despite appearing in two skills
        assert_eq!(result.secrets_count, 1);
        assert_eq!(vault.get("SHARED_TOKEN").await.unwrap().as_deref(), Some("shared_value"));
    }

    #[tokio::test]
    async fn test_full_roundtrip_populate_then_inject() {
        let tmp = TempDir::new().unwrap();
        let vault = test_vault(&tmp).await;

        let env_path = write_env(tmp.path(), "RT_API_KEY=sk-123\nRT_TIMEOUT=60\n");
        let deploy_path = write_deploy_toml(tmp.path(), r#"
version = 1

[agent.secrets.RT_API_KEY]
secret = "RT_API_KEY"
required = true
description = "API key"

[skills.my-skill.variables.RT_TIMEOUT]
default = "30"
description = "Timeout"

[skills.my-skill.variables.RT_CITY]
default = "Rome"
description = "City"
"#);

        // Build: populate
        let result = populate_vault(&deploy_path, Some(&env_path), &vault).await.unwrap();
        assert_eq!(result.secrets_count, 1);
        assert_eq!(result.variables_count, 2); // TIMEOUT from .env, CITY from default

        // Serve: inject
        let count = inject_env_from_vault(&deploy_path, &vault).await.unwrap();
        assert_eq!(count, 3);
        assert_eq!(std::env::var("RT_API_KEY").unwrap(), "sk-123");
        assert_eq!(std::env::var("RT_TIMEOUT").unwrap(), "60"); // .env override
        assert_eq!(std::env::var("RT_CITY").unwrap(), "Rome"); // default

        // Cleanup
        std::env::remove_var("RT_API_KEY");
        std::env::remove_var("RT_TIMEOUT");
        std::env::remove_var("RT_CITY");
    }
}
