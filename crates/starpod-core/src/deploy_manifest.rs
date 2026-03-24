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

// ── Skill env input ─────────────────────────────────────────────────────────

/// Input from a single skill's env declaration.
/// This mirrors the skill frontmatter env block without depending on starpod-skills.
pub struct SkillEnvInput {
    pub name: String,
    pub secrets: Vec<(String, bool, String)>,   // (key, required, description)
    pub variables: Vec<(String, Option<String>, String)>, // (key, default, description)
}

// ── Config input ────────────────────────────────────────────────────────────

/// Minimal config info needed to infer agent-level env requirements.
pub struct AgentConfigInput {
    /// Model specs (e.g. ["anthropic/claude-sonnet-4-6"]).
    pub models: Vec<String>,
    /// Whether telegram channel is enabled.
    pub telegram_enabled: bool,
}

// ── Generator ───────────────────────────────────────────────────────────────

impl DeployManifest {
    /// Generate a deploy manifest from agent config and skill env declarations.
    pub fn generate(config: &AgentConfigInput, skill_envs: Vec<SkillEnvInput>) -> Self {
        let mut agent = EnvSection::default();

        // Infer agent-level secrets from provider config
        let mut providers_seen = std::collections::HashSet::new();
        for model_spec in &config.models {
            if let Some(provider) = model_spec.split('/').next() {
                if providers_seen.insert(provider.to_string()) {
                    let key = format!("{}_API_KEY", provider.to_uppercase());
                    let desc = format!("{} API key", provider);
                    agent.secrets.insert(key.clone(), SecretEntry {
                        secret: key,
                        required: true,
                        description: desc,
                    });
                }
            }
        }

        // Telegram bot token if channel is enabled
        if config.telegram_enabled {
            agent.secrets.insert("TELEGRAM_BOT_TOKEN".to_string(), SecretEntry {
                secret: "TELEGRAM_BOT_TOKEN".to_string(),
                required: true,
                description: "Telegram bot token".to_string(),
            });
        }

        // Build per-skill sections
        let mut skills = BTreeMap::new();
        for skill_env in skill_envs {
            let mut section = EnvSection::default();
            for (key, required, description) in skill_env.secrets {
                section.secrets.insert(key.clone(), SecretEntry { secret: key, required, description });
            }
            for (key, default, description) in skill_env.variables {
                section.variables.insert(key, VariableEntry { default, description });
            }
            if !section.is_empty() {
                skills.insert(skill_env.name, section);
            }
        }

        DeployManifest {
            version: 1,
            agent,
            skills,
        }
    }

    /// Serialize to a TOML string with a header comment.
    pub fn to_toml(&self) -> Result<String> {
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| StarpodError::Config(format!("Failed to serialize deploy.toml: {}", e)))?;
        Ok(format!(
            "# deploy.toml — auto-generated, do not edit manually\n\
             # Regenerated on push/deploy from skill frontmatter + agent.toml\n\n{}",
            toml_str
        ))
    }

    /// Write the manifest to a file.
    pub fn write_to(&self, path: &Path) -> Result<()> {
        let content = self.to_toml()?;
        std::fs::write(path, content)
            .map_err(|e| StarpodError::Io(e))?;
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
        };
        let manifest = DeployManifest::generate(&config, vec![]);
        assert_eq!(manifest.version, 1);
        assert!(manifest.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
        assert!(manifest.skills.is_empty());
    }

    #[test]
    fn test_generate_with_telegram() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: true,
        };
        let manifest = DeployManifest::generate(&config, vec![]);
        assert!(manifest.agent.secrets.contains_key("TELEGRAM_BOT_TOKEN"));
    }

    #[test]
    fn test_generate_with_skill_envs() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
        };
        let skill_envs = vec![
            SkillEnvInput {
                name: "github-pr-review".to_string(),
                secrets: vec![
                    ("GITHUB_TOKEN".to_string(), true, "GitHub PAT".to_string()),
                ],
                variables: vec![
                    ("GITHUB_ORG".to_string(), Some("".to_string()), "Default org".to_string()),
                ],
            },
        ];
        let manifest = DeployManifest::generate(&config, skill_envs);
        assert!(manifest.skills.contains_key("github-pr-review"));
        let skill = &manifest.skills["github-pr-review"];
        assert!(skill.secrets.contains_key("GITHUB_TOKEN"));
        assert!(skill.secrets["GITHUB_TOKEN"].required);
        assert!(skill.variables.contains_key("GITHUB_ORG"));
        assert_eq!(skill.variables["GITHUB_ORG"].default.as_deref(), Some(""));
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
        };
        let manifest = DeployManifest::generate(&config, vec![]);
        assert!(manifest.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
        assert!(manifest.agent.secrets.contains_key("OPENAI_API_KEY"));
        assert_eq!(manifest.agent.secrets.len(), 2); // no duplicate
    }

    #[test]
    fn test_to_toml_output() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
        };
        let skill_envs = vec![
            SkillEnvInput {
                name: "check-weather".to_string(),
                secrets: vec![
                    ("OPENWEATHER_API_KEY".to_string(), true, "OpenWeatherMap API key".to_string()),
                ],
                variables: vec![
                    ("DEFAULT_CITY".to_string(), Some("Rome".to_string()), "Fallback city".to_string()),
                ],
            },
        ];
        let manifest = DeployManifest::generate(&config, skill_envs);
        let toml = manifest.to_toml().unwrap();

        assert!(toml.contains("# deploy.toml"));
        assert!(toml.contains("version = 1"));
        assert!(toml.contains("[agent.secrets.ANTHROPIC_API_KEY]"));
        assert!(toml.contains("secret = \"ANTHROPIC_API_KEY\""));
        assert!(toml.contains("[skills.check-weather.secrets.OPENWEATHER_API_KEY]"));
        assert!(toml.contains("secret = \"OPENWEATHER_API_KEY\""));
        assert!(toml.contains("[skills.check-weather.variables.DEFAULT_CITY]"));
        assert!(toml.contains("default = \"Rome\""));
    }

    #[test]
    fn test_write_to_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("deploy.toml");

        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
        };
        let manifest = DeployManifest::generate(&config, vec![]);
        manifest.write_to(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("version = 1"));
    }

    #[test]
    fn test_empty_models_no_agent_secrets() {
        let config = AgentConfigInput {
            models: vec![],
            telegram_enabled: false,
        };
        let manifest = DeployManifest::generate(&config, vec![]);
        assert!(manifest.agent.secrets.is_empty());
    }

    #[test]
    fn test_malformed_model_spec_no_slash() {
        let config = AgentConfigInput {
            models: vec!["just-a-model".to_string()],
            telegram_enabled: false,
        };
        let manifest = DeployManifest::generate(&config, vec![]);
        // Should use the whole string as provider name
        assert!(manifest.agent.secrets.contains_key("JUST-A-MODEL_API_KEY"));
    }

    #[test]
    fn test_multiple_skills_sorted() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
        };
        let skill_envs = vec![
            SkillEnvInput {
                name: "zeta-skill".to_string(),
                secrets: vec![("Z_KEY".to_string(), true, "Z key".to_string())],
                variables: vec![],
            },
            SkillEnvInput {
                name: "alpha-skill".to_string(),
                secrets: vec![("A_KEY".to_string(), true, "A key".to_string())],
                variables: vec![],
            },
            SkillEnvInput {
                name: "mid-skill".to_string(),
                secrets: vec![],
                variables: vec![("M_VAR".to_string(), Some("default".to_string()), "M var".to_string())],
            },
        ];
        let manifest = DeployManifest::generate(&config, skill_envs);
        assert_eq!(manifest.skills.len(), 3);
        // BTreeMap ensures sorted order
        let keys: Vec<&String> = manifest.skills.keys().collect();
        assert_eq!(keys, vec!["alpha-skill", "mid-skill", "zeta-skill"]);
    }

    #[test]
    fn test_skill_with_empty_env_omitted() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: false,
        };
        let skill_envs = vec![
            SkillEnvInput {
                name: "empty-env".to_string(),
                secrets: vec![],
                variables: vec![],
            },
        ];
        let manifest = DeployManifest::generate(&config, skill_envs);
        // Skills with no secrets or variables should be omitted
        assert!(manifest.skills.is_empty());
    }

    #[test]
    fn test_secret_with_empty_description() {
        let config = AgentConfigInput {
            models: vec![],
            telegram_enabled: false,
        };
        let skill_envs = vec![
            SkillEnvInput {
                name: "terse".to_string(),
                secrets: vec![("TOKEN".to_string(), true, String::new())],
                variables: vec![],
            },
        ];
        let manifest = DeployManifest::generate(&config, skill_envs);
        assert!(manifest.skills["terse"].secrets["TOKEN"].description.is_empty());
        // Verify TOML output doesn't include empty description
        let toml = manifest.to_toml().unwrap();
        assert!(toml.contains("[skills.terse.secrets.TOKEN]"));
        assert!(toml.contains("required = true"));
    }

    #[test]
    fn test_variable_without_default() {
        let config = AgentConfigInput {
            models: vec![],
            telegram_enabled: false,
        };
        let skill_envs = vec![
            SkillEnvInput {
                name: "no-defaults".to_string(),
                secrets: vec![],
                variables: vec![("REGION".to_string(), None, "Cloud region".to_string())],
            },
        ];
        let manifest = DeployManifest::generate(&config, skill_envs);
        assert!(manifest.skills["no-defaults"].variables["REGION"].default.is_none());
    }

    #[test]
    fn test_toml_roundtrip_parseable() {
        let config = AgentConfigInput {
            models: vec!["anthropic/claude-sonnet-4-6".to_string()],
            telegram_enabled: true,
        };
        let skill_envs = vec![
            SkillEnvInput {
                name: "my-skill".to_string(),
                secrets: vec![("KEY".to_string(), true, "A key".to_string())],
                variables: vec![("VAR".to_string(), Some("val".to_string()), "A var".to_string())],
            },
        ];
        let manifest = DeployManifest::generate(&config, skill_envs);
        let toml_str = manifest.to_toml().unwrap();

        // Strip the header comment lines for parsing
        let toml_body: String = toml_str.lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed: DeployManifest = toml::from_str(&toml_body).unwrap();
        assert_eq!(parsed.version, 1);
        assert!(parsed.agent.secrets.contains_key("ANTHROPIC_API_KEY"));
        assert_eq!(parsed.agent.secrets["ANTHROPIC_API_KEY"].secret, "ANTHROPIC_API_KEY");
        assert!(parsed.agent.secrets.contains_key("TELEGRAM_BOT_TOKEN"));
        assert_eq!(parsed.agent.secrets["TELEGRAM_BOT_TOKEN"].secret, "TELEGRAM_BOT_TOKEN");
        assert!(parsed.skills["my-skill"].secrets.contains_key("KEY"));
        assert_eq!(parsed.skills["my-skill"].secrets["KEY"].secret, "KEY");
        assert_eq!(parsed.skills["my-skill"].variables["VAR"].default.as_deref(), Some("val"));
    }
}
