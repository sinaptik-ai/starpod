//! Starpod CLI — personal AI assistant platform.
//!
//! Commands:
//! - `starpod init`  — bootstrap `.starpod/` in current directory
//! - `starpod dev`   — start local server (HTTP + WS + Telegram)
//! - `starpod serve` — production mode (no browser, no API key display)
//! - `starpod deploy` — deploy to remote (stub)
//! - `starpod repl`  — interactive terminal chat
//! - `starpod chat`  — one-shot message, print response, exit
//! - `starpod auth`  — login/logout/status for remote deployment

mod auth;
mod deploy;
mod onboarding;

use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use colored::Colorize;
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

use agent_sdk::{ContentBlock, Message};
use starpod_agent::StarpodAgent;
use starpod_core::{detect_mode, load_agent_config, ChatMessage, ResolvedPaths, StarpodConfig};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OutputFormat {
    #[default]
    Text,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!(
                "unknown format '{}' (expected 'text' or 'json')",
                other
            )),
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text => write!(f, "text"),
            Self::Json => write!(f, "json"),
        }
    }
}

#[derive(Parser)]
#[command(
    name = "starpod",
    about = "Starpod — personal AI assistant platform",
    version
)]
struct Cli {
    /// Output format: text (default) or json.
    #[arg(long, global = true, default_value = "text")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Bootstrap a new agent in the current directory.
    Init {
        /// Agent display name (default: "Nova").
        #[arg(long)]
        name: Option<String>,
        /// Model in provider/model format (default: "anthropic/claude-haiku-4-5").
        #[arg(long)]
        model: Option<String>,
        /// Seed environment variables into vault (repeatable: --env KEY=VAL).
        #[arg(long = "env", value_name = "KEY=VAL")]
        env_vars: Vec<String>,
    },

    /// Start the agent in development mode.
    Dev {
        /// Port to serve on (overrides config).
        #[arg(short, long)]
        port: Option<u16>,
    },

    /// Start the agent in production mode.
    Serve,

    /// Deploy the agent to a remote instance.
    Deploy {
        /// Instance display name.
        #[arg(long)]
        name: Option<String>,
        /// GCP zone (e.g. europe-west4-a).
        #[arg(long)]
        zone: Option<String>,
        /// Machine size: small, medium, large, xlarge.
        #[arg(long, default_value = "small")]
        machine_type: String,
        /// Skip interactive prompts (use defaults: secrets=yes, memory=yes, home=no).
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Start an interactive REPL session.
    Repl,

    /// Send a one-shot chat message.
    Chat {
        /// The message to send.
        message: String,
    },

    /// Authenticate with the Starpod backend.
    Auth {
        #[command(subcommand)]
        action: AuthCommand,
    },
}

// ── Auth subcommands ────────────────────────────────────────────────────────

#[derive(Subcommand)]
enum AuthCommand {
    /// Log in via the web UI (opens browser).
    Login {
        /// Backend URL (env: STARPOD_URL).
        #[arg(long)]
        url: Option<String>,
        /// API key for non-interactive login (CI/headless).
        #[arg(long)]
        api_key: Option<String>,
        /// Email to associate with the API key (used with --api-key).
        #[arg(long)]
        email: Option<String>,
    },
    /// Remove saved credentials.
    Logout,
    /// Show current authentication status.
    Status,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Strip optional markdown code fences from an AI response that should contain
/// raw JSON.  Handles ` ```json ... ``` `, bare ` ``` ... ``` `, and plain JSON.
#[cfg(test)]
fn strip_json_fence(raw: &str) -> &str {
    let s = raw.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    s.strip_suffix("```").unwrap_or(s).trim()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max])
    } else {
        s.to_string()
    }
}

fn tool_icon(name: &str) -> &str {
    match name {
        "Read" => "📄",
        "Grep" => "🔍",
        "Glob" => "📂",
        "Bash" => "⚡",
        "Edit" => "✏️",
        "Write" => "💾",
        "MemorySearch" => "🧠",
        "MemoryWrite" => "📝",
        "MemoryAppendDaily" => "📅",
        "EnvGet" => "🔑",
        "FileRead" => "📖",
        "FileWrite" => "📝",
        "FileList" => "📂",
        "FileDelete" => "🗑️",
        "SkillActivate" => "⚡",
        "SkillCreate" => "🛠️",
        "SkillUpdate" => "🛠️",
        "SkillDelete" => "🗑️",
        "SkillList" => "📋",
        "CronAdd" => "⏰",
        "CronList" => "📋",
        "CronRemove" => "🗑️",
        "CronRuns" => "📊",
        _ => "🔧",
    }
}

fn tool_input_preview(name: &str, input: &serde_json::Value) -> String {
    if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
        path.to_string()
    } else if let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) {
        pattern.to_string()
    } else if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
        truncate(cmd, 60)
    } else if let Some(q) = input.get("query").and_then(|v| v.as_str()) {
        truncate(q, 60)
    } else if let Some(key) = input.get("key").and_then(|v| v.as_str()) {
        if name == "EnvGet" {
            key.to_string()
        } else {
            let s = serde_json::to_string(input).unwrap_or_default();
            truncate(&s, 80)
        }
    } else if let Some(file) = input.get("file").and_then(|v| v.as_str()) {
        file.to_string()
    } else {
        let s = serde_json::to_string(input).unwrap_or_default();
        truncate(&s, 80)
    }
}

fn print_header_with_name(name: &str) {
    let label = format!("{}  ·  AI Assistant", name);
    let pad_total = 42_usize.saturating_sub(label.len());
    let pad_left = pad_total / 2;
    let pad_right = pad_total - pad_left;
    let inner = format!(
        "  │{}{}{} │",
        " ".repeat(pad_left),
        label,
        " ".repeat(pad_right)
    );
    println!();
    println!(
        "{}",
        "  ╭──────────────────────────────────────────╮".bright_cyan()
    );
    println!("{}", inner.bright_cyan());
    println!(
        "{}",
        "  ╰──────────────────────────────────────────╯".bright_cyan()
    );
    println!();
}

fn print_separator() {
    println!(
        "  {}",
        "─────────────────────────────────────────────".dimmed()
    );
}

/// Process the agent stream with rich output. Returns (result_text, ResultMessage).
async fn process_stream(
    stream: &mut agent_sdk::Query,
    start: &Instant,
) -> anyhow::Result<(String, Option<agent_sdk::ResultMessage>)> {
    let mut result_text = String::new();
    let mut result_msg = None;
    let mut turn = 0u32;

    while let Some(msg_result) = stream.next().await {
        let message = msg_result?;

        match &message {
            Message::System(sys) => {
                println!(
                    "  {} {}",
                    "●".bright_green(),
                    format!("Session {}", &sys.session_id[..8]).dimmed()
                );
                if let Some(ref model) = sys.model {
                    println!("  {} Model: {}", "│".dimmed(), model.bright_white());
                }
                if let Some(ref tools) = sys.tools {
                    println!("  {} Tools: {}", "│".dimmed(), tools.join(", ").dimmed());
                }
                print_separator();
            }
            Message::Assistant(assistant) => {
                turn += 1;
                let elapsed = start.elapsed().as_secs_f64();
                println!(
                    "\n  {} {}",
                    format!("Turn {turn}").bright_magenta().bold(),
                    format!("[{elapsed:.1}s]").dimmed()
                );

                for block in &assistant.content {
                    match block {
                        ContentBlock::Text { text } => {
                            if !text.trim().is_empty() {
                                if !result_text.is_empty() {
                                    result_text.push('\n');
                                }
                                result_text.push_str(text);
                            }
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            let icon = tool_icon(name);
                            let preview = tool_input_preview(name, input);
                            println!(
                                "  {} {} {}",
                                icon,
                                name.bright_blue().bold(),
                                preview.dimmed()
                            );
                        }
                        _ => {}
                    }
                }
            }
            Message::User(user) => {
                for block in &user.content {
                    if let ContentBlock::ToolResult {
                        content, is_error, ..
                    } = block
                    {
                        let result_str = content
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| serde_json::to_string(content).unwrap_or_default());

                        let lines: Vec<&str> = result_str.lines().collect();
                        let preview = if lines.len() > 3 {
                            format!(
                                "{}\n    {} {}",
                                lines[..3].join("\n    "),
                                "...".dimmed(),
                                format!("({} more lines)", lines.len() - 3).dimmed()
                            )
                        } else {
                            truncate(&result_str, 200)
                        };

                        if is_error == &Some(true) {
                            println!("    {} {}", "✗".red(), preview.red());
                        } else {
                            println!("    {} {}", "✓".green(), preview.dimmed());
                        }
                    }
                }
            }
            Message::Result(result) => {
                if result_text.is_empty() {
                    if let Some(text) = &result.result {
                        result_text = text.clone();
                    }
                }
                result_msg = Some(result.clone());
            }
            _ => {}
        }
    }

    Ok((result_text, result_msg))
}

fn print_result(result_text: &str, result_msg: &agent_sdk::ResultMessage, start: &Instant) {
    println!();
    print_separator();

    if result_msg.is_error {
        println!("  {} {}", "✗".red().bold(), "Error".red().bold());
        for err in &result_msg.errors {
            println!("    {}", err.red());
        }
    }

    if !result_text.is_empty() {
        println!();
        for line in result_text.lines() {
            println!("  {}", line);
        }
    }

    println!();
    print_separator();
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "  {} {:.1}s  {} {} turns  {} ${:.4}  {} {}in / {}out",
        "⏱".dimmed(),
        elapsed,
        "↻".dimmed(),
        result_msg.num_turns,
        "💰".dimmed(),
        result_msg.total_cost_usd,
        "📊".dimmed(),
        result_msg
            .usage
            .as_ref()
            .map(|u| format!("{}k", u.input_tokens / 1000))
            .unwrap_or_default()
            .bright_white(),
        result_msg
            .usage
            .as_ref()
            .map(|u| format!("{}k", u.output_tokens / 1000))
            .unwrap_or_default()
            .bright_white(),
    );
    print_separator();
}

// ── Agent resolution ────────────────────────────────────────────────────────

/// Resolve mode, paths, config, and build agent from CWD.
async fn resolve_agent() -> anyhow::Result<(StarpodAgent, StarpodConfig, ResolvedPaths)> {
    let mode = detect_mode(None)?;
    let paths = ResolvedPaths::resolve(&mode)?;
    let agent_config = load_agent_config(&paths)?;
    let starpod_config = agent_config.clone().into_starpod_config(&paths);
    let agent = StarpodAgent::with_paths(agent_config, paths.clone()).await?;
    Ok((agent, starpod_config, paths))
}

/// Read all secrets from vault and inject into process environment variables.
async fn inject_vault_env(paths: &ResolvedPaths, proxy_enabled: bool) -> anyhow::Result<()> {
    let vault_path = paths.db_dir.join("vault.db");
    if !vault_path.exists() {
        return Ok(());
    }
    let master_key = starpod_vault::derive_master_key(&paths.db_dir)?;
    let vault = starpod_vault::Vault::new(&vault_path, &master_key).await?;
    for key in vault.list_keys().await? {
        if let Some(val) = vault.get(&key, None).await? {
            #[cfg(feature = "secret-proxy")]
            if proxy_enabled {
                // System keys (ANTHROPIC_API_KEY, etc.) are consumed by the
                // Starpod process itself to call LLM APIs. They must never
                // be opaque-ified — only user-facing secrets get tokens.
                if !starpod_vault::is_system_key(&key) {
                    let entry = vault.get_entry(&key).await?.unwrap_or_else(|| {
                        starpod_vault::VaultEntry {
                            key: key.clone(),
                            is_secret: true,
                            allowed_hosts: None,
                            created_at: String::new(),
                            updated_at: String::new(),
                        }
                    });
                    if entry.is_secret {
                        let hosts = entry.allowed_hosts.unwrap_or_default();
                        let token = starpod_vault::opaque::encode_opaque_token(
                            vault.cipher(),
                            &val,
                            &hosts,
                        )?;
                        std::env::set_var(&key, &token);
                        continue;
                    }
                }
            }
            let _ = proxy_enabled; // suppress unused warning when feature disabled
            std::env::set_var(&key, &val);
        }
    }
    Ok(())
}

/// Build the cron notification sender from config.
fn build_cron_notifier(config: &StarpodConfig) -> Option<starpod_cron::NotificationSender> {
    let telegram_token = config.resolved_telegram_token();

    if let Some(ref token) = telegram_token {
        let token = token.clone();
        Some(Arc::new(
            move |_job_name, _session_id, result_text, _success| {
                let token = token.clone();
                Box::pin(async move {
                    tracing::debug!(
                        "Cron notification: {}",
                        &result_text[..result_text.len().min(100)]
                    );
                    let _ = token;
                })
            },
        ))
    } else {
        None
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let output_format = cli.format;

    match cli.command {
        // ── Init: bootstrap .starpod/ in CWD ─────────────────────────
        Commands::Init {
            name,
            model,
            env_vars,
        } => {
            let cwd = std::env::current_dir()?;
            let starpod_dir = cwd.join(".starpod");

            if starpod_dir.exists() {
                eprintln!(
                    "  {} Already initialized: {} exists.",
                    "✗".red().bold(),
                    ".starpod/".bright_white()
                );
                std::process::exit(1);
            }

            let agent_name = name.as_deref().unwrap_or("Nova");
            let model_spec = model.as_deref().unwrap_or("anthropic/claude-haiku-4-5");

            // Parse provider/model
            let (provider, model_name) = starpod_core::parse_model_spec(model_spec)
                .unwrap_or(("anthropic", "claude-haiku-4-5"));

            // Create directory structure
            let config_dir = starpod_dir.join("config");
            tokio::fs::create_dir_all(&config_dir).await?;
            tokio::fs::create_dir_all(starpod_dir.join("db")).await?;
            tokio::fs::create_dir_all(starpod_dir.join("skills")).await?;
            tokio::fs::create_dir_all(starpod_dir.join("users")).await?;

            // Write config files
            tokio::fs::write(
                config_dir.join("agent.toml"),
                onboarding::generate_agent_toml(agent_name, provider, model_name),
            )
            .await?;
            tokio::fs::write(
                config_dir.join("SOUL.md"),
                onboarding::generate_soul(agent_name),
            )
            .await?;
            tokio::fs::write(
                config_dir.join("frontend.toml"),
                onboarding::generate_frontend_toml(agent_name),
            )
            .await?;

            // Empty lifecycle files
            for filename in &["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md"] {
                tokio::fs::write(config_dir.join(filename), "").await?;
            }

            // Agent home directory (filesystem sandbox)
            for dir in &["desktop", "documents", "projects", "downloads"] {
                tokio::fs::create_dir_all(cwd.join("home").join(dir)).await?;
            }

            // Update .gitignore
            let gitignore_path = cwd.join(".gitignore");
            let mut gitignore_content = if gitignore_path.exists() {
                tokio::fs::read_to_string(&gitignore_path).await?
            } else {
                String::new()
            };
            let mut additions = Vec::new();
            if !gitignore_content.contains(".starpod/db/") {
                additions.push(".starpod/db/");
            }
            if !gitignore_content.contains("home/") {
                additions.push("home/");
            }
            if !additions.is_empty() {
                if !gitignore_content.is_empty() && !gitignore_content.ends_with('\n') {
                    gitignore_content.push('\n');
                }
                gitignore_content.push_str(&additions.join("\n"));
                gitignore_content.push('\n');
            }
            tokio::fs::write(&gitignore_path, gitignore_content).await?;

            // Seed vault with --env KEY=VAL pairs
            if !env_vars.is_empty() {
                let master_key = starpod_vault::derive_master_key(&starpod_dir.join("db"))?;
                let vault = starpod_vault::Vault::new(
                    &starpod_dir.join("db").join("vault.db"),
                    &master_key,
                )
                .await?;
                for kv in &env_vars {
                    if let Some((key, val)) = kv.split_once('=') {
                        vault.set(key.trim(), val.trim(), None).await?;
                    } else {
                        eprintln!("  {} Ignoring invalid --env value: {}", "⚠".yellow(), kv);
                    }
                }
            }

            println!();
            println!(
                "  {} Initialized agent '{}' in {}",
                "✓".green().bold(),
                agent_name.bright_white(),
                cwd.display()
            );
            println!(
                "  {} Run {} to start.",
                "→".dimmed(),
                "starpod dev".bright_white()
            );
            println!();
        }

        // ── Dev: start agent in development mode ─────────────────────
        Commands::Dev { port } => {
            let (agent, mut config, paths) = resolve_agent().await?;
            inject_vault_env(&paths, config.proxy.enabled).await?;

            // Override port if specified
            if let Some(p) = port {
                config.server_addr = format!("127.0.0.1:{}", p);
            }

            let addr = config.server_addr.clone();
            let display_name = config.agent_name.clone();
            let agent = Arc::new(agent);

            // Bootstrap auth store
            let auth_bootstrap = starpod_gateway::create_auth_store(&paths).await?;
            let auth_store = auth_bootstrap.store.clone();
            let cron_notifier = build_cron_notifier(&config);

            // Resolve API key for browser auto-login
            let browser_token = auth_bootstrap.admin_key.clone();

            print_header_with_name(&display_name);
            println!("  {} {}", "Server".dimmed(), addr.bright_green());
            if let Some(ref key) = browser_token {
                println!("  {} {}", "API Key".dimmed(), key.bright_cyan());
            }
            print_separator();

            // Open browser with auto-login token
            if let Some(ref key) = browser_token {
                let _ = open::that(format!("http://{}?token={}", addr, key));
            } else {
                let _ = open::that(format!("http://{}", addr));
            }

            starpod_gateway::serve_with_agent(
                agent,
                config,
                cron_notifier,
                paths,
                Some(auth_store),
            )
            .await?;
        }

        // ── Serve: production mode ──────────────────────────────────
        Commands::Serve => {
            let (agent, config, paths) = resolve_agent().await?;
            inject_vault_env(&paths, config.proxy.enabled).await?;

            let addr = config.server_addr.clone();
            let display_name = config.agent_name.clone();
            let agent = Arc::new(agent);
            let auth = starpod_gateway::create_auth_store(&paths)
                .await
                .ok()
                .map(|b| b.store);
            let cron_notifier = build_cron_notifier(&config);

            println!();
            println!(
                "  {} {}",
                display_name.bright_cyan().bold(),
                "is running".bright_white()
            );
            println!();
            println!(
                "  {} {}",
                "Frontend".dimmed(),
                format!("http://{}", addr).bright_white()
            );
            println!(
                "  {} {}",
                "API     ".dimmed(),
                format!("http://{}/api", addr).bright_white()
            );
            println!(
                "  {} {}",
                "WS      ".dimmed(),
                format!("ws://{}/ws", addr).bright_white()
            );
            println!();

            starpod_gateway::serve_with_agent(agent, config, cron_notifier, paths, auth).await?;
        }

        // ── Deploy ──────────────────────────────────────────────────
        Commands::Deploy {
            name,
            zone,
            machine_type,
            yes,
        } => {
            deploy::run_deploy(name, zone, machine_type, yes).await?;
        }

        // ── Repl ────────────────────────────────────────────────────
        Commands::Repl => {
            let (agent, config, paths) = resolve_agent().await?;
            inject_vault_env(&paths, config.proxy.enabled).await?;
            let name = config.agent_name.clone();
            run_repl(agent, &name).await?;
        }

        // ── Chat ────────────────────────────────────────────────────
        Commands::Chat { message } => {
            let _ephemeral_guard: Option<tempfile::TempDir>;
            let (agent, config, paths) = match resolve_agent().await {
                Ok((agent, config, paths)) => {
                    _ephemeral_guard = None;
                    (agent, config, paths)
                }
                Err(_) => {
                    let (tmp, paths) = starpod_core::create_ephemeral_instance()?;
                    let agent_config = starpod_core::load_agent_config(&paths)?;
                    let starpod_config = agent_config.clone().into_starpod_config(&paths);
                    let agent = StarpodAgent::with_paths(agent_config, paths.clone()).await?;
                    _ephemeral_guard = Some(tmp);
                    (agent, starpod_config, paths)
                }
            };
            inject_vault_env(&paths, config.proxy.enabled).await?;

            let name = config.agent_name.clone();
            print_header_with_name(&name);
            let start = Instant::now();

            let chat_msg = ChatMessage {
                text: message.clone(),
                user_id: None,
                channel_id: Some("main".into()),
                channel_session_key: Some(uuid::Uuid::new_v4().to_string()),
                attachments: Vec::new(),
                triggered_by: None,
                model: None,
            };
            let (mut stream, session_id, _followup_tx, _attachments) =
                agent.chat_stream(&chat_msg).await?;
            let (result_text, result_msg) = process_stream(&mut stream, &start).await?;

            if let Some(ref result) = result_msg {
                agent
                    .finalize_chat(&session_id, &message, &result_text, result, None)
                    .await;
                print_result(&result_text, result, &start);
            }
            println!();
        }

        // ── Auth ────────────────────────────────────────────────────
        Commands::Auth { action } => match action {
            AuthCommand::Login {
                url,
                api_key,
                email,
            } => {
                let spawner_url = url
                    .or_else(|| std::env::var(auth::SPAWNER_URL_ENV).ok())
                    .unwrap_or_else(|| auth::DEFAULT_SPAWNER_URL.to_string());

                if let Some(existing) = auth::load_credentials() {
                    println!(
                        "  {} Already logged in as {}",
                        "ℹ".bright_cyan(),
                        existing.email.bright_white()
                    );
                    println!(
                        "  {} Run {} to log out first.",
                        "→".dimmed(),
                        "starpod auth logout".bright_white()
                    );
                    return Ok(());
                }

                if let Some(key) = api_key {
                    let email = email.unwrap_or_else(|| "cli@starpod.local".to_string());
                    let creds = auth::Credentials {
                        backend_url: spawner_url.trim_end_matches('/').to_string(),
                        api_key: key,
                        email: email.clone(),
                    };
                    auth::save_credentials(&creds).map_err(|e| anyhow::anyhow!(e))?;
                    println!(
                        "  {} Authenticated as {} (API key)",
                        "✓".green().bold(),
                        email.bright_white()
                    );
                    println!(
                        "  {} Credentials saved to {}",
                        "→".dimmed(),
                        "~/.starpod/credentials.toml".bright_white()
                    );
                } else {
                    match auth::browser_login(&spawner_url).await {
                        Ok(creds) => {
                            println!();
                            println!(
                                "  {} Authenticated as {}",
                                "✓".green().bold(),
                                creds.email.bright_white()
                            );
                            println!(
                                "  {} Credentials saved to {}",
                                "→".dimmed(),
                                "~/.starpod/credentials.toml".bright_white()
                            );
                        }
                        Err(e) => {
                            eprintln!("  {} Login failed: {}", "✗".red().bold(), e);
                            std::process::exit(1);
                        }
                    }
                }
            }

            AuthCommand::Logout => match auth::delete_credentials() {
                Ok(()) => {
                    println!("  {} Logged out. Credentials removed.", "✓".green().bold());
                }
                Err(e) => {
                    eprintln!("  {} {}", "✗".red().bold(), e);
                    std::process::exit(1);
                }
            },

            AuthCommand::Status => match auth::load_credentials() {
                Some(creds) => {
                    if output_format == OutputFormat::Json {
                        let preview = if creds.api_key.len() > 12 {
                            format!(
                                "{}...{}",
                                &creds.api_key[..8],
                                &creds.api_key[creds.api_key.len() - 4..]
                            )
                        } else {
                            "****".to_string()
                        };
                        let json = serde_json::json!({
                            "logged_in": true,
                            "email": creds.email,
                            "backend_url": creds.backend_url,
                            "api_key_preview": preview,
                        });
                        println!("{}", serde_json::to_string_pretty(&json).unwrap());
                    } else {
                        println!("  {} Logged in", "✓".green().bold());
                        println!("  {} Email:   {}", "│".dimmed(), creds.email.bright_white());
                        println!("  {} Backend: {}", "│".dimmed(), creds.backend_url);
                        let preview = if creds.api_key.len() > 12 {
                            format!(
                                "{}...{}",
                                &creds.api_key[..8],
                                &creds.api_key[creds.api_key.len() - 4..]
                            )
                        } else {
                            "****".to_string()
                        };
                        println!("  {} API Key: {}", "│".dimmed(), preview);
                    }
                }
                None => {
                    if output_format == OutputFormat::Json {
                        let json = serde_json::json!({"logged_in": false});
                        println!("{}", serde_json::to_string_pretty(&json).unwrap());
                    } else {
                        println!("  {} Not logged in", "✗".yellow().bold());
                        println!(
                            "  {} Run {} to authenticate.",
                            "→".dimmed(),
                            "starpod auth login".bright_white()
                        );
                    }
                }
            },
        },
    }

    Ok(())
}

/// Interactive REPL mode with rich output.
async fn run_repl(agent: StarpodAgent, name: &str) -> anyhow::Result<()> {
    let session_key = uuid::Uuid::new_v4().to_string();

    print_header_with_name(name);
    println!(
        "  {} Type your message, or {} to quit.\n",
        "│".dimmed(),
        "'exit'".bright_yellow()
    );

    let mut rl = rustyline::DefaultEditor::new()?;

    loop {
        let prompt = format!("{} ", "you>".bright_green().bold());
        let line = match rl.readline(&prompt) {
            Ok(line) => line,
            Err(
                rustyline::error::ReadlineError::Interrupted | rustyline::error::ReadlineError::Eof,
            ) => {
                println!("\n  {} {}", "●".bright_yellow(), "Goodbye!".dimmed());
                break;
            }
            Err(e) => {
                eprintln!("Input error: {}", e);
                break;
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "exit" || line == "quit" {
            println!("\n  {} {}", "●".bright_yellow(), "Goodbye!".dimmed());
            break;
        }

        rl.add_history_entry(line)?;

        let start = Instant::now();
        let chat_msg = ChatMessage {
            text: line.to_string(),
            user_id: None,
            channel_id: Some("main".into()),
            channel_session_key: Some(session_key.clone()),
            attachments: Vec::new(),
            triggered_by: None,
            model: None,
        };
        let (mut stream, session_id, _followup_tx, _attachments) =
            agent.chat_stream(&chat_msg).await?;
        let (result_text, result_msg) = process_stream(&mut stream, &start).await?;

        if let Some(ref result) = result_msg {
            agent
                .finalize_chat(&session_id, line, &result_text, result, None)
                .await;
            print_result(&result_text, result, &start);
        } else if !result_text.is_empty() {
            println!("\n  {}\n", result_text);
        }

        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_json_fence_plain_json() {
        let input = r#"{"description": "hello", "body": "world"}"#;
        assert_eq!(strip_json_fence(input), input);
    }

    #[test]
    fn strip_json_fence_with_json_fence() {
        let input = "```json\n{\"description\": \"hello\"}\n```";
        assert_eq!(strip_json_fence(input), r#"{"description": "hello"}"#);
    }

    #[test]
    fn strip_json_fence_with_bare_fence() {
        let input = "```\n{\"body\": \"test\"}\n```";
        assert_eq!(strip_json_fence(input), r#"{"body": "test"}"#);
    }

    #[test]
    fn strip_json_fence_with_surrounding_whitespace() {
        let input = "  \n```json\n{\"a\": 1}\n```\n  ";
        assert_eq!(strip_json_fence(input), r#"{"a": 1}"#);
    }

    #[test]
    fn strip_json_fence_no_fence_with_whitespace() {
        let input = "  {\"a\": 1}  ";
        assert_eq!(strip_json_fence(input), r#"{"a": 1}"#);
    }

    #[test]
    fn strip_json_fence_multiline_body() {
        let input = "```json\n{\n  \"description\": \"d\",\n  \"body\": \"line1\\nline2\"\n}\n```";
        let result = strip_json_fence(input);
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
        let _: serde_json::Value = serde_json::from_str(result).unwrap();
    }

    #[test]
    fn strip_json_fence_empty_string() {
        assert_eq!(strip_json_fence(""), "");
    }

    #[test]
    fn strip_json_fence_only_fences() {
        assert_eq!(strip_json_fence("```json\n```"), "");
    }

    #[test]
    fn strip_json_fence_bare_fences_only() {
        assert_eq!(strip_json_fence("```\n```"), "");
    }

    #[test]
    fn strip_json_fence_preserves_inner_backticks() {
        let input = r#"{"body": "Use `code` here"}"#;
        assert_eq!(strip_json_fence(input), input);
    }

    #[test]
    fn strip_json_fence_with_trailing_newline_after_fence() {
        let input = "```json\n{\"a\": 1}\n```\n";
        assert_eq!(strip_json_fence(input), r#"{"a": 1}"#);
    }

    #[test]
    fn strip_json_fence_does_not_strip_partial_fence() {
        let input = "`{\"a\": 1}`";
        assert_eq!(strip_json_fence(input), input);
    }

    #[test]
    fn strip_json_fence_idempotent() {
        let input = r#"{"a": 1}"#;
        assert_eq!(strip_json_fence(strip_json_fence(input)), input);

        let fenced = "```json\n{\"a\": 1}\n```";
        let once = strip_json_fence(fenced);
        assert_eq!(strip_json_fence(once), once);
    }
}
