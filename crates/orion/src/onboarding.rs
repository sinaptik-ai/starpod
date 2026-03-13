//! Interactive onboarding wizard for `orion agent init`.

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
        "Welcome to Orion!".bright_white().bold(),
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
        .default("Orion".to_string())
        .interact_text()?;
    let agent_name = if agent_name == "Orion" {
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

/// Generate config.toml content from onboarding results.
pub fn generate_config(result: &OnboardingResult) -> String {
    let mut config = String::new();

    config.push_str("# Orion agent configuration\n");
    config.push_str("# See: https://github.com/gventuri/orion-rs\n\n");

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
    config.push_str("# Anthropic API key (or set ANTHROPIC_API_KEY env var)\n");
    config.push_str("# api_key = \"\"\n\n");
    config.push_str("# Reasoning effort for extended thinking: \"low\", \"medium\", \"high\"\n");
    config.push_str("# reasoning_effort = \"medium\"\n\n");

    // Identity
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n",
    );
    config.push_str("# AGENT IDENTITY\n");
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n\n",
    );
    config.push_str("[identity]\n");
    match &result.agent_name {
        Some(name) => config.push_str(&format!("name = \"{}\"\n", escape_toml(name))),
        None => config.push_str("name = \"Orion\"\n"),
    }
    config.push_str("# emoji = \"\"\n");
    match &result.agent_soul {
        Some(soul) => config.push_str(&format!("soul = \"{}\"\n", escape_toml(soul))),
        None => config.push_str("# soul = \"\"\n"),
    }
    config.push('\n');

    // User
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n",
    );
    config.push_str("# USER PROFILE\n");
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n\n",
    );
    config.push_str("[user]\n");
    match &result.user_name {
        Some(name) => config.push_str(&format!("name = \"{}\"\n", escape_toml(name))),
        None => config.push_str("# name = \"Your Name\"\n"),
    }
    match &result.timezone {
        Some(tz) => config.push_str(&format!("timezone = \"{}\"\n", tz)),
        None => config.push_str("# timezone = \"America/New_York\"\n"),
    }
    config.push('\n');

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

    // Telegram
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n",
    );
    config.push_str("# TELEGRAM\n");
    config.push_str(
        "# ══════════════════════════════════════════════════════════════════════════════\n\n",
    );
    config.push_str("[telegram]\n");
    match &result.telegram_token {
        Some(token) => config.push_str(&format!("bot_token = \"{}\"\n", escape_toml(token))),
        None => config.push_str("# bot_token = \"123456:ABC...\"     # Or set TELEGRAM_BOT_TOKEN env var\n"),
    }
    match &result.telegram_user_id {
        Some(uid) => config.push_str(&format!("allowed_users = [{}]\n", uid)),
        None => config.push_str("# allowed_users = [123456789]     # User IDs allowed to chat (empty = no one)\n"),
    }
    config.push_str("# stream_mode = \"off\"             # \"edit_in_place\" or \"off\"\n");
    config.push_str("# edit_throttle_ms = 300          # Min interval between streaming edits\n");

    config
}

/// Escape a string for TOML value (handles quotes and backslashes).
fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
