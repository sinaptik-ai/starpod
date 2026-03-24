//! Workspace and agent scaffolding helpers.

use std::sync::LazyLock;

use agent_sdk::models::ModelRegistry;
use colored::Colorize;
use dialoguer::{Input, Password, Select, theme::ColorfulTheme};

/// Shared model registry (embedded defaults).
static REGISTRY: LazyLock<ModelRegistry> = LazyLock::new(ModelRegistry::with_defaults);

/// Provider choices available during init, derived from the model registry.
pub fn providers() -> Vec<&'static str> {
    REGISTRY.provider_names()
}

/// Default model for a provider, from the model registry.
pub fn default_model(provider: &str) -> &str {
    REGISTRY.default_model(provider).unwrap_or("claude-haiku-4-5")
}

/// Environment variable name for a provider's API key, from the model registry.
pub fn env_key_for_provider(provider: &str) -> Option<&str> {
    REGISTRY.api_key_env(provider)
}

/// Collected answers from the interactive wizard.
pub struct InitAnswers {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub brave_api_key: Option<String>,
    pub first_agent_name: Option<String>,
    pub agent_display_name: Option<String>,
}

/// Run the interactive init wizard. Returns `None` if the user cancels (Ctrl+C).
pub fn run_wizard() -> Option<InitAnswers> {
    let theme = ColorfulTheme::default();

    println!();
    println!(
        "  {} Welcome to {}!",
        "★".bright_yellow().bold(),
        "Starpod".bright_white().bold()
    );
    println!("  Let's set up your workspace.\n");

    // 1. Provider selection
    let provider_list = providers();
    let provider_idx = Select::with_theme(&theme)
        .with_prompt("Which LLM provider?")
        .items(&provider_list)
        .default(0)
        .interact_opt()
        .ok()
        .flatten()?;
    let provider = provider_list[provider_idx];

    // 2. Model (pre-filled with default for chosen provider)
    let default_model = default_model(provider);
    let model: String = Input::with_theme(&theme)
        .with_prompt("Model")
        .default(default_model.to_string())
        .interact_text()
        .ok()?;

    // 3. API key (if provider needs one)
    let api_key = if let Some(env_name) = env_key_for_provider(provider) {
        // Check if already set in environment
        if std::env::var(env_name).is_ok() {
            println!(
                "  {} {} is already set in your environment.",
                "✓".green().bold(),
                env_name.bright_white()
            );
            None
        } else {
            let key: String = Password::with_theme(&theme)
                .with_prompt(format!("{} (will be saved to .env)", env_name))
                .allow_empty_password(true)
                .interact()
                .ok()?;
            if key.is_empty() { None } else { Some(key) }
        }
    } else {
        None
    };

    // 4. Brave Search API key (optional — enables web search)
    let brave_api_key = if std::env::var("BRAVE_API_KEY").is_ok() {
        println!(
            "  {} {} is already set in your environment.",
            "✓".green().bold(),
            "BRAVE_API_KEY".bright_white()
        );
        None
    } else {
        let enable_web = Select::with_theme(&theme)
            .with_prompt("Enable web search? (requires a free Brave Search API key)")
            .items(&["Yes", "No, skip for now"])
            .default(0)
            .interact_opt()
            .ok()
            .flatten()?;

        if enable_web == 0 {
            println!(
                "  {} Get a free API key at {}",
                "→".bright_blue().bold(),
                "https://brave.com/search/api/".bright_white()
            );
            let key: String = Password::with_theme(&theme)
                .with_prompt("BRAVE_API_KEY (will be saved to .env)")
                .allow_empty_password(true)
                .interact()
                .ok()?;
            if key.is_empty() { None } else { Some(key) }
        } else {
            None
        }
    };

    // 5. Create first agent?
    let create_agent = Select::with_theme(&theme)
        .with_prompt("Create your first agent now?")
        .items(&["Yes", "No, I'll do it later"])
        .default(0)
        .interact_opt()
        .ok()
        .flatten()?;

    let (first_agent_name, agent_display_name) = if create_agent == 0 {
        let name: String = Input::with_theme(&theme)
            .with_prompt("Agent slug (lowercase, hyphens)")
            .default("my-agent".to_string())
            .interact_text()
            .ok()?;
        let display: String = Input::with_theme(&theme)
            .with_prompt("Agent display name")
            .default("Aster".to_string())
            .interact_text()
            .ok()?;
        (Some(name), Some(display))
    } else {
        (None, None)
    };

    Some(InitAnswers {
        provider: provider.to_string(),
        model,
        api_key,
        brave_api_key,
        first_agent_name,
        agent_display_name,
    })
}

/// Generate `starpod.toml` with the given provider and model.
pub fn generate_workspace_config_with(provider: &str, model: &str) -> String {
    format!(
        r#"# Starpod workspace configuration
# This is a template — values here are baked into each agent's agent.toml when created.
# It is NOT read at runtime. Edit each agent's agent.toml directly to change settings.

models = ["{provider}/{model}"]
max_turns = 30
# max_tokens = 16384
server_addr = "127.0.0.1:3000"
# reasoning_effort = "low"  # low, medium, high (for models with extended thinking)
# compaction_model = "{provider}/{model}"  # model used for conversation compaction summaries (provider/model format)
# followup_mode = "inject"  # inject = merge into running loop, queue = run after current loop

# Provider API keys must be set in .env (e.g. ANTHROPIC_API_KEY=sk-ant-...)
# [providers.{provider}]
# enabled = true
# base_url = "https://..."
# models = []  # preferred models shown first

# [memory]
# half_life_days = 30.0
# mmr_lambda = 0.7  # 0.0 = max diversity, 1.0 = pure relevance
# vector_search = true
# chunk_size = 1600  # ~400 tokens
# chunk_overlap = 320  # ~80 tokens
# bootstrap_file_cap = 20000  # max chars from a single file in bootstrap context
# export_sessions = true  # export closed sessions for long-term recall

# [compaction]
# context_budget = 160000  # token budget triggering compaction
# summary_max_tokens = 4096
# min_keep_messages = 4

# [cron]
# default_max_retries = 3
# default_timeout_secs = 7200  # 2 hours
# max_concurrent_runs = 1

# [attachments]
# enabled = true
# allowed_extensions = []  # empty = all allowed, e.g. ["jpg", "png", "pdf"]
# max_file_size = 20971520  # 20 MB

# [internet]
# enabled = true  # enable WebSearch and WebFetch tools
# timeout_secs = 15  # request timeout for web operations
# max_fetch_bytes = 524288  # max response body size (512 KiB)
# BRAVE_API_KEY must be set in .env for web search to work
"#
    )
}

/// Generate a default `starpod.toml` for a new workspace (used by `--default` path and tests).
#[cfg(test)]
fn generate_workspace_config() -> String {
    generate_workspace_config_with("anthropic", "claude-haiku-4-5")
}

/// Generate the `.env` content for the selected provider.
#[cfg(test)]
fn generate_env_content(provider: &str, api_key: Option<&str>) -> String {
    generate_env_content_full(provider, api_key, None)
}

/// Generate the `.env` content including the Brave Search API key.
pub fn generate_env_content_full(
    provider: &str,
    api_key: Option<&str>,
    brave_api_key: Option<&str>,
) -> String {
    let mut out = if let (Some(env_name), Some(key)) = (env_key_for_provider(provider), api_key) {
        format!("{}={}\n", env_name, key)
    } else if let Some(env_name) = env_key_for_provider(provider) {
        format!("# {}=your-key-here\n", env_name)
    } else {
        "# No API key needed for this provider.\n".to_string()
    };

    match brave_api_key {
        Some(key) if !key.is_empty() => {
            out.push_str(&format!("BRAVE_API_KEY={}\n", key));
        }
        _ => {
            out.push_str("# BRAVE_API_KEY=your-brave-key-here\n");
        }
    }

    out
}


#[cfg(test)]
mod tests {
    use super::*;

    // ── Config generation ────────────────────────────────────────────

    #[test]
    fn workspace_config_is_valid_toml() {
        let config_str = generate_workspace_config();
        let val: toml::Value = toml::from_str(&config_str)
            .expect("Generated workspace config must be valid TOML");
        let table = val.as_table().unwrap();
        let models = table["models"].as_array().unwrap();
        assert_eq!(models[0].as_str(), Some("anthropic/claude-haiku-4-5"));
        assert_eq!(table["max_turns"].as_integer(), Some(30));
    }

    #[test]
    fn workspace_config_parses_as_agent_config() {
        let config_str = generate_workspace_config();
        let config: starpod_core::AgentConfig = toml::from_str(&config_str).unwrap();
        assert_eq!(config.models, vec!["anthropic/claude-haiku-4-5"]);
        assert_eq!(config.max_turns, 30);
        assert_eq!(config.server_addr, "127.0.0.1:3000");
    }

    #[test]
    fn custom_workspace_config_is_valid_toml() {
        let config_str = generate_workspace_config_with("openai", "gpt-4o");
        let val: toml::Value = toml::from_str(&config_str)
            .expect("Generated workspace config must be valid TOML");
        let table = val.as_table().unwrap();
        let models = table["models"].as_array().unwrap();
        assert_eq!(models[0].as_str(), Some("openai/gpt-4o"));
    }

    #[test]
    fn every_provider_generates_valid_toml() {
        for &provider in &providers() {
            let model = default_model(provider);
            let config_str = generate_workspace_config_with(provider, model);
            let config: starpod_core::AgentConfig = toml::from_str(&config_str)
                .unwrap_or_else(|e| panic!("Config for provider '{}' failed to parse: {}", provider, e));
            assert_eq!(config.models, vec![format!("{provider}/{model}")]);
            assert_eq!(config.max_turns, 30);
            assert_eq!(config.server_addr, "127.0.0.1:3000");
        }
    }

    // ── .env generation ──────────────────────────────────────────────

    #[test]
    fn env_content_with_key() {
        let env = generate_env_content("anthropic", Some("sk-ant-test123"));
        assert!(env.contains("ANTHROPIC_API_KEY=sk-ant-test123"));
    }

    #[test]
    fn env_content_without_key() {
        let env = generate_env_content("anthropic", None);
        assert!(env.starts_with("# ANTHROPIC_API_KEY="));
    }

    #[test]
    fn env_content_ollama() {
        let env = generate_env_content("ollama", None);
        assert!(env.contains("No API key needed"));
    }

    #[test]
    fn env_content_ollama_with_key_still_no_env_var() {
        // ollama has no env key, so even passing a key produces the "no key needed" message
        let env = generate_env_content("ollama", Some("ignored"));
        assert!(env.contains("No API key needed"));
    }

    #[test]
    fn env_content_every_keyed_provider() {
        let keyed = ["anthropic", "openai", "gemini", "groq", "deepseek", "openrouter"];
        for provider in keyed {
            let env_name = env_key_for_provider(provider).unwrap();
            // with key
            let env = generate_env_content(provider, Some("test-key"));
            assert!(env.contains(&format!("{}=test-key", env_name)), "provider: {}", provider);
            // without key
            let env = generate_env_content(provider, None);
            assert!(env.starts_with(&format!("# {}=", env_name)), "provider: {}", provider);
        }
    }

    // ── Default models ───────────────────────────────────────────────

    #[test]
    fn default_models_are_set() {
        assert_eq!(default_model("anthropic"), "claude-haiku-4-5");
        assert_eq!(default_model("openai"), "gpt-4o");
        assert_eq!(default_model("ollama"), "llama3.3");
    }

    #[test]
    fn every_provider_has_a_default_model() {
        for provider in &providers() {
            let model = default_model(provider);
            assert!(!model.is_empty(), "provider '{}' has no default model", provider);
        }
    }

    #[test]
    fn unknown_provider_falls_back() {
        assert_eq!(default_model("unknown"), "claude-haiku-4-5");
        assert_eq!(env_key_for_provider("unknown"), None);
    }

    // ── Env keys ─────────────────────────────────────────────────────

    #[test]
    fn env_keys_for_providers() {
        assert_eq!(env_key_for_provider("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(env_key_for_provider("openai"), Some("OPENAI_API_KEY"));
        assert_eq!(env_key_for_provider("gemini"), Some("GEMINI_API_KEY"));
        assert_eq!(env_key_for_provider("groq"), Some("GROQ_API_KEY"));
        assert_eq!(env_key_for_provider("deepseek"), Some("DEEPSEEK_API_KEY"));
        assert_eq!(env_key_for_provider("openrouter"), Some("OPENROUTER_API_KEY"));
        assert_eq!(env_key_for_provider("ollama"), None);
    }

    // ── Full env generation ───────────────────────────────────────────

    #[test]
    fn env_content_full_with_brave_key() {
        let env = generate_env_content_full("anthropic", Some("sk-ant-123"), Some("BSA-key-456"));
        assert!(env.contains("ANTHROPIC_API_KEY=sk-ant-123"));
        assert!(env.contains("BRAVE_API_KEY=BSA-key-456"));
        assert!(!env.contains("# BRAVE_API_KEY="));
    }

    #[test]
    fn env_content_full_without_brave_key() {
        let env = generate_env_content_full("anthropic", Some("sk-ant-123"), None);
        assert!(env.contains("ANTHROPIC_API_KEY=sk-ant-123"));
        assert!(env.contains("# BRAVE_API_KEY=your-brave-key-here"));
    }

    // ── Provider list ────────────────────────────────────────────────

    #[test]
    fn providers_list_is_not_empty() {
        let p = providers();
        assert!(!p.is_empty());
        assert!(p.contains(&"anthropic"));
    }
}
