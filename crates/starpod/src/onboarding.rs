//! Interactive onboarding wizard for `starpod agent init`.

use colored::Colorize;
use dialoguer::{Confirm, Input, Select};

/// Collected answers from the onboarding wizard.
pub struct OnboardingResult {
    pub user_name: Option<String>,
    pub timezone: Option<String>,
    pub agent_name: Option<String>,
    pub agent_soul: Option<String>,
    pub telegram_token: Option<String>,
    pub telegram_user_id: Option<String>,
    pub provider: String,
    pub model: String,
}

const MODELS: &[(&str, &str)] = &[
    ("claude-sonnet-4-6", "Claude Sonnet 4.6 (recommended)"),
    ("claude-haiku-4-5", "Claude Haiku 4.5 (fast & cheap)"),
    ("claude-opus-4-6", "Claude Opus 4.6 (most capable)"),
];

/// Detect the system's IANA timezone (e.g. "Europe/Rome").
pub fn detect_system_timezone() -> Option<String> {
    iana_time_zone::get_timezone().ok()
}

/// Run the interactive onboarding wizard. Returns collected answers.
pub fn run() -> anyhow::Result<OnboardingResult> {
    println!();
    println!(
        "  {} {}",
        "Welcome to Starpod!".bright_white().bold(),
        "Let's set up your AI assistant.".dimmed()
    );
    println!();

    // ── User profile ──
    println!("  {}", "─── About you ───".dimmed());

    let user_name: String = Input::new()
        .with_prompt("  Your name")
        .allow_empty(true)
        .interact_text()?;
    let user_name = if user_name.is_empty() {
        None
    } else {
        Some(user_name)
    };

    // Auto-detect timezone, let user confirm or override
    let detected_tz = detect_system_timezone();
    let timezone = if let Some(ref tz) = detected_tz {
        let keep = Confirm::new()
            .with_prompt(format!("  Detected timezone: {}. Use it?", tz.bright_white()))
            .default(true)
            .interact()?;
        if keep {
            Some(tz.clone())
        } else {
            let custom: String = Input::new()
                .with_prompt("  Timezone (IANA format, e.g. Europe/Rome)")
                .allow_empty(true)
                .interact_text()?;
            if custom.is_empty() { None } else { Some(custom) }
        }
    } else {
        let custom: String = Input::new()
            .with_prompt("  Timezone (IANA format, e.g. Europe/Rome)")
            .allow_empty(true)
            .interact_text()?;
        if custom.is_empty() { None } else { Some(custom) }
    };

    println!();
    println!("  {}", "─── Agent personality ───".dimmed());

    let agent_name: String = Input::new()
        .with_prompt("  Agent name")
        .default("Aster".to_string())
        .interact_text()?;
    let agent_name = if agent_name == "Aster" {
        None
    } else {
        Some(agent_name)
    };

    let soul: String = Input::new()
        .with_prompt("  Personality (e.g. \"friendly and concise\")")
        .allow_empty(true)
        .interact_text()?;
    let agent_soul = if soul.is_empty() { None } else { Some(soul) };

    // ── Model ──
    println!();
    println!("  {}", "─── LLM model ───".dimmed());

    let model_items: Vec<String> = MODELS.iter().map(|(_, label)| label.to_string()).collect();

    let model_idx = Select::new()
        .with_prompt("  Model")
        .items(&model_items)
        .default(0)
        .interact()?;

    let model = MODELS[model_idx].0.to_string();

    // ── Telegram ──
    println!();
    println!("  {}", "─── Telegram ───".dimmed());

    let (telegram_token, telegram_user_id) = if Confirm::new()
        .with_prompt("  Set up Telegram bot?")
        .default(false)
        .interact()?
    {
        let token: String = Input::new()
            .with_prompt("  Bot token (from @BotFather)")
            .allow_empty(true)
            .interact_text()?;
        let user_id: String = Input::new()
            .with_prompt("  Your Telegram user ID (numeric)")
            .allow_empty(true)
            .interact_text()?;
        (
            if token.is_empty() { None } else { Some(token) },
            if user_id.is_empty() { None } else { Some(user_id) },
        )
    } else {
        (None, None)
    };

    Ok(OnboardingResult {
        user_name,
        timezone,
        agent_name,
        agent_soul,
        telegram_token,
        telegram_user_id,
        provider: "anthropic".to_string(),
        model,
    })
}

/// Generate config.toml content from onboarding results (shared config, no channels).
pub fn generate_config(result: &OnboardingResult) -> String {
    let mut config = String::new();

    config.push_str("# Starpod agent configuration (shared across instances)\n");
    config.push_str("# See: https://github.com/gventuri/starpod-rs\n");
    config.push_str("#\n");
    config.push_str("# Instance-specific settings (channels, overrides) go in instance.toml.\n");
    config.push_str("# Agent personality lives in .starpod/data/SOUL.md.\n");
    config.push_str("# User profile lives in .starpod/data/USER.md.\n\n");

    // General
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n",
    );
    config.push_str("# GENERAL\n");
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n\n",
    );

    config.push_str(&format!("provider = \"{}\"\n", result.provider));
    config.push_str(&format!("model = \"{}\"\n", result.model));
    config.push_str("max_turns = 30\n");
    config.push_str("server_addr = \"127.0.0.1:3000\"\n\n");

    // Agent name
    match &result.agent_name {
        Some(name) => config.push_str(&format!("agent_name = \"{}\"\n", escape_toml(name))),
        None => config.push_str("agent_name = \"Aster\"\n"),
    }
    // Timezone
    match &result.timezone {
        Some(tz) => config.push_str(&format!("timezone = \"{}\"\n", tz)),
        None => config.push_str("# timezone = \"America/New_York\"\n"),
    }
    config.push('\n');

    config.push_str("# Reasoning effort for extended thinking: \"low\", \"medium\", \"high\"\n");
    config.push_str("# reasoning_effort = \"medium\"\n\n");

    // Providers
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n",
    );
    config.push_str("# LLM PROVIDERS\n");
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n\n",
    );
    config.push_str("# [providers.anthropic]\n");
    config.push_str("# api_key = \"sk-ant-...\"                      # Or set ANTHROPIC_API_KEY env var\n\n");
    config.push_str("# [providers.openai]\n");
    config.push_str("# api_key = \"sk-...\"                          # Or set OPENAI_API_KEY env var\n");
    config.push_str("# models = [\"gpt-4o\", \"gpt-4o-mini\"]\n\n");

    // Attachments
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n",
    );
    config.push_str("# ATTACHMENTS\n");
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n\n",
    );
    config.push_str("[attachments]\n");
    config.push_str("# enabled = true                   # Set to false to disable attachments entirely\n");
    config.push_str("# allowed_extensions = []          # Allowed file extensions, e.g. [\"jpg\", \"png\", \"pdf\"]\n");
    config.push_str("#                                  # Empty list = all extensions allowed\n");
    config.push_str("# max_file_size = 20971520         # Max file size in bytes (default: 20 MB)\n");

    config
}

/// Generate instance.toml content from onboarding results (channels + instance overrides).
pub fn generate_instance_config(result: &OnboardingResult) -> String {
    let mut config = String::new();

    config.push_str("# Instance-specific configuration (overrides config.toml)\n");
    config.push_str("#\n");
    config.push_str("# This file can contain any setting from config.toml as an override,\n");
    config.push_str("# plus channel configurations which are ONLY valid here.\n\n");

    // Channels
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n",
    );
    config.push_str("# CHANNELS\n");
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n\n",
    );
    config.push_str("[channels.telegram]\n");
    match &result.telegram_token {
        Some(token) => config.push_str(&format!("bot_token = \"{}\"\n", escape_toml(token))),
        None => config.push_str("# bot_token = \"123456:ABC...\"     # Or set TELEGRAM_BOT_TOKEN env var\n"),
    }
    match &result.telegram_user_id {
        Some(uid) => config.push_str(&format!("allowed_users = [{}]\n", uid)),
        None => config.push_str("# allowed_users = [123456789]     # User IDs allowed to chat (empty = no one)\n"),
    }
    config.push_str("# stream_mode = \"final_only\"      # \"final_only\" or \"all_messages\"\n");

    config
}

/// Generate custom SOUL.md content from onboarding results.
/// Returns `None` if no custom name or soul was provided (default SOUL.md will be used).
pub fn generate_soul_md(result: &OnboardingResult) -> Option<String> {
    if result.agent_name.is_none() && result.agent_soul.is_none() {
        return None;
    }

    let name = result.agent_name.as_deref().unwrap_or("Aster");
    let mut content = format!("# Soul\n\nYou are {name}, a personal AI assistant.");

    if let Some(ref soul) = result.agent_soul {
        content.push_str(&format!(" {soul}"));
    } else {
        content.push_str(" You are helpful, direct, and thoughtful.");
    }

    content.push_str("\n\n## Core Traits\n\
        - You remember past conversations and learn from them\n\
        - You adapt your communication style to the user's preferences\n\
        - You are proactive about offering relevant information from memory\n\
        - You are honest about what you know and don't know\n\n\
        ## Communication Style\n\
        - Be concise but thorough when needed\n\
        - Use a friendly, professional tone\n\
        - Ask clarifying questions when the request is ambiguous\n\
        - Offer context from past conversations when relevant\n");

    Some(content)
}

/// Generate custom USER.md content from onboarding results.
/// Returns `None` if no user info was provided (default USER.md will be used).
pub fn generate_user_md(result: &OnboardingResult) -> Option<String> {
    if result.user_name.is_none() && result.timezone.is_none() {
        return None;
    }

    let mut content = String::from("# User Profile\n\n");
    if let Some(ref name) = result.user_name {
        content.push_str(&format!("- Name: {name}\n"));
    }
    if let Some(ref tz) = result.timezone {
        content.push_str(&format!("- Timezone: {tz}\n"));
    }

    Some(content)
}

/// Escape a string for TOML value (handles quotes and backslashes).
fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_result() -> OnboardingResult {
        OnboardingResult {
            user_name: None,
            timezone: None,
            agent_name: None,
            agent_soul: None,
            telegram_token: None,
            telegram_user_id: None,
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5".to_string(),
        }
    }

    // ── generate_config tests ───────────────────────────────────────

    #[test]
    fn config_contains_no_channels() {
        let config = generate_config(&default_result());
        assert!(!config.contains("[channels"));
        assert!(!config.contains("[telegram"));
        assert!(!config.contains("bot_token"));
    }

    #[test]
    fn config_contains_no_identity_or_user_section() {
        let config = generate_config(&default_result());
        assert!(!config.contains("[identity]"));
        assert!(!config.contains("[user]"));
    }

    #[test]
    fn config_contains_agent_name_at_top_level() {
        let config = generate_config(&default_result());
        assert!(config.contains("agent_name = \"Aster\""));
    }

    #[test]
    fn config_custom_agent_name() {
        let mut result = default_result();
        result.agent_name = Some("Nova".to_string());
        let config = generate_config(&result);
        assert!(config.contains("agent_name = \"Nova\""));
    }

    #[test]
    fn config_timezone_at_top_level() {
        let mut result = default_result();
        result.timezone = Some("Europe/Rome".to_string());
        let config = generate_config(&result);
        assert!(config.contains("timezone = \"Europe/Rome\""));
    }

    #[test]
    fn config_parses_as_valid_toml() {
        let mut result = default_result();
        result.agent_name = Some("TestBot".to_string());
        result.timezone = Some("UTC".to_string());
        let config = generate_config(&result);
        let parsed: starpod_core::StarpodConfig = toml::from_str(&config).unwrap();
        assert_eq!(parsed.agent_name, "TestBot");
        assert_eq!(parsed.timezone.as_deref(), Some("UTC"));
        assert_eq!(parsed.model, "claude-haiku-4-5");
    }

    // ── generate_instance_config tests ──────────────────────────────

    #[test]
    fn instance_config_contains_channels() {
        let config = generate_instance_config(&default_result());
        assert!(config.contains("[channels.telegram]"));
    }

    #[test]
    fn instance_config_with_telegram() {
        let mut result = default_result();
        result.telegram_token = Some("123:ABC".to_string());
        result.telegram_user_id = Some("999".to_string());
        let config = generate_instance_config(&result);
        assert!(config.contains("bot_token = \"123:ABC\""));
        assert!(config.contains("allowed_users = [999]"));
    }

    #[test]
    fn instance_config_parses_as_valid_toml() {
        let mut result = default_result();
        result.telegram_token = Some("123:ABC".to_string());
        result.telegram_user_id = Some("999".to_string());
        let config = generate_instance_config(&result);
        let parsed: starpod_core::StarpodConfig = toml::from_str(&config).unwrap();
        let tg = parsed.channels.telegram.unwrap();
        assert_eq!(tg.bot_token.as_deref(), Some("123:ABC"));
        assert_eq!(tg.allowed_user_ids(), vec![999]);
    }

    // ── generate_soul_md tests ──────────────────────────────────────

    #[test]
    fn soul_md_none_when_defaults() {
        assert!(generate_soul_md(&default_result()).is_none());
    }

    #[test]
    fn soul_md_with_custom_name() {
        let mut result = default_result();
        result.agent_name = Some("Nova".to_string());
        let soul = generate_soul_md(&result).unwrap();
        assert!(soul.contains("You are Nova"));
        assert!(soul.contains("# Soul"));
    }

    #[test]
    fn soul_md_with_custom_personality() {
        let mut result = default_result();
        result.agent_soul = Some("Be very concise.".to_string());
        let soul = generate_soul_md(&result).unwrap();
        assert!(soul.contains("You are Aster")); // default name
        assert!(soul.contains("Be very concise."));
    }

    #[test]
    fn soul_md_with_both() {
        let mut result = default_result();
        result.agent_name = Some("Jarvis".to_string());
        result.agent_soul = Some("You speak like a butler.".to_string());
        let soul = generate_soul_md(&result).unwrap();
        assert!(soul.contains("You are Jarvis"));
        assert!(soul.contains("You speak like a butler."));
    }

    // ── generate_user_md tests ──────────────────────────────────────

    #[test]
    fn user_md_none_when_defaults() {
        assert!(generate_user_md(&default_result()).is_none());
    }

    #[test]
    fn user_md_with_name() {
        let mut result = default_result();
        result.user_name = Some("Alice".to_string());
        let user = generate_user_md(&result).unwrap();
        assert!(user.contains("- Name: Alice"));
    }

    #[test]
    fn user_md_with_timezone() {
        let mut result = default_result();
        result.timezone = Some("US/Pacific".to_string());
        let user = generate_user_md(&result).unwrap();
        assert!(user.contains("- Timezone: US/Pacific"));
    }

    #[test]
    fn user_md_with_both() {
        let mut result = default_result();
        result.user_name = Some("Bob".to_string());
        result.timezone = Some("Europe/London".to_string());
        let user = generate_user_md(&result).unwrap();
        assert!(user.contains("- Name: Bob"));
        assert!(user.contains("- Timezone: Europe/London"));
    }

    // ── escape_toml tests ───────────────────────────────────────────

    #[test]
    fn escape_toml_handles_quotes() {
        assert_eq!(escape_toml(r#"say "hello""#), r#"say \"hello\""#);
    }

    #[test]
    fn escape_toml_handles_backslashes() {
        assert_eq!(escape_toml(r"path\to\file"), r"path\\to\\file");
    }
}
