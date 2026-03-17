//! Workspace and agent scaffolding helpers.

/// Generate a default `starpod.toml` for a new workspace.
///
/// The generated config contains sensible defaults for provider, model,
/// max_turns, and server_addr. All values can be overridden per-agent
/// in each agent's `agent.toml`.
pub fn generate_workspace_config() -> String {
    r#"# Starpod workspace configuration
# These are defaults for all agents. Each agent can override in its own agent.toml.

provider = "anthropic"
model = "claude-sonnet-4-6"
max_turns = 30
server_addr = "127.0.0.1:3000"

# Provider API keys must be set in .env (e.g. ANTHROPIC_API_KEY=sk-ant-...)
# [providers.anthropic]
# base_url = "https://api.anthropic.com/v1/messages"

# [memory]
# half_life_days = 30.0
# vector_search = true
"#.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_config_is_valid_toml() {
        let config_str = generate_workspace_config();
        let val: toml::Value = toml::from_str(&config_str)
            .expect("Generated workspace config must be valid TOML");
        let table = val.as_table().unwrap();
        assert_eq!(table["provider"].as_str(), Some("anthropic"));
        assert_eq!(table["model"].as_str(), Some("claude-sonnet-4-6"));
        assert_eq!(table["max_turns"].as_integer(), Some(30));
    }

    #[test]
    fn workspace_config_parses_as_agent_config() {
        let config_str = generate_workspace_config();
        let config: starpod_core::AgentConfig = toml::from_str(&config_str).unwrap();
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.model, "claude-sonnet-4-6");
        assert_eq!(config.max_turns, 30);
        assert_eq!(config.server_addr, "127.0.0.1:3000");
    }
}
