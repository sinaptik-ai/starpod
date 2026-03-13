use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use colored::Colorize;
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

use agent_sdk::{ContentBlock, Message};
use orion_agent::OrionAgent;
use orion_core::OrionConfig;

#[derive(Parser)]
#[command(name = "orion", about = "Orion — personal AI assistant platform", version)]
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

    /// Instance management (coming soon).
    Instance {
        #[command(subcommand)]
        action: InstanceCommand,
    },

    // ── Utility subcommands (will move under `agent` later) ──

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

// ── Agent subcommands ──────────────────────────────────────────────────────

#[derive(Subcommand)]
enum AgentCommand {
    /// Initialize a new Orion project in the current directory.
    Init,

    /// Start the gateway HTTP/WS server (+ Telegram bot if configured).
    Serve,

    /// Send a one-shot chat message.
    Chat {
        /// The message to send.
        message: String,
    },

    /// Start an interactive REPL session.
    Repl,
}

// ── Instance subcommands (stubs for future backend) ────────────────────────

#[derive(Subcommand)]
enum InstanceCommand {
    /// Create a new remote instance.
    Create,
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
    /// Create a new skill from a file or inline content.
    Create {
        /// Skill name.
        name: String,
        /// Markdown content (or use --file).
        #[arg(short, long)]
        content: Option<String>,
        /// Read content from a file.
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

fn print_header() {
    println!();
    println!(
        "{}",
        "  ╭──────────────────────────────────────────╮".bright_cyan()
    );
    println!(
        "{}",
        "  │          Orion  ·  AI Assistant           │".bright_cyan()
    );
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
            AgentCommand::Init => {
                let cwd = std::env::current_dir()?;
                OrionConfig::init(&cwd).await?;
                println!(
                    "  {} Initialized Orion project in {}",
                    "✓".green().bold(),
                    cwd.join(".orion").display()
                );
                println!(
                    "  {} Edit {} to configure your agent.",
                    "→".dimmed(),
                    ".orion/config.toml".bright_white()
                );
            }

            AgentCommand::Serve => {
                let config = OrionConfig::load().await?;
                let addr = &config.server_addr;
                let agent = Arc::new(OrionAgent::new(config.clone()).await?);

                // Start Telegram bot in background if token is configured
                let telegram_token = config
                    .telegram_bot_token
                    .clone()
                    .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok());

                let telegram_active = telegram_token.is_some();
                if let Some(token) = telegram_token {
                    let tg_agent = Arc::clone(&agent);
                    tokio::spawn(async move {
                        if let Err(e) = orion_telegram::run_with_agent(tg_agent, token).await {
                            tracing::error!(error = %e, "Telegram bot error");
                        }
                    });
                }

                // Print startup banner
                println!();
                println!(
                    "  {} {}",
                    "Orion".bright_cyan().bold(),
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
                    "Telegram".dimmed(),
                    if telegram_active {
                        "connected".green().to_string()
                    } else {
                        "not configured".yellow().to_string()
                    }
                );
                println!(
                    "  {} {}",
                    "Model   ".dimmed(),
                    config.model.bright_white()
                );
                println!(
                    "  {} {}",
                    "Project ".dimmed(),
                    config.project_root.display().to_string().bright_white()
                );
                println!();

                orion_gateway::serve_with_agent(agent, config).await?;
            }

            AgentCommand::Chat { message } => {
                let config = OrionConfig::load().await?;
                print_header();
                let start = Instant::now();
                let agent = OrionAgent::new(config).await?;

                let (mut stream, session_id) = agent.chat_stream(&message).await?;
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
                let config = OrionConfig::load().await?;
                run_repl(config).await?;
            }
        },

        // ── Instance commands (stubs) ──────────────────────────────────
        Commands::Instance { action } => {
            match action {
                InstanceCommand::Create => {
                    println!("  {} Instance management is coming soon.", "ℹ".bright_cyan());
                    println!("  {} This will connect to the Orion backend to spin up remote instances.", "→".dimmed());
                }
                InstanceCommand::List => {
                    println!("  {} No instances running (backend not connected).", "ℹ".bright_cyan());
                }
                InstanceCommand::Kill { id } => {
                    println!("  {} Cannot kill instance {}: backend not connected.", "✗".red(), id);
                }
                InstanceCommand::Pause { id } => {
                    println!("  {} Cannot pause instance {}: backend not connected.", "✗".red(), id);
                }
                InstanceCommand::Restart { id } => {
                    println!("  {} Cannot restart instance {}: backend not connected.", "✗".red(), id);
                }
            }
        }

        // ── Utility commands ───────────────────────────────────────────
        Commands::Memory { action } => {
            let config = OrionConfig::load().await?;
            let agent = OrionAgent::new(config).await?;
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

        Commands::Vault { action } => {
            let config = OrionConfig::load().await?;
            let agent = OrionAgent::new(config).await?;
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

        Commands::Sessions { action } => {
            let config = OrionConfig::load().await?;
            let agent = OrionAgent::new(config).await?;
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

        Commands::Skills { action } => {
            let config = OrionConfig::load().await?;
            let agent = OrionAgent::new(config).await?;
            match action {
                SkillAction::List => {
                    let skills = agent.skills().list()?;
                    if skills.is_empty() {
                        println!("No skills found.");
                    } else {
                        for s in &skills {
                            let preview = if s.content.len() > 60 {
                                format!("{}...", &s.content[..60])
                            } else {
                                s.content.clone()
                            };
                            println!("  {} — {}", s.name, preview.replace('\n', " "));
                        }
                    }
                }
                SkillAction::Show { name } => {
                    match agent.skills().get(&name)? {
                        Some(skill) => println!("{}", skill.content),
                        None => println!("Skill '{}' not found.", name),
                    }
                }
                SkillAction::Create { name, content, file } => {
                    let content = if let Some(path) = file {
                        std::fs::read_to_string(&path)?
                    } else if let Some(c) = content {
                        c
                    } else {
                        anyhow::bail!("Provide --content or --file");
                    };
                    agent.skills().create(&name, &content)?;
                    println!("Created skill '{}'.", name);
                }
                SkillAction::Delete { name } => {
                    agent.skills().delete(&name)?;
                    println!("Deleted skill '{}'.", name);
                }
            }
        }

        Commands::Cron { action } => {
            let config = OrionConfig::load().await?;
            let agent = OrionAgent::new(config).await?;
            match action {
                CronAction::List => {
                    let jobs = agent.cron().list_jobs().await?;
                    if jobs.is_empty() {
                        println!("No cron jobs.");
                    } else {
                        for j in &jobs {
                            let status = if j.enabled { "enabled" } else { "disabled" };
                            let next = j.next_run_at.as_deref().unwrap_or("none");
                            println!(
                                "  {} [{}] next={} — {}",
                                j.name, status, next,
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
                    let jobs = agent.cron().list_jobs().await?;
                    let job = jobs.iter().find(|j| j.name == name);
                    match job {
                        Some(j) => {
                            let runs = agent.cron().list_runs(&j.id, limit).await?;
                            if runs.is_empty() {
                                println!("No runs for '{}'.", name);
                            } else {
                                for r in &runs {
                                    let summary = r.result_summary.as_deref().unwrap_or("");
                                    println!(
                                        "  {} {:?} {}",
                                        r.started_at, r.status,
                                        truncate(summary, 60)
                                    );
                                }
                            }
                        }
                        None => println!("Job '{}' not found.", name),
                    }
                }
            }
        }
    }

    Ok(())
}

/// Interactive REPL mode with rich output.
async fn run_repl(config: OrionConfig) -> anyhow::Result<()> {
    let agent = OrionAgent::new(config).await?;

    print_header();
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
        let (mut stream, session_id) = agent.chat_stream(line).await?;
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
