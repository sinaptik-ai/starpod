//! Agent scaffolding helpers.

use std::sync::LazyLock;

use agent_sdk::models::ModelRegistry;

/// Shared model registry (embedded defaults).
#[allow(dead_code)]
static REGISTRY: LazyLock<ModelRegistry> = LazyLock::new(ModelRegistry::with_defaults);

/// Provider choices available during init, derived from the model registry.
#[allow(dead_code)]
pub fn providers() -> Vec<&'static str> {
    REGISTRY.provider_names()
}

/// Default model for a provider, from the model registry.
#[allow(dead_code)]
pub fn default_model(provider: &str) -> &str {
    REGISTRY
        .default_model(provider)
        .unwrap_or("claude-haiku-4-5")
}

/// Environment variable name for a provider's API key, from the model registry.
#[allow(dead_code)]
pub fn env_key_for_provider(provider: &str) -> Option<&str> {
    REGISTRY.api_key_env(provider)
}

/// Generate `agent.toml` content for a new agent instance.
pub fn generate_agent_toml(agent_name: &str, provider: &str, model: &str) -> String {
    let model_spec = format!("{provider}/{model}");
    format!(
        r#"# Agent configuration
# This file is self-contained — all settings are here.

agent_name = "{agent_name}"
models = ["{model_spec}"]
max_turns = 30
server_addr = "127.0.0.1:3000"

# max_tokens = 16384
# reasoning_effort = "low"  # low, medium, high
# compaction_model = "{model_spec}"
# timezone = "Europe/Rome"  # IANA format, used for cron scheduling
# followup_mode = "inject"  # inject or queue

# [memory]
# half_life_days = 30.0
# mmr_lambda = 0.7
# vector_search = true
# chunk_size = 1600
# chunk_overlap = 320
# bootstrap_file_cap = 20000
# export_sessions = true

# [compaction]
# context_budget = 160000
# summary_max_tokens = 4096
# min_keep_messages = 4

# [cron]
# default_max_retries = 3
# default_timeout_secs = 7200
# max_concurrent_runs = 1

# [attachments]
# enabled = true
# allowed_extensions = []
# max_file_size = 20971520

# [internet]
# enabled = true
# timeout_secs = 15
# max_fetch_bytes = 524288

# [channels.telegram]
# enabled = true
# gap_minutes = 360
# allowed_users = []
# stream_mode = "final_only"
"#,
    )
}

/// Generate default SOUL.md content for a new agent.
pub fn generate_soul(agent_name: &str) -> String {
    format!(
        "# Soul\n\n\
         You are {agent_name}, a personal AI assistant. You are helpful, direct, and thoughtful.\n\n\
         ## Core Traits\n\
         - You remember past conversations and learn from them\n\
         - You adapt your communication style to the user's preferences\n\
         - You are proactive about offering relevant information from memory\n\
         - You are honest about what you know and don't know\n\n\
         ## Communication Style\n\
         - Be concise but thorough when needed\n\
         - Use a friendly, professional tone\n\
         - Ask clarifying questions when the request is ambiguous\n\
         - Offer context from past conversations when relevant\n",
    )
}

/// Generate default frontend.toml for the web UI welcome screen.
pub fn generate_frontend_toml(agent_name: &str) -> String {
    format!(
        r#"# Frontend configuration for the web UI welcome screen.

# Greeting text shown below the logo (default: "ready_")
# greeting = "Hi! I'm {agent_name}."

# Suggested prompts shown as clickable chips
prompts = [
    "What can you help me with?",
    "What do you remember about me?",
]
"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_toml_is_valid_toml() {
        let config_str = generate_agent_toml("Nova", "anthropic", "claude-haiku-4-5");
        let val: toml::Value =
            toml::from_str(&config_str).expect("Generated agent config must be valid TOML");
        let table = val.as_table().unwrap();
        let models = table["models"].as_array().unwrap();
        assert_eq!(models[0].as_str(), Some("anthropic/claude-haiku-4-5"));
        assert_eq!(table["max_turns"].as_integer(), Some(30));
        assert_eq!(table["agent_name"].as_str(), Some("Nova"));
    }

    #[test]
    fn agent_toml_parses_as_agent_config() {
        let config_str = generate_agent_toml("Nova", "anthropic", "claude-haiku-4-5");
        let config: starpod_core::AgentConfig = toml::from_str(&config_str).unwrap();
        assert_eq!(config.models, vec!["anthropic/claude-haiku-4-5"]);
        assert_eq!(config.max_turns, 30);
        assert_eq!(config.server_addr, "127.0.0.1:3000");
    }

    #[test]
    fn custom_agent_toml_is_valid_toml() {
        let config_str = generate_agent_toml("MyBot", "openai", "gpt-4o");
        let val: toml::Value =
            toml::from_str(&config_str).expect("Generated agent config must be valid TOML");
        let table = val.as_table().unwrap();
        let models = table["models"].as_array().unwrap();
        assert_eq!(models[0].as_str(), Some("openai/gpt-4o"));
    }

    #[test]
    fn every_provider_generates_valid_toml() {
        for &provider in &providers() {
            let model = default_model(provider);
            let config_str = generate_agent_toml("Test", provider, model);
            let config: starpod_core::AgentConfig =
                toml::from_str(&config_str).unwrap_or_else(|e| {
                    panic!("Config for provider '{}' failed to parse: {}", provider, e)
                });
            assert_eq!(config.models, vec![format!("{provider}/{model}")]);
            assert_eq!(config.max_turns, 30);
            assert_eq!(config.server_addr, "127.0.0.1:3000");
        }
    }

    #[test]
    fn default_models_are_set() {
        assert_eq!(default_model("anthropic"), "claude-haiku-4-5");
        assert_eq!(default_model("openai"), "gpt-4o");
        assert_eq!(default_model("ollama"), "qwen3.5:9b");
    }

    #[test]
    fn every_provider_has_a_default_model() {
        for provider in &providers() {
            let model = default_model(provider);
            assert!(
                !model.is_empty(),
                "provider '{}' has no default model",
                provider
            );
        }
    }

    #[test]
    fn unknown_provider_falls_back() {
        assert_eq!(default_model("unknown"), "claude-haiku-4-5");
        assert_eq!(env_key_for_provider("unknown"), None);
    }

    #[test]
    fn env_keys_for_providers() {
        assert_eq!(env_key_for_provider("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(env_key_for_provider("openai"), Some("OPENAI_API_KEY"));
        assert_eq!(env_key_for_provider("ollama"), None);
    }

    #[test]
    fn providers_list_is_not_empty() {
        let p = providers();
        assert!(!p.is_empty());
        assert!(p.contains(&"anthropic"));
    }

    #[test]
    fn soul_contains_agent_name() {
        let soul = generate_soul("TestBot");
        assert!(soul.contains("TestBot"));
        assert!(soul.contains("# Soul"));
    }

    #[test]
    fn frontend_toml_is_valid_toml() {
        let content = generate_frontend_toml("Nova");
        let val: toml::Value =
            toml::from_str(&content).expect("Generated frontend.toml must be valid TOML");
        let table = val.as_table().unwrap();
        let prompts = table["prompts"].as_array().unwrap();
        assert_eq!(prompts.len(), 2);
    }

    #[test]
    fn frontend_toml_references_agent_name() {
        let content = generate_frontend_toml("Luna");
        assert!(
            content.contains("Luna"),
            "frontend.toml should reference agent name"
        );
    }

    #[test]
    fn agent_name_with_spaces() {
        let config_str = generate_agent_toml("My Cool Bot", "anthropic", "claude-haiku-4-5");
        let config: starpod_core::AgentConfig = toml::from_str(&config_str).unwrap();
        assert_eq!(config.agent_name, "My Cool Bot");
    }

    #[test]
    fn soul_structure_has_sections() {
        let soul = generate_soul("Nova");
        assert!(soul.contains("# Soul"));
        assert!(soul.contains("## Core Traits"));
        assert!(soul.contains("## Communication Style"));
    }

    #[test]
    fn agent_toml_has_all_commented_sections() {
        let content = generate_agent_toml("Test", "anthropic", "claude-haiku-4-5");
        for section in &[
            "[memory]",
            "[compaction]",
            "[cron]",
            "[attachments]",
            "[internet]",
            "[channels.telegram]",
        ] {
            assert!(
                content.contains(section),
                "agent.toml should contain commented {section}"
            );
        }
    }
}
