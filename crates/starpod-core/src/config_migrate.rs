//! Config migration system for `agent.toml`.
//!
//! When the config schema evolves between Starpod versions, numbered migrations
//! transform the raw TOML to match the new schema. This runs at startup
//! (in `main.rs`, before [`load_agent_config`](crate::load_agent_config)) so the
//! config is always in the expected format by the time the agent reads it.
//!
//! # How it works
//!
//! 1. Read `agent.toml` and check the `config_version` integer field (default: 0).
//! 2. Compare against the migration registry — a `Vec<Migration>` in ascending order.
//! 3. Apply each migration whose version is greater than the current version.
//! 4. Write the updated TOML back with the new `config_version`.
//!
//! # Adding a new migration
//!
//! Append to the [`migrations()`] function:
//!
//! ```rust,ignore
//! Migration {
//!     version: 2,
//!     apply: |doc| {
//!         // Example: rename "old_field" → "new_field"
//!         if let Some(table) = doc.as_table_mut() {
//!             if let Some(val) = table.remove("old_field") {
//!                 table.insert("new_field".into(), val);
//!             }
//!         }
//!     },
//! },
//! ```
//!
//! Migrations must be listed in ascending version order with no gaps.
//! The file is only rewritten when changes are needed.

use std::path::Path;

use tracing::{debug, info};

use crate::{Result, StarpodError};

/// A single config migration step.
///
/// `version` is the version this migration upgrades *to*. For example, a
/// migration with `version: 2` runs when the config is at version 1.
/// The `apply` function receives the entire TOML document as a mutable
/// [`toml::Value`] and should transform it in-place.
struct Migration {
    /// Target version after this migration runs.
    version: u32,
    /// Transform function that modifies the TOML document in-place.
    apply: fn(&mut toml::Value),
}

/// The global migration registry. Add new migrations here.
///
/// Each migration's `version` is the version it migrates *to*. They must be
/// listed in ascending order. A config at version N will have migrations
/// N+1, N+2, ... applied in sequence.
fn migrations() -> Vec<Migration> {
    vec![
        // Migration 1: ensure config_version field exists (baseline).
        // Existing configs without config_version are treated as version 0.
        Migration {
            version: 1,
            apply: |_doc| {
                // Baseline — no transforms needed. Just establishes versioning.
            },
        },
    ]
}

/// Read `agent.toml`, apply any pending config migrations, and write back.
///
/// Returns `Ok(true)` if migrations were applied, `Ok(false)` if already up to date.
/// The file is only written if changes were made.
///
/// # Behavior
///
/// - **Missing file**: returns `Ok(false)` — nothing to migrate.
/// - **No `config_version` field**: treated as version 0 (all migrations apply).
/// - **`config_version` >= latest**: no-op, returns `Ok(false)`.
/// - **`config_version` > latest** (downgraded binary): also no-op.
///
/// # Errors
///
/// Returns `Err` if the file exists but is unreadable, contains invalid TOML,
/// or cannot be written back after migration.
pub fn migrate_config(agent_toml: &Path) -> Result<bool> {
    if !agent_toml.exists() {
        debug!(path = %agent_toml.display(), "agent.toml not found, skipping config migration");
        return Ok(false);
    }

    let content = std::fs::read_to_string(agent_toml).map_err(|e| {
        StarpodError::Config(format!("Failed to read {}: {}", agent_toml.display(), e))
    })?;

    let mut doc: toml::Value = content.parse().map_err(|e| {
        StarpodError::Config(format!("Failed to parse {}: {}", agent_toml.display(), e))
    })?;

    let current_version = doc
        .get("config_version")
        .and_then(|v| v.as_integer())
        .unwrap_or(0) as u32;

    let all_migrations = migrations();
    let latest_version = all_migrations.last().map(|m| m.version).unwrap_or(0);

    if current_version >= latest_version {
        debug!(
            version = current_version,
            "config already at latest version"
        );
        return Ok(false);
    }

    info!(
        from = current_version,
        to = latest_version,
        "migrating agent.toml"
    );

    for migration in &all_migrations {
        if migration.version > current_version {
            debug!(version = migration.version, "applying config migration");
            (migration.apply)(&mut doc);
        }
    }

    // Set config_version
    if let Some(table) = doc.as_table_mut() {
        table.insert(
            "config_version".to_string(),
            toml::Value::Integer(latest_version as i64),
        );
    }

    let output = toml::to_string_pretty(&doc)
        .map_err(|e| StarpodError::Config(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(agent_toml, output).map_err(|e| {
        StarpodError::Config(format!("Failed to write {}: {}", agent_toml.display(), e))
    })?;

    info!(version = latest_version, "config migration complete");
    Ok(true)
}

/// Return the latest config version number from the migration registry.
///
/// This is the `config_version` value that `agent.toml` will have after
/// all migrations have been applied. Useful in tests to assert the final state.
pub fn latest_config_version() -> u32 {
    migrations().last().map(|m| m.version).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── Basic migration scenarios ──────────────────────────────────────

    #[test]
    fn migrate_fresh_config_adds_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "agent_name = \"Test\"").unwrap();

        let migrated = migrate_config(&path).unwrap();
        assert!(migrated);

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        assert_eq!(
            doc.get("config_version")
                .and_then(|v| v.as_integer())
                .unwrap(),
            latest_config_version() as i64
        );
    }

    #[test]
    fn migrate_preserves_all_existing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(
            &path,
            r#"
agent_name = "Nova"
timezone = "Europe/Rome"
max_turns = 200
max_tokens = 16384
server_addr = "0.0.0.0:3001"
models = ["anthropic/claude-haiku-4-5"]
"#,
        )
        .unwrap();

        migrate_config(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        assert_eq!(doc["agent_name"].as_str().unwrap(), "Nova");
        assert_eq!(doc["timezone"].as_str().unwrap(), "Europe/Rome");
        assert_eq!(doc["max_turns"].as_integer().unwrap(), 200);
        assert_eq!(doc["max_tokens"].as_integer().unwrap(), 16384);
        assert_eq!(doc["server_addr"].as_str().unwrap(), "0.0.0.0:3001");
        let models = doc["models"].as_array().unwrap();
        assert_eq!(models[0].as_str().unwrap(), "anthropic/claude-haiku-4-5");
    }

    #[test]
    fn migrate_already_current_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        let content = format!(
            "config_version = {}\nagent_name = \"Test\"\n",
            latest_config_version()
        );
        std::fs::write(&path, &content).unwrap();

        let migrated = migrate_config(&path).unwrap();
        assert!(!migrated);

        // File should be unchanged
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, content);
    }

    #[test]
    fn migrate_nonexistent_file_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let migrated = migrate_config(&path).unwrap();
        assert!(!migrated);
    }

    // ── Idempotency ────────────────────────────────────────────────────

    #[test]
    fn migrate_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(&path, "agent_name = \"Test\"\n").unwrap();

        // First migration applies
        assert!(migrate_config(&path).unwrap());

        // Read state after first migration
        let after_first = std::fs::read_to_string(&path).unwrap();

        // Second migration is a no-op
        assert!(!migrate_config(&path).unwrap());

        // Content unchanged
        let after_second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after_first, after_second);
    }

    // ── Edge cases ─────────────────────────────────────────────────────

    #[test]
    fn migrate_empty_file_treated_as_version_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        // Empty TOML is a valid empty table
        std::fs::write(&path, "").unwrap();

        let migrated = migrate_config(&path).unwrap();
        assert!(migrated);

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        assert_eq!(
            doc["config_version"].as_integer().unwrap(),
            latest_config_version() as i64
        );
    }

    #[test]
    fn migrate_preserves_nested_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(
            &path,
            r#"
agent_name = "Test"

[auth]
rate_limit_requests = 100
rate_limit_window_secs = 60

[memory]
enabled = true
auto_index = true
"#,
        )
        .unwrap();

        migrate_config(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        assert_eq!(
            doc["auth"]["rate_limit_requests"].as_integer().unwrap(),
            100
        );
        assert_eq!(doc["memory"]["enabled"].as_bool().unwrap(), true);
    }

    #[test]
    fn migrate_preserves_array_of_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(
            &path,
            r#"
models = ["anthropic/claude-haiku-4-5", "ollama/llama3"]
agent_name = "Multi"
"#,
        )
        .unwrap();

        migrate_config(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        let models = doc["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].as_str().unwrap(), "anthropic/claude-haiku-4-5");
        assert_eq!(models[1].as_str().unwrap(), "ollama/llama3");
    }

    #[test]
    fn migrate_handles_future_config_version_gracefully() {
        // If someone has a config_version higher than ours, don't touch it
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(&path, "config_version = 9999\n").unwrap();

        let migrated = migrate_config(&path).unwrap();
        assert!(!migrated);
    }

    #[test]
    fn migrate_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(&path, "this is not valid toml [[[").unwrap();

        let result = migrate_config(&path);
        assert!(result.is_err());
    }

    #[test]
    fn migrate_config_version_zero_treated_as_unversioned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(&path, "config_version = 0\nagent_name = \"Test\"\n").unwrap();

        // Version 0 should trigger migration to version 1
        let migrated = migrate_config(&path).unwrap();
        assert!(migrated);

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        assert_eq!(
            doc["config_version"].as_integer().unwrap(),
            latest_config_version() as i64
        );
    }

    // ── latest_config_version ──────────────────────────────────────────

    #[test]
    fn latest_config_version_is_positive() {
        assert!(latest_config_version() >= 1);
    }

    #[test]
    fn latest_config_version_matches_registry() {
        let all = migrations();
        let max = all.iter().map(|m| m.version).max().unwrap_or(0);
        assert_eq!(latest_config_version(), max);
    }

    // ── Migration registry invariants ──────────────────────────────────

    #[test]
    fn migrations_are_in_ascending_order() {
        let all = migrations();
        for window in all.windows(2) {
            assert!(
                window[0].version < window[1].version,
                "migration {} should come before {}",
                window[0].version,
                window[1].version
            );
        }
    }

    #[test]
    fn migrations_start_at_one() {
        let all = migrations();
        assert!(!all.is_empty());
        assert_eq!(all[0].version, 1);
    }

    #[test]
    fn migrations_have_no_gaps() {
        let all = migrations();
        for (i, m) in all.iter().enumerate() {
            assert_eq!(
                m.version,
                (i + 1) as u32,
                "expected version {} at index {}, got {}",
                i + 1,
                i,
                m.version
            );
        }
    }
}
