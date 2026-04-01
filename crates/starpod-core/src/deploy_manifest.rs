//! Deploy manifest generator.
//!
//! Produces a `deploy.toml` that declares all environment requirements
//! (secrets + variables) for an agent deployment. The manifest is generated
//! client-side by scanning agent config and skill frontmatter, then pushed
//! as part of the blueprint so the backend can validate readiness.
//!
//! # Structure
//!
//! The manifest is organized by owner — agent-level requirements first,
//! then per-skill sections:
//!
//! ```toml
//! version = 1
//!
//! [agent.secrets.ANTHROPIC_API_KEY]
//! secret = "ANTHROPIC_API_KEY"
//! required = true
//! description = "anthropic API key"
//!
//! [skills.github-pr-review.secrets.GITHUB_TOKEN]
//! secret = "GITHUB_TOKEN"
//! required = true
//! description = "GitHub PAT for PR access"
//!
//! [skills.github-pr-review.variables.GITHUB_ORG]
//! default = ""
//! description = "Default org to scope PRs"
//! ```
//!
//! # Secret resolution
//!
//! Each secret entry has a `secret` field that names the vault secret to
//! resolve from. By default this matches the declaration key, but can be
//! overridden to alias a differently-named vault secret (e.g. the agent
//! declares `ANTHROPIC_API_KEY` but resolves from `ANTHROPIC_API_KEY_PROD`).
//!
//! The full resolution chain on the platform is:
//! `instance.secret_overrides[key]` → `deploy.toml secret field` → `literal key`

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Result, StarpodError};

// ── Manifest types ──────────────────────────────────────────────────────────

/// A secret entry in deploy.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    /// Which stored secret to resolve from the vault. Defaults to the
    /// declaration key (e.g. `GITHUB_TOKEN`), but can be overridden to
    /// alias a differently-named vault secret (e.g. `GITHUB_TOKEN_PROD`).
    pub secret: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A variable entry in deploy.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Env section for either agent-level or a specific skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvSection {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, SecretEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<String, VariableEntry>,
}

/// The full deploy.toml manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployManifest {
    pub version: u32,
    #[serde(default, skip_serializing_if = "EnvSection::is_empty")]
    pub agent: EnvSection,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skills: BTreeMap<String, EnvSection>,
}

impl EnvSection {
    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty() && self.variables.is_empty()
    }
}

// ── Config input ────────────────────────────────────────────────────────────

/// Minimal config info needed to infer agent-level env requirements.
pub struct AgentConfigInput {
    /// Model specs (e.g. ["anthropic/claude-sonnet-4-6"]).
    pub models: Vec<String>,
    /// Whether telegram channel is enabled.
    pub telegram_enabled: bool,
    /// Whether internet/web search is enabled (needs BRAVE_API_KEY).
    pub internet_enabled: bool,
}

// ── Generator ───────────────────────────────────────────────────────────────

impl DeployManifest {
    /// Generate a deploy manifest from agent config.
    ///
    /// Skills no longer declare env requirements (they reference connectors
    /// instead), so the manifest only contains agent-level secrets.
    pub fn generate(config: &AgentConfigInput) -> Self {
        let mut agent = EnvSection::default();

        // Infer agent-level secrets from provider config
        // Local providers (ollama, etc.) don't require API keys
        const LOCAL_PROVIDERS: &[&str] = &["ollama"];
        let mut providers_seen = std::collections::HashSet::new();
        for model_spec in &config.models {
            if let Some(provider) = model_spec.split('/').next() {
                if providers_seen.insert(provider.to_string()) {
                    let is_local = LOCAL_PROVIDERS.contains(&provider);

                    // Bedrock uses AWS credentials (access key + secret key), not a single API key
                    if provider == "bedrock" {
                        agent.secrets.insert(
                            "AWS_ACCESS_KEY_ID".to_string(),
                            SecretEntry {
                                secret: "AWS_ACCESS_KEY_ID".to_string(),
                                required: true,
                                description: "AWS access key ID for Bedrock".to_string(),
                            },
                        );
                        agent.secrets.insert(
                            "AWS_SECRET_ACCESS_KEY".to_string(),
                            SecretEntry {
                                secret: "AWS_SECRET_ACCESS_KEY".to_string(),
                                required: true,
                                description: "AWS secret access key for Bedrock".to_string(),
                            },
                        );
                    } else if provider == "vertex" {
                        // Vertex uses Google service account credentials
                        agent.secrets.insert(
                            "GOOGLE_APPLICATION_CREDENTIALS".to_string(),
                            SecretEntry {
                                secret: "GOOGLE_APPLICATION_CREDENTIALS".to_string(),
                                required: true,
                                description: "Path to Google service account JSON for Vertex AI"
                                    .to_string(),
                            },
                        );
                    } else {
                        let key = format!("{}_API_KEY", provider.to_uppercase());
                        let desc = format!("{} API key", provider);
                        agent.secrets.insert(
                            key.clone(),
                            SecretEntry {
                                secret: key,
                                required: !is_local,
                                description: desc,
                            },
                        );
                    }
                }
            }
        }

        // Telegram bot token if channel is enabled
        if config.telegram_enabled {
            agent.secrets.insert(
                "TELEGRAM_BOT_TOKEN".to_string(),
                SecretEntry {
                    secret: "TELEGRAM_BOT_TOKEN".to_string(),
                    required: true,
                    description: "Telegram bot token".to_string(),
                },
            );
        }

        // Brave Search API key if internet/web search is enabled
        if config.internet_enabled {
            agent.secrets.insert(
                "BRAVE_API_KEY".to_string(),
                SecretEntry {
                    secret: "BRAVE_API_KEY".to_string(),
                    required: false,
                    description: "Brave Search API key for web search".to_string(),
                },
            );
        }

        DeployManifest {
            version: 1,
            agent,
            skills: BTreeMap::new(),
        }
    }

    /// Load an existing deploy.toml from a file path.
    /// Returns `None` if the file doesn't exist.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path).map_err(StarpodError::Io)?;
        // Strip comment lines before parsing
        let body: String = content
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let manifest = toml::from_str(&body)
            .map_err(|e| StarpodError::Config(format!("Failed to parse deploy.toml: {}", e)))?;
        Ok(Some(manifest))
    }

    /// Merge a freshly generated manifest with an existing one, preserving
    /// user customizations.
    ///
    /// **What the generator owns** (overwritten from source of truth):
    /// - Which keys exist in `skills.*.secrets` and `skills.*.variables`
    /// - `required` and `description` on all entries
    ///
    /// **What the user owns** (preserved when already set):
    /// - `secret` field on secret entries (vault alias)
    /// - `default` on variable entries (operator override)
    /// - Any extra entries under `agent.*` (user-added, not from skills/config)
    ///
    /// **What gets removed**:
    /// - Skill sections for skills no longer present
    /// - Skill secret/variable keys removed from frontmatter
    pub fn merge_with_existing(mut self, existing: &DeployManifest) -> Self {
        // ── Agent section: merge, preserve user additions ───────────
        // For generated agent secrets: preserve user's `secret` alias
        for (key, entry) in &mut self.agent.secrets {
            if let Some(old) = existing.agent.secrets.get(key) {
                // Preserve user's alias if they changed it from the default
                if old.secret != *key {
                    entry.secret = old.secret.clone();
                }
            }
        }
        // For generated agent variables: preserve user's `default` override
        for (key, entry) in &mut self.agent.variables {
            if let Some(old) = existing.agent.variables.get(key) {
                entry.default = old.default.clone();
            }
        }
        // Keep user-added agent entries that the generator didn't produce
        for (key, entry) in &existing.agent.secrets {
            if !self.agent.secrets.contains_key(key) {
                self.agent.secrets.insert(key.clone(), entry.clone());
            }
        }
        for (key, entry) in &existing.agent.variables {
            if !self.agent.variables.contains_key(key) {
                self.agent.variables.insert(key.clone(), entry.clone());
            }
        }

        // ── Skill sections: merge per-skill, drop removed skills ────
        // Only skills present in self (generated) survive. Removed skills
        // are dropped. Within each skill, preserve user's aliases/defaults.
        for (skill_name, section) in &mut self.skills {
            if let Some(old_section) = existing.skills.get(skill_name) {
                for (key, entry) in &mut section.secrets {
                    if let Some(old) = old_section.secrets.get(key) {
                        if old.secret != *key {
                            entry.secret = old.secret.clone();
                        }
                    }
                }
                for (key, entry) in &mut section.variables {
                    if let Some(old) = old_section.variables.get(key) {
                        entry.default = old.default.clone();
                    }
                }
            }
        }

        self
    }

    /// Generate, merge with any existing deploy.toml at `path`, and write.
    pub fn generate_and_write(
        config: &AgentConfigInput,
        path: &Path,
    ) -> Result<Self> {
        let generated = Self::generate(config);
        let merged = if let Some(existing) = Self::load(path)? {
            generated.merge_with_existing(&existing)
        } else {
            generated
        };
        merged.write_to(path)?;
        Ok(merged)
    }

    /// Serialize to a TOML string with a header comment.
    pub fn to_toml(&self) -> Result<String> {
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| StarpodError::Config(format!("Failed to serialize deploy.toml: {}", e)))?;
        Ok(format!(
            "# deploy.toml — auto-generated on push/deploy\n\
             # User edits to `secret` aliases and variable `default` values are preserved\n\n{}",
            toml_str
        ))
    }

    /// Write the manifest to a file.
    pub fn write_to(&self, path: &Path) -> Result<()> {
        let content = self.to_toml()?;
        std::fs::write(path, content).map_err(StarpodError::Io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_minimal() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert_eq!(manifest.version, 1);
        assert!(manifest.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
        assert!(manifest.skills.is_empty());
    }

    #[test]
    fn test_generate_with_telegram() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: true,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest.agent.secrets.contains_key("TELEGRAM_BOT_TOKEN"));
    }

    #[test]
    fn test_generate_has_no_skill_sections() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest.skills.is_empty());
    }

    #[test]
    fn test_multiple_providers() {
        let config = AgentConfigInput {
            models: vec![
                "anthropic/claude-sonnet-4-6".to_string(),
                "openai/gpt-4o".to_string(),
                "anthropic/claude-haiku-4-5".to_string(), // duplicate provider
            ],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
        assert!(manifest.agent.secrets.contains_key("OPENAI_API_KEY"));
        assert_eq!(manifest.agent.secrets.len(), 2); // no duplicate
    }

    #[test]
    fn test_local_provider_optional_api_key() {
        let config = AgentConfigInput {
            models: vec!["ollama/llama3".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest.agent.secrets.contains_key("OLLAMA_API_KEY"));
        assert!(!manifest.agent.secrets["OLLAMA_API_KEY"].required);
    }

    #[test]
    fn test_to_toml_output() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        let toml = manifest.to_toml().unwrap();

        assert!(toml.contains("# deploy.toml"));
        assert!(toml.contains("version = 1"));
        assert!(toml.contains("[agent.secrets.ANTHROPIC_API_KEY]"));
        assert!(toml.contains("secret = \"ANTHROPIC_API_KEY\""));
    }

    #[test]
    fn test_write_to_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("deploy.toml");

        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        manifest.write_to(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("version = 1"));
    }

    #[test]
    fn test_empty_models_no_agent_secrets() {
        let config = AgentConfigInput {
            models: vec![],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest.agent.secrets.is_empty());
    }

    #[test]
    fn test_malformed_model_spec_no_slash() {
        let config = AgentConfigInput {
            models: vec!["just-a-model".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        // Should use the whole string as provider name
        assert!(manifest.agent.secrets.contains_key("JUST-A-MODEL_API_KEY"));
    }

    #[test]
    fn test_toml_roundtrip_parseable() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: true,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        let toml_str = manifest.to_toml().unwrap();

        // Strip the header comment lines for parsing
        let toml_body: String = toml_str
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed: DeployManifest = toml::from_str(&toml_body).unwrap();
        assert_eq!(parsed.version, 1);
        assert!(parsed.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
        assert_eq!(
            parsed.agent.secrets["ANTHROPIC_API_KEY"].secret,
            "ANTHROPIC_API_KEY"
        );
        assert!(parsed.agent.secrets.contains_key("TELEGRAM_BOT_TOKEN"));
        assert_eq!(
            parsed.agent.secrets["TELEGRAM_BOT_TOKEN"].secret,
            "TELEGRAM_BOT_TOKEN"
        );
    }

    // ── Merge tests ───────────────────────────────────────────────────

    fn minimal_config() -> AgentConfigInput {
        AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        }
    }

    #[test]
    fn test_merge_preserves_agent_secret_alias() {
        let mut existing = DeployManifest::generate(&minimal_config());
        existing
            .agent
            .secrets
            .get_mut("ANTHROPIC_API_KEY")
            .unwrap()
            .secret = "ANTHROPIC_API_KEY_STAGING".to_string();

        let generated = DeployManifest::generate(&minimal_config());
        let merged = generated.merge_with_existing(&existing);

        assert_eq!(
            merged.agent.secrets["ANTHROPIC_API_KEY"].secret,
            "ANTHROPIC_API_KEY_STAGING"
        );
    }

    #[test]
    fn test_merge_keeps_user_added_agent_entries() {
        let mut existing = DeployManifest::generate(&minimal_config());
        // User manually added a custom agent secret
        existing.agent.secrets.insert(
            "CUSTOM_API_KEY".to_string(),
            SecretEntry {
                secret: "CUSTOM_API_KEY".to_string(),
                required: false,
                description: "User-added custom key".to_string(),
            },
        );
        // User manually added an agent variable
        existing.agent.variables.insert(
            "LOG_LEVEL".to_string(),
            VariableEntry {
                default: Some("info".to_string()),
                description: "Log level".to_string(),
            },
        );

        let generated = DeployManifest::generate(&minimal_config());
        let merged = generated.merge_with_existing(&existing);

        // User additions preserved
        assert!(merged.agent.secrets.contains_key("CUSTOM_API_KEY"));
        assert_eq!(
            merged.agent.secrets["CUSTOM_API_KEY"].description,
            "User-added custom key"
        );
        assert!(merged.agent.variables.contains_key("LOG_LEVEL"));
    }

    #[test]
    fn test_generate_and_write_merges() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("deploy.toml");

        let config = minimal_config();

        // First generation
        DeployManifest::generate_and_write(&config, &path).unwrap();

        // User edits the file: change alias
        let mut manifest = DeployManifest::load(&path).unwrap().unwrap();
        manifest
            .agent
            .secrets
            .get_mut("ANTHROPIC_API_KEY")
            .unwrap()
            .secret = "ANTHROPIC_API_KEY_PROD".to_string();
        manifest.write_to(&path).unwrap();

        // Second generation
        let result = DeployManifest::generate_and_write(&config, &path).unwrap();

        // User's alias preserved
        assert_eq!(
            result.agent.secrets["ANTHROPIC_API_KEY"].secret,
            "ANTHROPIC_API_KEY_PROD"
        );
    }

    #[test]
    fn test_load_nonexistent_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = DeployManifest::load(&tmp.path().join("nope.toml")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_corrupt_file_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("deploy.toml");
        std::fs::write(&path, "not valid toml {{{").unwrap();
        assert!(DeployManifest::load(&path).is_err());
    }

    #[test]
    fn test_merge_with_empty_existing() {
        let existing = DeployManifest {
            version: 1,
            agent: EnvSection::default(),
            skills: BTreeMap::new(),
        };

        let generated = DeployManifest::generate(&minimal_config());

        let merged = generated.merge_with_existing(&existing);
        // Everything from generated comes through
        assert!(merged.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_generate_and_write_first_run_no_existing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("deploy.toml");

        // File doesn't exist yet
        assert!(!path.exists());

        let result = DeployManifest::generate_and_write(&minimal_config(), &path).unwrap();

        assert!(path.exists());
        assert!(result.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_merge_preserves_user_added_agent_variable_default() {
        let mut existing = DeployManifest::generate(&minimal_config());
        existing.agent.variables.insert(
            "DEBUG".to_string(),
            VariableEntry {
                default: Some("true".to_string()),
                description: "Debug mode".to_string(),
            },
        );

        let generated = DeployManifest::generate(&minimal_config());
        let merged = generated.merge_with_existing(&existing);

        assert!(merged.agent.variables.contains_key("DEBUG"));
        assert_eq!(
            merged.agent.variables["DEBUG"].default.as_deref(),
            Some("true")
        );
    }

    // ── Bedrock provider tests ──────────────────────────────────────────

    #[test]
    fn test_bedrock_provider_generates_aws_credentials() {
        let config = AgentConfigInput {
            models: vec!["bedrock/eu.anthropic.claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest.agent.secrets.contains_key("AWS_ACCESS_KEY_ID"));
        assert!(manifest.agent.secrets.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(manifest.agent.secrets["AWS_ACCESS_KEY_ID"].required);
        assert!(manifest.agent.secrets["AWS_SECRET_ACCESS_KEY"].required);
        // Should NOT contain a BEDROCK_API_KEY
        assert!(!manifest.agent.secrets.contains_key("BEDROCK_API_KEY"));
    }

    #[test]
    fn test_bedrock_provider_no_duplicate_aws_keys() {
        let config = AgentConfigInput {
            models: vec![
                "bedrock/eu.anthropic.claude-sonnet-4-6".to_string(),
                "bedrock/us.anthropic.claude-opus-4-6-v1".to_string(),
            ],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        // Only 2 secrets (the AWS credential pair), not duplicated
        assert_eq!(manifest.agent.secrets.len(), 2);
        assert!(manifest.agent.secrets.contains_key("AWS_ACCESS_KEY_ID"));
        assert!(manifest.agent.secrets.contains_key("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn test_bedrock_mixed_with_other_providers() {
        let config = AgentConfigInput {
            models: vec![
                "bedrock/eu.anthropic.claude-sonnet-4-6".to_string(),
                "anthropic/claude-haiku-4-5".to_string(),
            ],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest.agent.secrets.contains_key("AWS_ACCESS_KEY_ID"));
        assert!(manifest.agent.secrets.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(manifest.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
        assert_eq!(manifest.agent.secrets.len(), 3);
    }

    #[test]
    fn test_bedrock_toml_output_contains_aws_keys() {
        let config = AgentConfigInput {
            models: vec!["bedrock/eu.anthropic.claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        let toml = manifest.to_toml().unwrap();
        assert!(toml.contains("[agent.secrets.AWS_ACCESS_KEY_ID]"));
        assert!(toml.contains("[agent.secrets.AWS_SECRET_ACCESS_KEY]"));
        assert!(!toml.contains("BEDROCK_API_KEY"));
    }

    // ── Vertex AI provider tests ────────────────────────────────────────

    #[test]
    fn test_vertex_provider_generates_gcp_credentials() {
        let config = AgentConfigInput {
            models: vec!["vertex/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest
            .agent
            .secrets
            .contains_key("GOOGLE_APPLICATION_CREDENTIALS"));
        assert!(manifest.agent.secrets["GOOGLE_APPLICATION_CREDENTIALS"].required);
        // Should NOT contain a VERTEX_API_KEY
        assert!(!manifest.agent.secrets.contains_key("VERTEX_API_KEY"));
    }

    #[test]
    fn test_vertex_provider_no_duplicate_gcp_keys() {
        let config = AgentConfigInput {
            models: vec![
                "vertex/claude-sonnet-4-6".to_string(),
                "vertex/claude-opus-4-6".to_string(),
            ],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        // Only 1 secret (GOOGLE_APPLICATION_CREDENTIALS), not duplicated
        assert_eq!(manifest.agent.secrets.len(), 1);
        assert!(manifest
            .agent
            .secrets
            .contains_key("GOOGLE_APPLICATION_CREDENTIALS"));
    }

    #[test]
    fn test_vertex_mixed_with_other_providers() {
        let config = AgentConfigInput {
            models: vec![
                "vertex/claude-sonnet-4-6".to_string(),
                "anthropic/claude-haiku-4-5".to_string(),
                "bedrock/eu.anthropic.claude-sonnet-4-6".to_string(),
            ],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        assert!(manifest
            .agent
            .secrets
            .contains_key("GOOGLE_APPLICATION_CREDENTIALS"));
        assert!(manifest.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
        assert!(manifest.agent.secrets.contains_key("AWS_ACCESS_KEY_ID"));
        assert!(manifest.agent.secrets.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert_eq!(manifest.agent.secrets.len(), 4);
    }

    #[test]
    fn test_vertex_toml_output_contains_gcp_keys() {
        let config = AgentConfigInput {
            models: vec!["vertex/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
            internet_enabled: false,
        };
        let manifest = DeployManifest::generate(&config);
        let toml = manifest.to_toml().unwrap();
        assert!(toml.contains("[agent.secrets.GOOGLE_APPLICATION_CREDENTIALS]"));
        assert!(!toml.contains("VERTEX_API_KEY"));
    }
}
