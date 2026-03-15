mod onboarding;

use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use colored::Colorize;
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

use agent_sdk::{ContentBlock, Message};
use starpod_agent::StarpodAgent;
use starpod_core::{ChatMessage, StarpodConfig};
use starpod_instances::InstanceClient;

#[derive(Parser)]
#[command(name = "starpod", about = "Starpod — personal AI assistant platform", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Agent management — init, serve, chat, repl.
    Agent {
        #[command(subcommand)]
        action: AgentCommand,
    },

    /// Remote instance management — create, list, kill, pause, restart, logs, ssh, health.
    Instance {
        #[command(subcommand)]
        action: InstanceCommand,
    },
}

// ── Agent subcommands ──────────────────────────────────────────────────────

#[derive(Subcommand)]
enum AgentCommand {
    /// Initialize a new Starpod project in the current directory.
    Init {
        /// Skip the interactive wizard and use defaults.
        #[arg(long, alias = "skip-onboarding")]
        default: bool,

        /// Your name (used in conversations).
        #[arg(long)]
        name: Option<String>,

        /// IANA timezone (e.g. "Europe/Rome"). Auto-detected if omitted.
        #[arg(long)]
        timezone: Option<String>,

        /// Agent display name.
        #[arg(long, default_value = "Aster")]
        agent_name: String,

        /// Agent personality text.
        #[arg(long)]
        soul: Option<String>,

        /// Claude model to use.
        #[arg(long, default_value = "claude-sonnet-4-6")]
        model: String,
    },

    /// Start the gateway HTTP/WS server (+ Telegram bot if configured).
    Serve,

    /// Send a one-shot chat message.
    Chat {
        /// The message to send.
        message: String,
    },

    /// Start an interactive REPL session.
    Repl,

    // ── Utility subcommands ──

    /// Memory management commands.
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Vault credential management.
    Vault {
        #[command(subcommand)]
        action: VaultAction,
    },

    /// Session management.
    Sessions {
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Skill management.
    Skills {
        #[command(subcommand)]
        action: SkillAction,
    },

    /// Cron job management.
    Cron {
        #[command(subcommand)]
        action: CronAction,
    },
}

// ── Instance subcommands ────────────────────────────────────────────────────

#[derive(Subcommand)]
enum InstanceCommand {
    /// Create a new remote instance.
    Create {
        /// Instance name.
        #[arg(short, long)]
        name: Option<String>,
        /// Cloud region.
        #[arg(short, long)]
        region: Option<String>,
    },
    /// List running instances.
    List,
    /// Kill a running instance.
    Kill {
        /// Instance ID.
        id: String,
    },
    /// Pause a running instance.
    Pause {
        /// Instance ID.
        id: String,
    },
    /// Restart a paused or running instance.
    Restart {
        /// Instance ID.
        id: String,
    },
    /// Stream logs from a running instance.
    Logs {
        /// Instance ID.
        id: String,
        /// Number of recent log lines to fetch first.
        #[arg(short, long, default_value = "50")]
        tail: usize,
    },
    /// Open an SSH shell into a remote instance.
    Ssh {
        /// Instance ID.
        id: String,
    },
    /// Show health / resource usage for an instance.
    Health {
        /// Instance ID.
        id: String,
    },
}

// ── Utility subcommands ────────────────────────────────────────────────────

#[derive(Subcommand)]
enum MemoryAction {
    /// Search memory with a query.
    Search {
        /// The search query.
        query: String,
        /// Maximum results.
        #[arg(short, long, default_value = "5")]
        limit: usize,
    },
    /// Rebuild the FTS index.
    Reindex,
}

#[derive(Subcommand)]
enum VaultAction {
    /// Get a credential.
    Get {
        /// The key to retrieve.
        key: String,
    },
    /// Set a credential.
    Set {
        /// The key to store.
        key: String,
        /// The value to encrypt and store.
        value: String,
    },
    /// Delete a credential.
    Delete {
        /// The key to delete.
        key: String,
    },
    /// List all keys.
    List,
}

#[derive(Subcommand)]
enum SessionAction {
    /// List recent sessions.
    List {
        /// Maximum sessions to show.
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum SkillAction {
    /// List all skills.
    List,
    /// Show a skill's content.
    Show {
        /// Skill name.
        name: String,
    },
    /// Create a new AgentSkills-compatible skill.
    Create {
        /// Skill name (lowercase, hyphens, e.g. 'code-review').
        name: String,
        /// Description of what the skill does and when to use it.
        #[arg(short, long)]
        description: String,
        /// Markdown instructions (or use --file for the body).
        #[arg(short, long)]
        body: Option<String>,
        /// Read instructions from a file.
        #[arg(short, long)]
        file: Option<String>,
    },
    /// Delete a skill.
    Delete {
        /// Skill name.
        name: String,
    },
}

#[derive(Subcommand)]
enum CronAction {
    /// List all cron jobs.
    List,
    /// Remove a cron job by name.
    Remove {
        /// Job name.
        name: String,
    },
    /// Show recent runs for a job.
    Runs {
        /// Job name.
        name: String,
        /// Maximum runs to show.
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Trigger a cron job immediately.
    Run {
        /// Job name.
        name: String,
    },
    /// Edit a cron job's properties.
    Edit {
        /// Job name.
        name: String,
        /// New prompt.
        #[arg(long)]
        prompt: Option<String>,
        /// New schedule (cron expression).
        #[arg(long)]
        schedule: Option<String>,
        /// Enable or disable the job.
        #[arg(long)]
        enabled: Option<bool>,
        /// Max retries on failure.
        #[arg(long)]
        max_retries: Option<u32>,
        /// Timeout in seconds.
        #[arg(long)]
        timeout_secs: Option<u32>,
        /// Session mode: isolated or main.
        #[arg(long)]
        session_mode: Option<String>,
    },
}

// ── Helpers ────────────────────────────────────────────────────────────────

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
        "VaultGet" => "🔐",
        "VaultSet" => "🔑",
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
        if name.starts_with("Vault") {
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
                            .unwrap_or_else(|| {
                                serde_json::to_string(content).unwrap_or_default()
                            });

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

// ── Main ───────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        // ── Agent commands ─────────────────────────────────────────────
        Commands::Agent { action } => match action {
            AgentCommand::Init {
                default,
                name,
                timezone,
                agent_name,
                soul,
                model,
            } => {
                let cwd = std::env::current_dir()?;

                // Check if already initialized — show a friendly error
                if cwd.join(".starpod").exists() {
                    eprintln!(
                        "  {} Already initialized: {} exists.",
                        "✗".red().bold(),
                        ".starpod/".bright_white()
                    );
                    eprintln!(
                        "  {} Edit {} to reconfigure.",
                        "→".dimmed(),
                        ".starpod/config.toml".bright_white()
                    );
                    std::process::exit(1);
                }

                // Check if any CLI params were explicitly provided
                let has_cli_params = name.is_some()
                    || timezone.is_some()
                    || agent_name != "Aster"
                    || soul.is_some()
                    || model != "claude-sonnet-4-6";

                let config_content = if default || has_cli_params {
                    // Non-interactive: build from CLI args
                    let tz = timezone.or_else(onboarding::detect_system_timezone);
                    let result = onboarding::OnboardingResult {
                        user_name: name,
                        timezone: tz,
                        agent_name: if agent_name == "Aster" {
                            None
                        } else {
                            Some(agent_name)
                        },
                        agent_soul: soul,
                        telegram_token: None,
                        telegram_user_id: None,
                        provider: "anthropic".to_string(),
                        model,
                    };
                    Some(onboarding::generate_config(&result))
                } else {
                    // Interactive onboarding wizard
                    match onboarding::run() {
                        Ok(result) => Some(onboarding::generate_config(&result)),
                        Err(_) => {
                            println!(
                                "\n  {} Using default configuration.",
                                "→".dimmed()
                            );
                            None
                        }
                    }
                };

                StarpodConfig::init(&cwd, config_content.as_deref()).await?;

                println!();
                println!(
                    "  {} Initialized Starpod project in {}",
                    "✓".green().bold(),
                    cwd.join(".starpod").display()
                );
                println!(
                    "  {} Edit {} to fine-tune your config.",
                    "→".dimmed(),
                    ".starpod/config.toml".bright_white()
                );
                println!(
                    "  {} Run {} to start your agent.",
                    "→".dimmed(),
                    "starpod agent serve".bright_white()
                );
                println!(
                    "  {} Docs available at {} when serving.",
                    "→".dimmed(),
                    "/docs".bright_white()
                );
                println!();
            }

            AgentCommand::Serve => {
                let config = StarpodConfig::load().await?;
                let addr = &config.server_addr;
                let agent_name = config.identity.display_name().to_string();
                let agent = Arc::new(StarpodAgent::new(config.clone()).await?);

                // Start Telegram bot in background if token is configured
                let telegram_token = config.resolved_telegram_token();
                let telegram_allowed = config.resolved_telegram_allowed_user_ids();
                let telegram_allowed_usernames = config.resolved_telegram_allowed_usernames();

                let telegram_active = telegram_token.is_some();

                // Build a cron notification sender for Telegram (if configured)
                let cron_notifier: Option<starpod_cron::NotificationSender> =
                    if let Some(ref token) = telegram_token {
                        if !telegram_allowed.is_empty() {
                            let token = token.clone();
                            let users = telegram_allowed.clone();
                            Some(Arc::new(move |_job_name, result_text, _success| {
                                let token = token.clone();
                                let users = users.clone();
                                Box::pin(async move {
                                    starpod_telegram::send_notification(&token, &users, &result_text).await;
                                })
                            }))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                if let Some(token) = telegram_token.clone() {
                    let tg_agent = Arc::clone(&agent);
                    let allowed = telegram_allowed.clone();
                    let allowed_names = telegram_allowed_usernames.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            starpod_telegram::run_with_agent_filtered(tg_agent, token, allowed, allowed_names).await
                        {
                            tracing::error!(error = %e, "Telegram bot error");
                        }
                    });
                }

                // Print startup banner
                println!();
                println!(
                    "  {} {}",
                    agent_name.bright_cyan().bold(),
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
                println!(
                    "  {} {}",
                    "Docs    ".dimmed(),
                    format!("http://{}/docs", addr).bright_white()
                );
                println!(
                    "  {} {}",
                    "Telegram".dimmed(),
                    if telegram_active {
                        let mode = &config.telegram.stream_mode;
                        format!("connected (stream: {})", mode).green().to_string()
                    } else {
                        "not configured".yellow().to_string()
                    }
                );
                println!(
                    "  {} {}",
                    "Provider".dimmed(),
                    config.provider.bright_white()
                );
                println!(
                    "  {} {}",
                    "Model   ".dimmed(),
                    config.model.bright_white()
                );
                if let Some(ref effort) = config.reasoning_effort {
                    println!(
                        "  {} {:?}",
                        "Thinking".dimmed(),
                        effort
                    );
                }
                println!(
                    "  {} {}",
                    "Project ".dimmed(),
                    config.project_root.display().to_string().bright_white()
                );
                println!();

                starpod_gateway::serve_with_agent(agent, config, cron_notifier).await?;
            }

            AgentCommand::Chat { message } => {
                let config = StarpodConfig::load().await?;
                let name = config.identity.display_name().to_string();
                print_header_with_name(&name);
                let start = Instant::now();
                let agent = StarpodAgent::new(config).await?;

                let chat_msg = ChatMessage {
                    text: message.clone(),
                    user_id: None,
                    channel_id: Some("main".into()),
                    channel_session_key: Some(uuid::Uuid::new_v4().to_string()),
                    attachments: Vec::new(),
                };
                let (mut stream, session_id, _followup_tx) = agent.chat_stream(&chat_msg).await?;
                let (result_text, result_msg) = process_stream(&mut stream, &start).await?;

                if let Some(ref result) = result_msg {
                    agent
                        .finalize_chat(&session_id, &message, &result_text, result)
                        .await;
                    print_result(&result_text, result, &start);
                }
                println!();
            }

            AgentCommand::Repl => {
                let config = StarpodConfig::load().await?;
                let name = config.identity.display_name().to_string();
                run_repl(config, &name).await?;
            }

            // ── Utility commands ──────────────────────────────────────
            AgentCommand::Memory { action } => {
                let config = StarpodConfig::load().await?;
                let agent = StarpodAgent::new(config).await?;
                match action {
                    MemoryAction::Search { query, limit } => {
                        let results = agent.memory().search(&query, limit).await?;
                        if results.is_empty() {
                            println!("No results found.");
                        } else {
                            for (i, r) in results.iter().enumerate() {
                                println!(
                                    "--- [{}/{}] {} (lines {}-{}) ---",
                                    i + 1,
                                    results.len(),
                                    r.source,
                                    r.line_start,
                                    r.line_end
                                );
                                println!("{}\n", r.text);
                            }
                        }
                    }
                    MemoryAction::Reindex => {
                        agent.memory().reindex().await?;
                        println!("Memory index rebuilt.");
                    }
                }
            }

            AgentCommand::Vault { action } => {
                let config = StarpodConfig::load().await?;
                let agent = StarpodAgent::new(config).await?;
                match action {
                    VaultAction::Get { key } => match agent.vault().get(&key).await? {
                        Some(value) => println!("{}", value),
                        None => println!("No value found for key: {}", key),
                    },
                    VaultAction::Set { key, value } => {
                        agent.vault().set(&key, &value).await?;
                        println!("Stored '{}'.", key);
                    }
                    VaultAction::Delete { key } => {
                        agent.vault().delete(&key).await?;
                        println!("Deleted '{}'.", key);
                    }
                    VaultAction::List => {
                        let keys = agent.vault().list_keys().await?;
                        if keys.is_empty() {
                            println!("Vault is empty.");
                        } else {
                            for key in &keys {
                                println!("  {}", key);
                            }
                        }
                    }
                }
            }

            AgentCommand::Sessions { action } => {
                let config = StarpodConfig::load().await?;
                let agent = StarpodAgent::new(config).await?;
                match action {
                    SessionAction::List { limit } => {
                        let sessions = agent.session_mgr().list_sessions(limit).await?;
                        if sessions.is_empty() {
                            println!("No sessions found.");
                        } else {
                            for s in &sessions {
                                let status = if s.is_closed { "closed" } else { "open" };
                                let summary = s
                                    .summary
                                    .as_deref()
                                    .unwrap_or("(no summary)");
                                println!(
                                    "  {} [{}] msgs={} {}",
                                    &s.id[..8],
                                    status,
                                    s.message_count,
                                    summary
                                );
                            }
                        }
                    }
                }
            }

            AgentCommand::Skills { action } => {
                let config = StarpodConfig::load().await?;
                let agent = StarpodAgent::new(config).await?;
                match action {
                    SkillAction::List => {
                        let skills = agent.skills().list()?;
                        if skills.is_empty() {
                            println!("No skills found.");
                        } else {
                            for s in &skills {
                                println!("  {} — {}", s.name, s.description.replace('\n', " "));
                            }
                        }
                    }
                    SkillAction::Show { name } => {
                        match agent.skills().get(&name)? {
                            Some(skill) => {
                                println!("Name: {}", skill.name);
                                println!("Description: {}", skill.description);
                                if let Some(ref compat) = skill.compatibility {
                                    println!("Compatibility: {}", compat);
                                }
                                println!("---");
                                println!("{}", skill.body);
                            }
                            None => println!("Skill '{}' not found.", name),
                        }
                    }
                    SkillAction::Create { name, description, body, file } => {
                        let body = if let Some(path) = file {
                            std::fs::read_to_string(&path)?
                        } else if let Some(b) = body {
                            b
                        } else {
                            anyhow::bail!("Provide --body or --file");
                        };
                        agent.skills().create(&name, &description, &body)?;
                        println!("Created skill '{}'.", name);
                    }
                    SkillAction::Delete { name } => {
                        agent.skills().delete(&name)?;
                        println!("Deleted skill '{}'.", name);
                    }
                }
            }

            AgentCommand::Cron { action } => {
                let config = StarpodConfig::load().await?;
                let agent = StarpodAgent::new(config).await?;
                match action {
                    CronAction::List => {
                        let jobs = agent.cron().list_jobs().await?;
                        if jobs.is_empty() {
                            println!("No cron jobs.");
                        } else {
                            for j in &jobs {
                                let status = if j.enabled { "enabled" } else { "disabled" };
                                let next = j.next_run_at
                                    .map(starpod_cron::store::epoch_to_rfc3339)
                                    .unwrap_or_else(|| "none".to_string());
                                let mode = j.session_mode.as_str();
                                let mut extra = String::new();
                                if j.retry_count > 0 {
                                    extra.push_str(&format!(" retry={}/{}", j.retry_count, j.max_retries));
                                }
                                if let Some(ref err) = j.last_error {
                                    extra.push_str(&format!(" err={}", truncate(err, 30)));
                                }
                                println!(
                                    "  {} [{}] [{}] next={}{} — {}",
                                    j.name, status, mode, next, extra,
                                    truncate(&j.prompt, 60)
                                );
                            }
                        }
                    }
                    CronAction::Remove { name } => {
                        agent.cron().remove_job_by_name(&name).await?;
                        println!("Removed job '{}'.", name);
                    }
                    CronAction::Runs { name, limit } => {
                        let job = agent.cron().get_job_by_name(&name).await?;
                        match job {
                            Some(j) => {
                                let runs = agent.cron().list_runs(&j.id, limit).await?;
                                if runs.is_empty() {
                                    println!("No runs for '{}'.", name);
                                } else {
                                    for r in &runs {
                                        let summary = r.result_summary.as_deref().unwrap_or("");
                                        let started = starpod_cron::store::epoch_to_rfc3339(r.started_at);
                                        println!(
                                            "  {} {:?} {}",
                                            started, r.status,
                                            truncate(summary, 60)
                                        );
                                    }
                                }
                            }
                            None => println!("Job '{}' not found.", name),
                        }
                    }
                    CronAction::Run { name } => {
                        let agent = Arc::new(agent);
                        let job = agent.cron().get_job_by_name(&name).await?;
                        match job {
                            Some(j) => {
                                println!("Running job '{}'...", name);
                                let msg = ChatMessage {
                                    text: j.prompt.clone(),
                                    user_id: Some("cron-cli".into()),
                                    channel_id: Some(match j.session_mode {
                                        starpod_cron::SessionMode::Main => "main".into(),
                                        starpod_cron::SessionMode::Isolated => "scheduler".into(),
                                    }),
                                    channel_session_key: match j.session_mode {
                                        starpod_cron::SessionMode::Main => Some("main".into()),
                                        starpod_cron::SessionMode::Isolated => None,
                                    },
                                    attachments: Vec::new(),
                                };
                                match agent.chat(msg).await {
                                    Ok(resp) => println!("{}", resp.text),
                                    Err(e) => eprintln!("Job failed: {}", e),
                                }
                            }
                            None => println!("Job '{}' not found.", name),
                        }
                    }
                    CronAction::Edit { name, prompt, schedule, enabled, max_retries, timeout_secs, session_mode } => {
                        let job = agent.cron().get_job_by_name(&name).await?;
                        match job {
                            Some(j) => {
                                let schedule_update = schedule.map(|expr| {
                                    starpod_cron::Schedule::Cron { expr }
                                });
                                let session_mode_update = session_mode.map(|s| {
                                    starpod_cron::SessionMode::from_str(&s)
                                });
                                let update = starpod_cron::JobUpdate {
                                    prompt,
                                    schedule: schedule_update,
                                    enabled,
                                    max_retries,
                                    timeout_secs,
                                    session_mode: session_mode_update,
                                };
                                agent.cron().update_job(&j.id, &update).await?;
                                println!("Updated job '{}'.", name);
                            }
                            None => println!("Job '{}' not found.", name),
                        }
                    }
                }
            }
        },

        // ── Instance commands ──────────────────────────────────────────
        Commands::Instance { action } => {
            let config = StarpodConfig::load().await?;
            let backend_url = config.resolved_instance_backend_url();

            let Some(backend_url) = backend_url else {
                eprintln!(
                    "  {} Instance backend not configured.",
                    "✗".red().bold()
                );
                eprintln!(
                    "  {} Set {} in .starpod/config.toml or env var {}.",
                    "→".dimmed(),
                    "instance_backend_url".bright_white(),
                    "STARPOD_INSTANCE_BACKEND_URL".bright_white()
                );
                std::process::exit(1);
            };

            let api_key = config.resolved_api_key();
            let client = InstanceClient::new_with_timeout(&backend_url, api_key, config.instances.http_timeout_secs)?;

            match action {
                InstanceCommand::Create { name, region } => {
                    let req = starpod_instances::CreateInstanceRequest { name, region };
                    match client.create_instance(&req).await {
                        Ok(inst) => {
                            println!(
                                "  {} Created instance {}",
                                "✓".green().bold(),
                                inst.id.bright_white()
                            );
                            if let Some(name) = &inst.name {
                                println!("  {} Name: {}", "│".dimmed(), name);
                            }
                            println!("  {} Status: {}", "│".dimmed(), inst.status);
                            if let Some(region) = &inst.region {
                                println!("  {} Region: {}", "│".dimmed(), region);
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} Failed to create instance: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::List => {
                    match client.list_instances().await {
                        Ok(instances) => {
                            if instances.is_empty() {
                                println!("  {} No instances found.", "ℹ".bright_cyan());
                            } else {
                                for inst in &instances {
                                    let name = inst.name.as_deref().unwrap_or("(unnamed)");
                                    let region = inst.region.as_deref().unwrap_or("-");
                                    println!(
                                        "  {} [{}] {} region={}",
                                        &inst.id[..8.min(inst.id.len())],
                                        inst.status,
                                        name,
                                        region
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} Failed to list instances: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Kill { id } => {
                    match client.kill_instance(&id).await {
                        Ok(()) => println!("  {} Killed instance {}.", "✓".green().bold(), id),
                        Err(e) => {
                            eprintln!("  {} Failed to kill instance {}: {}", "✗".red(), id, e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Pause { id } => {
                    match client.pause_instance(&id).await {
                        Ok(()) => println!("  {} Paused instance {}.", "✓".green().bold(), id),
                        Err(e) => {
                            eprintln!("  {} Failed to pause instance {}: {}", "✗".red(), id, e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Restart { id } => {
                    match client.restart_instance(&id).await {
                        Ok(()) => println!("  {} Restarted instance {}.", "✓".green().bold(), id),
                        Err(e) => {
                            eprintln!("  {} Failed to restart instance {}: {}", "✗".red(), id, e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Logs { id, tail } => {
                    println!(
                        "  {} Streaming logs for {} (tail={})...\n",
                        "●".bright_green(),
                        id.bright_white(),
                        tail
                    );

                    match client.stream_logs(&id, Some(tail)).await {
                        Ok(stream) => {
                            use futures::StreamExt;
                            tokio::pin!(stream);
                            while let Some(entry) = StreamExt::next(&mut stream).await {
                                match entry {
                                    Ok(log) => {
                                        let ts = chrono::DateTime::from_timestamp(log.timestamp, 0)
                                            .map(|dt| dt.format("%H:%M:%S").to_string())
                                            .unwrap_or_else(|| log.timestamp.to_string());
                                        let level_colored = match log.level.as_str() {
                                            "error" => log.level.red().to_string(),
                                            "warn" => log.level.yellow().to_string(),
                                            "info" => log.level.green().to_string(),
                                            "debug" => log.level.dimmed().to_string(),
                                            _ => log.level.clone(),
                                        };
                                        println!(
                                            "  {} [{}] {}",
                                            ts.dimmed(),
                                            level_colored,
                                            log.message
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!("  {} Log stream error: {}", "✗".red(), e);
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} Failed to stream logs: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Ssh { id } => {
                    match client.get_ssh_info(&id).await {
                        Ok(ssh) => {
                            // If a private key is provided, write it to a temp file
                            let key_file = if let Some(ref key) = ssh.private_key {
                                let path = std::env::temp_dir().join(format!("starpod-ssh-{}.pem", id));
                                std::fs::write(&path, key)?;
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    std::fs::set_permissions(
                                        &path,
                                        std::fs::Permissions::from_mode(0o600),
                                    )?;
                                }
                                Some(path)
                            } else {
                                None
                            };

                            println!(
                                "  {} Connecting to {}@{}:{}...",
                                "●".bright_green(),
                                ssh.user,
                                ssh.host,
                                ssh.port
                            );

                            let mut cmd = std::process::Command::new("ssh");
                            cmd.arg("-p").arg(ssh.port.to_string());
                            if let Some(ref key_path) = key_file {
                                cmd.arg("-i").arg(key_path);
                                cmd.arg("-o").arg("StrictHostKeyChecking=no");
                            }
                            cmd.arg(format!("{}@{}", ssh.user, ssh.host));

                            let status = cmd.status()?;

                            // Clean up temp key file
                            if let Some(key_path) = key_file {
                                let _ = std::fs::remove_file(key_path);
                            }

                            if !status.success() {
                                std::process::exit(status.code().unwrap_or(1));
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} Failed to get SSH info: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Health { id } => {
                    match client.get_health(&id).await {
                        Ok(health) => {
                            println!("  {} Instance {} health:", "●".bright_green(), id.bright_white());
                            println!("  {} CPU:       {:.1}%", "│".dimmed(), health.cpu_percent);
                            println!("  {} Memory:    {} MB", "│".dimmed(), health.memory_mb);
                            println!("  {} Disk:      {} MB", "│".dimmed(), health.disk_mb);
                            println!("  {} Uptime:    {}s", "│".dimmed(), health.uptime_secs);
                            let hb = chrono::DateTime::from_timestamp(health.last_heartbeat, 0)
                                .map(|dt| dt.to_rfc3339())
                                .unwrap_or_else(|| health.last_heartbeat.to_string());
                            println!("  {} Heartbeat: {}", "│".dimmed(), hb);
                        }
                        Err(e) => {
                            eprintln!("  {} Failed to get health: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }

    }

    Ok(())
}

/// Interactive REPL mode with rich output.
async fn run_repl(config: StarpodConfig, name: &str) -> anyhow::Result<()> {
    let agent = StarpodAgent::new(config).await?;
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
                rustyline::error::ReadlineError::Interrupted
                | rustyline::error::ReadlineError::Eof,
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
        };
        let (mut stream, session_id, _followup_tx) = agent.chat_stream(&chat_msg).await?;
        let (result_text, result_msg) = process_stream(&mut stream, &start).await?;

        if let Some(ref result) = result_msg {
            agent.finalize_chat(&session_id, line, &result_text, result).await;
            print_result(&result_text, result, &start);
        } else if !result_text.is_empty() {
            println!("\n  {}\n", result_text);
        }

        println!();
    }

    Ok(())
}
