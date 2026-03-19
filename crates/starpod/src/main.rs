mod onboarding;

use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use colored::Colorize;
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

use agent_sdk::{ContentBlock, Message};
use starpod_agent::StarpodAgent;
use starpod_core::{
    ChatMessage, StarpodConfig, ResolvedPaths,
    detect_mode, load_agent_config,
};
use starpod_instances::{DeployClient, DeployOpts, InstanceClient, parse_env_file};

#[derive(Parser)]
#[command(name = "starpod", about = "Starpod — personal AI assistant platform", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new workspace in the current directory.
    Init {
        /// Skip the interactive wizard and use defaults.
        #[arg(long)]
        default: bool,
    },

    /// Agent management — create new agents, list existing ones.
    Agent {
        #[command(subcommand)]
        action: AgentCommand,
    },

    /// Apply blueprint and start agent in dev mode (workspace only).
    Dev {
        /// Agent name from agents/ directory.
        agent: String,
        /// Port to serve on (overrides config).
        #[arg(short, long)]
        port: Option<u16>,
    },

    /// Start the gateway HTTP/WS server (+ Telegram bot if configured).
    Serve {
        /// Agent name (required in workspace mode, optional in single-agent).
        #[arg(short, long)]
        agent: Option<String>,
    },

    /// Send a one-shot chat message.
    Chat {
        /// Agent name.
        #[arg(short, long)]
        agent: Option<String>,
        /// The message to send.
        message: String,
    },

    /// Start an interactive REPL session.
    Repl {
        /// Agent name.
        #[arg(short, long)]
        agent: Option<String>,
    },

    /// Memory management commands.
    Memory {
        /// Agent name.
        #[arg(short, long)]
        agent: Option<String>,
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Session management.
    Sessions {
        /// Agent name.
        #[arg(short, long)]
        agent: Option<String>,
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Skill management.
    Skill {
        /// Agent name (manages instance-level skills for this agent).
        #[arg(short, long)]
        agent: Option<String>,
        #[command(subcommand)]
        action: SkillAction,
    },

    /// Cron job management.
    Cron {
        /// Agent name.
        #[arg(short, long)]
        agent: Option<String>,
        #[command(subcommand)]
        action: CronAction,
    },

    /// Remote instance management.
    Instance {
        #[command(subcommand)]
        action: InstanceCommand,
    },

    /// Build a standalone .starpod/ from an agent blueprint.
    Build {
        /// Path to agent blueprint folder (must contain agent.toml).
        #[arg(long)]
        agent: String,
        /// Path to skills folder to include.
        #[arg(long)]
        skills: Option<String>,
        /// Where to create the .starpod/ directory (default: current dir).
        #[arg(long)]
        output: Option<String>,
        /// Path to .env file to include.
        #[arg(long = "env")]
        env_file: Option<String>,
        /// Overwrite existing .starpod/ blueprint files.
        #[arg(long)]
        force: bool,
    },

    /// Deploy stub (future).
    Deploy {
        /// Agent name from agents/ directory.
        agent_name: String,
        /// Cloud zone for the instance.
        #[arg(long)]
        zone: Option<String>,
        /// Machine type for the instance.
        #[arg(long)]
        machine_type: Option<String>,
        /// Skip instance creation (upload files & secrets only).
        #[arg(long)]
        no_instance: bool,
        /// Path to .env file with secrets (defaults to .env in workspace root).
        #[arg(long)]
        env_file: Option<String>,
    },
}

// ── Agent subcommands ──────────────────────────────────────────────────────

#[derive(Subcommand)]
enum AgentCommand {
    /// Create a new agent in the workspace.
    New {
        /// Agent name (lowercase, hyphens).
        name: String,
        /// Skip the interactive wizard and use defaults.
        #[arg(long)]
        default: bool,
        /// Agent display name.
        #[arg(long)]
        agent_name: Option<String>,
        /// Agent personality text.
        #[arg(long)]
        soul: Option<String>,
        /// Claude model to use.
        #[arg(long, default_value = "claude-sonnet-4-6")]
        model: String,
    },
    /// List agents in the workspace.
    List,
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
    /// Create a new skill (AI-generated from name + optional prompt).
    /// If name/description/prompt are omitted, prompts interactively.
    New {
        /// Skill name (lowercase, hyphens, e.g. 'code-review').
        name: Option<String>,
        /// Description of what the skill does and when to use it.
        #[arg(short, long)]
        description: Option<String>,
        /// Extra instructions or context for the AI generator.
        #[arg(short, long)]
        prompt: Option<String>,
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

// ── Skill generation prompt ───────────────────────────────────────────────

const SKILL_GEN_SYSTEM_PROMPT: &str = r#"You are a skill author for the AgentSkills open format (agentskills.io).

Given a natural language request, generate a skill definition with three fields:
- **name**: A concise, lowercase identifier using only letters, digits, and hyphens (max 64 chars). Must not start/end with a hyphen or contain consecutive hyphens.
- **description**: 1-2 sentences explaining what the skill does AND when to use it. Use imperative phrasing ("Use this skill when..."). Be "pushy" — explicitly list contexts where the skill applies, including indirect mentions. Max 1024 chars.
- **body**: Markdown instructions the agent follows when the skill is activated. Under 500 lines.

## Best practices for the body

- **Add what the agent lacks, omit what it knows.** Focus on project-specific conventions, domain procedures, non-obvious edge cases. Don't explain general knowledge.
- **Favor procedures over declarations.** Teach how to approach a class of problems, not what to produce for a specific instance.
- **Provide defaults, not menus.** Pick one recommended approach; mention alternatives briefly.
- **Match specificity to fragility.** Be prescriptive when operations are fragile or sequence matters; give freedom when multiple approaches are valid.
- **Use effective patterns:**
  - Gotchas sections for environment-specific facts that defy assumptions
  - Templates for output format (concrete structure, not prose)
  - Checklists for multi-step workflows with explicit step tracking
  - Validation loops: do work → run validator → fix issues → repeat
  - Plan-validate-execute: create plan → validate → execute
- **Design coherent units.** Not too narrow (needing multiple skills for one task), not too broad (hard to activate precisely).
- **Keep it actionable.** Concise stepwise guidance with working examples outperforms exhaustive documentation.

## Output

Return a JSON object with exactly: `name`, `description`, `body`.
"#;

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

/// Walk up from `start` to find the nearest directory containing `starpod.toml`.
fn find_workspace_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("starpod.toml").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
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

// ── Blueprint scaffold helper ────────────────────────────────────────────

/// Create agent blueprint directory structure (used by both `init` and `agent new`).
///
/// Reads `starpod.toml` (if it exists in `workspace_root`) and bakes its values
/// into the generated `agent.toml`, making each agent self-contained.
async fn scaffold_agent_blueprint(
    agent_dir: &std::path::Path,
    name: &str,
    display_name: &str,
    model: &str,
    provider: &str,
    soul: Option<&str>,
    workspace_root: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(agent_dir.join("files")).await?;

    // Read workspace starpod.toml to inherit defaults (scaffolding)
    let ws_config: Option<toml::Value> = if let Some(root) = workspace_root {
        let ws_toml = root.join("starpod.toml");
        if ws_toml.is_file() {
            let content = tokio::fs::read_to_string(&ws_toml).await?;
            toml::from_str(&content).ok()
        } else {
            None
        }
    } else {
        None
    };

    // Helper to read a string field from workspace config
    let ws_str = |key: &str, fallback: &str| -> String {
        ws_config
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_str())
            .unwrap_or(fallback)
            .to_string()
    };
    let ws_int = |key: &str, fallback: i64| -> i64 {
        ws_config
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_integer())
            .unwrap_or(fallback)
    };

    // Bake workspace defaults into agent.toml (self-contained)
    let effective_provider = ws_str("provider", provider);
    let effective_model = ws_str("model", model);
    let effective_max_turns = ws_int("max_turns", 30);
    let effective_server_addr = ws_str("server_addr", "127.0.0.1:3000");

    let agent_toml = format!(
        r#"# Agent configuration for {name}
# This file is self-contained — all settings are here (not inherited from starpod.toml).

agent_name = "{display_name}"
provider = "{provider}"
model = "{model}"
max_turns = {max_turns}
server_addr = "{server_addr}"
# skills = []  # empty = all workspace skills

# max_tokens = 16384
# reasoning_effort = "low"  # low, medium, high
# compaction_model = "{model}"
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

# [channels.telegram]
# enabled = true
# gap_minutes = 360  # inactivity gap before auto-closing session (6h)
# allowed_users = []  # numeric IDs or usernames, e.g. [123456789, "alice"]
# stream_mode = "final_only"  # final_only or all_messages
"#,
        name = name,
        display_name = display_name,
        provider = effective_provider,
        model = effective_model,
        max_turns = effective_max_turns,
        server_addr = effective_server_addr,
    );
    tokio::fs::write(agent_dir.join("agent.toml"), agent_toml).await?;

    let soul_content = match soul {
        Some(text) => format!(
            "# Soul\n\n\
             You are {display_name}, a personal AI assistant. {text}\n\n\
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
        ),
        None => format!(
            "# Soul\n\n\
             You are {display_name}, a personal AI assistant. You are helpful, direct, and thoughtful.\n\n\
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
        ),
    };
    tokio::fs::write(agent_dir.join("SOUL.md"), soul_content).await?;

    // Seed lifecycle files (empty defaults)
    for name in &["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md"] {
        let path = agent_dir.join(name);
        if !path.exists() {
            tokio::fs::write(&path, "").await?;
        }
    }

    // Seed frontend.toml with defaults
    let frontend_path = agent_dir.join("frontend.toml");
    if !frontend_path.exists() {
        let frontend_toml = format!(
            r#"# Frontend configuration for the web UI welcome screen.

# Greeting text shown below the logo (default: "ready_")
# greeting = "Hi! I'm {display_name}."

# Suggested prompts shown as clickable chips
prompts = [
    "What can you help me with?",
    "What do you remember about me?",
]
"#,
        );
        tokio::fs::write(&frontend_path, frontend_toml).await?;
    }

    Ok(())
}

// ── Agent resolution helper ─────────────────────────────────────────────

/// Resolve mode, paths, config, and build agent from an optional agent name flag.
/// This is the standard flow for all commands that need an agent.
async fn resolve_agent(
    agent_name: Option<String>,
) -> anyhow::Result<(StarpodAgent, StarpodConfig, ResolvedPaths)> {
    let mode = detect_mode(agent_name.as_deref())?;
    let paths = ResolvedPaths::resolve(&mode)?;
    let agent_config = load_agent_config(&paths)?;
    let starpod_config = agent_config.clone().into_starpod_config(&paths);
    let agent = StarpodAgent::with_paths(agent_config, paths.clone()).await?;
    Ok((agent, starpod_config, paths))
}

/// Set up telegram bot + cron notifier from config.
///
/// Spawns the telegram bot task if configured, and returns the cron notifier sender.
fn setup_telegram_and_notifier(
    agent: &Arc<StarpodAgent>,
    config: &StarpodConfig,
) -> Option<starpod_cron::NotificationSender> {
    let telegram_token = config.resolved_telegram_token();
    let telegram_allowed = config.resolved_telegram_allowed_user_ids();
    let telegram_allowed_usernames = config.resolved_telegram_allowed_usernames();

    let cron_notifier: Option<starpod_cron::NotificationSender> =
        if let Some(ref token) = telegram_token {
            if !telegram_allowed.is_empty() {
                let token = token.clone();
                let users = telegram_allowed.clone();
                Some(Arc::new(move |_job_name, _session_id, result_text, _success| {
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

    if let Some(token) = telegram_token {
        let tg_agent = Arc::clone(agent);
        let allowed = telegram_allowed;
        let allowed_names = telegram_allowed_usernames;
        tokio::spawn(async move {
            if let Err(e) =
                starpod_telegram::run_with_agent_filtered(tg_agent, token, allowed, allowed_names).await
            {
                tracing::error!(error = %e, "Telegram bot error");
            }
        });
    }

    cron_notifier
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
        // ── Init: scaffold a new workspace ────────────────────────────
        Commands::Init { default: use_defaults } => {
            let cwd = std::env::current_dir()?;

            if cwd.join("starpod.toml").exists() {
                eprintln!(
                    "  {} Already initialized: {} exists.",
                    "✗".red().bold(),
                    "starpod.toml".bright_white()
                );
                std::process::exit(1);
            }

            // Collect answers: interactive wizard or defaults
            let answers = if use_defaults {
                None
            } else {
                onboarding::run_wizard()
            };

            let (provider, model, api_key, first_agent, agent_display) = match answers {
                Some(a) => (
                    a.provider,
                    a.model,
                    a.api_key,
                    a.first_agent_name,
                    a.agent_display_name,
                ),
                None => (
                    "anthropic".to_string(),
                    "claude-sonnet-4-6".to_string(),
                    None,
                    None,
                    None,
                ),
            };

            // Create workspace scaffold
            tokio::fs::write(
                cwd.join("starpod.toml"),
                onboarding::generate_workspace_config_with(&provider, &model),
            ).await?;
            tokio::fs::create_dir_all(cwd.join("agents")).await?;
            tokio::fs::create_dir_all(cwd.join("skills")).await?;
            tokio::fs::write(
                cwd.join(".env"),
                onboarding::generate_env_content(&provider, api_key.as_deref()),
            ).await?;
            tokio::fs::write(
                cwd.join(".env.dev"),
                onboarding::generate_env_dev_content(&provider, api_key.as_deref()),
            ).await?;

            // Add .env and .instances/ to .gitignore if not already there
            let gitignore_path = cwd.join(".gitignore");
            let mut gitignore_content = if gitignore_path.exists() {
                tokio::fs::read_to_string(&gitignore_path).await?
            } else {
                String::new()
            };
            let mut additions = Vec::new();
            if !gitignore_content.contains(".env") {
                additions.push(".env");
            }
            if !gitignore_content.contains(".instances/") {
                additions.push(".instances/");
            }
            if !gitignore_content.contains("*/data/") {
                additions.push("*/data/");
            }
            if !additions.is_empty() {
                if !gitignore_content.is_empty() && !gitignore_content.ends_with('\n') {
                    gitignore_content.push('\n');
                }
                gitignore_content.push_str(&additions.join("\n"));
                gitignore_content.push('\n');
            }
            tokio::fs::write(&gitignore_path, gitignore_content).await?;

            println!();
            println!(
                "  {} Initialized Starpod workspace in {}",
                "✓".green().bold(),
                cwd.display()
            );

            // Create first agent if requested during wizard (blueprint layout)
            if let Some(agent_name) = first_agent {
                let display_name = agent_display.as_deref().unwrap_or("Aster");
                let agent_dir = cwd.join("agents").join(&agent_name);
                scaffold_agent_blueprint(&agent_dir, &agent_name, display_name, &model, &provider, None, Some(&cwd)).await?;

                println!(
                    "  {} Created agent '{}'",
                    "✓".green().bold(),
                    agent_name.bright_white()
                );
                println!(
                    "  {} Run {} to start.",
                    "→".dimmed(),
                    format!("starpod dev {}", agent_name).bright_white()
                );
            } else {
                println!(
                    "  {} Run {} to create your first agent.",
                    "→".dimmed(),
                    "starpod agent new <name>".bright_white()
                );
            }

            println!();
        }

        // ── Agent management ──────────────────────────────────────────
        Commands::Agent { action } => match action {
            AgentCommand::New {
                name,
                default: _use_default,
                agent_name,
                soul,
                model,
            } => {
                let cwd = std::env::current_dir()?;

                // Find workspace root
                let mut workspace_root = None;
                let mut dir = cwd.clone();
                loop {
                    if dir.join("starpod.toml").is_file() {
                        workspace_root = Some(dir);
                        break;
                    }
                    if !dir.pop() { break; }
                }

                let root = workspace_root.ok_or_else(|| {
                    anyhow::anyhow!("No starpod.toml found. Run `starpod init` first.")
                })?;

                let agent_dir = root.join("agents").join(&name);
                if agent_dir.exists() {
                    eprintln!(
                        "  {} Agent '{}' already exists at {}.",
                        "✗".red().bold(),
                        name,
                        agent_dir.display()
                    );
                    std::process::exit(1);
                }

                // Create blueprint scaffold (no runtime data — that goes in .instances/)
                let display_name = agent_name.as_deref().unwrap_or(&name);
                scaffold_agent_blueprint(
                    &agent_dir,
                    &name,
                    display_name,
                    &model,
                    "anthropic", // CLI default; starpod.toml values baked in via workspace_root
                    soul.as_deref(),
                    Some(&root),
                ).await?;

                println!();
                println!(
                    "  {} Created agent '{}' at {}",
                    "✓".green().bold(),
                    name.bright_white(),
                    agent_dir.display()
                );
                println!(
                    "  {} Edit {} for agent-specific config.",
                    "→".dimmed(),
                    agent_dir.join("agent.toml").display().to_string().bright_white()
                );
                println!(
                    "  {} Run {} to start in dev mode.",
                    "→".dimmed(),
                    format!("starpod dev {}", name).bright_white()
                );
                println!();
            }

            AgentCommand::List => {
                let cwd = std::env::current_dir()?;
                let mut workspace_root = None;
                let mut dir = cwd.clone();
                loop {
                    if dir.join("starpod.toml").is_file() {
                        workspace_root = Some(dir);
                        break;
                    }
                    if !dir.pop() { break; }
                }

                let root = workspace_root.ok_or_else(|| {
                    anyhow::anyhow!("No starpod.toml found. Run `starpod init` first.")
                })?;

                let agents_dir = root.join("agents");
                if !agents_dir.is_dir() {
                    println!("  No agents/ directory.");
                    return Ok(());
                }

                let mut entries = tokio::fs::read_dir(&agents_dir).await?;
                let mut agents = Vec::new();
                while let Some(entry) = entries.next_entry().await? {
                    if entry.path().is_dir() {
                        agents.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
                agents.sort();

                if agents.is_empty() {
                    println!("  No agents found. Run {} to create one.", "starpod agent new <name>".bright_white());
                } else {
                    for name in &agents {
                        let agent_toml = agents_dir.join(name).join("agent.toml");
                        let has_config = agent_toml.is_file();
                        let status = if has_config { "✓".green() } else { "?".yellow() };
                        println!("  {} {}", status, name);
                    }
                }
            }

        },

        // ── Dev ──────────────────────────────────────────────────────
        Commands::Dev { agent: agent_name, port } => {
            // Require workspace mode
            let cwd = std::env::current_dir()?;
            let mode = starpod_core::detect_mode_from(Some(&agent_name), &cwd)?;
            let workspace_root = match &mode {
                starpod_core::Mode::Workspace { root, .. } => root.clone(),
                _ => {
                    eprintln!("Error: `starpod dev` requires workspace mode (starpod.toml must exist).");
                    std::process::exit(1);
                }
            };

            // Apply blueprint
            let blueprint_dir = workspace_root.join("agents").join(&agent_name);
            let instance_dir = workspace_root.join(".instances").join(&agent_name);

            starpod_core::apply_blueprint(
                &blueprint_dir,
                &instance_dir,
                &workspace_root,
                starpod_core::EnvSource::Dev,
            )?;

            // Resolve paths as Instance mode
            let instance_mode = starpod_core::Mode::Instance {
                instance_root: instance_dir.clone(),
                agent_name: agent_name.clone(),
            };
            let paths = starpod_core::ResolvedPaths::resolve(&instance_mode)?;

            // Run migration if old data/ layout exists
            paths.migrate_if_needed();

            let mut agent_config = starpod_core::load_agent_config(&paths)?;

            // Override port if specified
            if let Some(p) = port {
                agent_config.server_addr = format!("127.0.0.1:{}", p);
            }

            let config = agent_config.clone().into_starpod_config(&paths);
            let addr = config.server_addr.clone();
            let display_name = config.agent_name.clone();

            let agent = Arc::new(StarpodAgent::with_paths(agent_config, paths.clone()).await?);
            let cron_notifier = setup_telegram_and_notifier(&agent, &config);

            print_header_with_name(&display_name);
            println!("  {} {} → {}", "DEV".bright_yellow().bold(), agent_name.bright_cyan(), instance_dir.display().to_string().dimmed());
            println!("  {} {}", "Server".dimmed(), addr.bright_green());
            print_separator();

            starpod_gateway::serve_with_agent(agent, config, cron_notifier, paths).await?;
        }

        // ── Serve ─────────────────────────────────────────────────────
        Commands::Serve { agent: agent_name } => {
            let (agent, config, paths) = resolve_agent(agent_name).await?;
            let addr = config.server_addr.clone();
            let display_name = config.agent_name.clone();
            let telegram_active = config.resolved_telegram_token().is_some();
            let agent = Arc::new(agent);
            let cron_notifier = setup_telegram_and_notifier(&agent, &config);

            // Print startup banner
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
            println!(
                "  {} {}",
                "Docs    ".dimmed(),
                format!("http://{}/docs", addr).bright_white()
            );
            println!(
                "  {} {}",
                "Telegram".dimmed(),
                if telegram_active {
                    let mode = config.channels.telegram.as_ref()
                        .map(|t| t.stream_mode.as_str())
                        .unwrap_or("final_only");
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

            starpod_gateway::serve_with_agent(agent, config, cron_notifier, paths).await?;
        }

        // ── Chat ──────────────────────────────────────────────────────
        Commands::Chat { agent: agent_name, message } => {
            let (agent, config, _paths) = resolve_agent(agent_name).await?;
            let name = config.agent_name.clone();
            print_header_with_name(&name);
            let start = Instant::now();

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

        // ── Repl ──────────────────────────────────────────────────────
        Commands::Repl { agent: agent_name } => {
            let (agent, config, _paths) = resolve_agent(agent_name).await?;
            let name = config.agent_name.clone();
            run_repl(agent, &name).await?;
        }

        // ── Memory ────────────────────────────────────────────────────
        Commands::Memory { agent: agent_name, action } => {
            let (agent, _config, _paths) = resolve_agent(agent_name).await?;
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

        // ── Sessions ──────────────────────────────────────────────────
        Commands::Sessions { agent: agent_name, action } => {
            let (agent, _config, _paths) = resolve_agent(agent_name).await?;
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

        // ── Skills ────────────────────────────────────────────────────
        Commands::Skill { agent: agent_name, action } => {
            // Resolve skills directory: use agent's instance skills if --agent given,
            // otherwise detect mode and use resolved skills_dir.
            let cwd = std::env::current_dir()?;
            let skills_dir = if let Some(ref name) = agent_name {
                // Workspace mode: resolve to instance skills
                let mode = starpod_core::detect_mode_from(Some(name), &cwd)?;
                let paths = starpod_core::ResolvedPaths::resolve(&mode)?;
                paths.skills_dir
            } else {
                // Try to detect mode automatically
                match starpod_core::detect_mode(None) {
                    Ok(mode) => {
                        let paths = starpod_core::ResolvedPaths::resolve(&mode)?;
                        paths.skills_dir
                    }
                    Err(_) => {
                        // Fallback: workspace skills/ or .starpod/skills/
                        if cwd.join("starpod.toml").is_file() {
                            cwd.join("skills")
                        } else if cwd.join(".starpod").is_dir() {
                            cwd.join(".starpod").join("skills")
                        } else {
                            cwd.join("skills")
                        }
                    }
                }
            };

            let store = starpod_skills::SkillStore::new(&skills_dir)?;

            match action {
                SkillAction::List => {
                    let skills = store.list()?;
                    if skills.is_empty() {
                        println!("No skills found.");
                    } else {
                        for s in &skills {
                            println!("  {} — {}", s.name, s.description.replace('\n', " "));
                        }
                    }
                }
                SkillAction::Show { name } => {
                    match store.get(&name)? {
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
                SkillAction::New { name, description, prompt } => {
                    // Interactive prompts for missing fields
                    use dialoguer::{Input, theme::ColorfulTheme};
                    let theme = ColorfulTheme::default();

                    let name: String = match name {
                        Some(n) => n,
                        None => Input::with_theme(&theme)
                            .with_prompt("Skill name (lowercase, hyphens)")
                            .interact_text()?,
                    };
                    let description: Option<String> = match description {
                        Some(d) => Some(d),
                        None => {
                            let d: String = Input::with_theme(&theme)
                                .with_prompt("Description (what the skill does)")
                                .allow_empty(true)
                                .interact_text()?;
                            if d.is_empty() { None } else { Some(d) }
                        }
                    };
                    let prompt: Option<String> = match prompt {
                        Some(p) => Some(p),
                        None => {
                            let p: String = Input::with_theme(&theme)
                                .with_prompt("Extra instructions / context (optional)")
                                .allow_empty(true)
                                .interact_text()?;
                            if p.is_empty() { None } else { Some(p) }
                        }
                    };

                    // Load .env so ANTHROPIC_API_KEY (and other provider keys) are available
                    if let Ok(mode) = starpod_core::detect_mode(agent_name.as_deref()) {
                        if let Ok(paths) = starpod_core::ResolvedPaths::resolve(&mode) {
                            let _ = starpod_core::load_agent_config(&paths);
                        }
                    }

                    println!(
                        "  {} Generating skill '{}'...\n",
                        "⚡".bright_yellow(),
                        name.bright_white()
                    );

                    // Build the user prompt for the AI.
                    let mut user_prompt = format!("Create a skill named \"{}\".", name);
                    if let Some(ref d) = description {
                        user_prompt.push_str(&format!("\n\nThe skill description MUST be: {}", d));
                    }
                    if let Some(ref p) = prompt {
                        user_prompt.push_str(&format!("\n\nAdditional context:\n{}", p));
                    }

                    let output_schema = serde_json::json!({
                        "type": "object",
                        "properties": {
                            "description": {
                                "type": "string",
                                "description": "1-2 sentence description of what the skill does and when to use it. Max 1024 chars. Use imperative phrasing: 'Use this skill when...'"
                            },
                            "body": {
                                "type": "string",
                                "description": "Markdown instructions for the agent to follow when the skill is activated. Should be under 500 lines / ~5000 tokens."
                            }
                        },
                        "required": ["description", "body"],
                        "additionalProperties": false
                    });

                    let options = agent_sdk::Options::builder()
                        .system_prompt(agent_sdk::options::SystemPrompt::Custom(
                            SKILL_GEN_SYSTEM_PROMPT.to_string(),
                        ))
                        .output_format(output_schema)
                        .max_turns(1)
                        .persist_session(false)
                        .permission_mode(agent_sdk::PermissionMode::Plan)
                        .build();

                    let mut stream = agent_sdk::query(&user_prompt, options);

                    let mut result_msg = None;
                    while let Some(msg_result) = stream.next().await {
                        let msg = msg_result?;
                        if let Message::Result(result) = msg {
                            result_msg = Some(result);
                        }
                    }

                    let result = result_msg
                        .ok_or_else(|| anyhow::anyhow!("No result from AI"))?;

                    if result.is_error {
                        anyhow::bail!(
                            "Skill generation failed: {}",
                            result.errors.join("; ")
                        );
                    }

                    let structured = result.structured_output.ok_or_else(|| {
                        anyhow::anyhow!("No structured output returned from AI")
                    })?;

                    #[derive(serde::Deserialize)]
                    struct SkillGen {
                        description: String,
                        body: String,
                    }

                    let gen: SkillGen = serde_json::from_value(structured)?;

                    let skill_desc = description.unwrap_or(gen.description);

                    store.create(&name, &skill_desc, &gen.body)?;

                    println!(
                        "  {} Created skill '{}'",
                        "✓".green().bold(),
                        name.bright_white()
                    );
                    println!(
                        "  {} {}",
                        "Description:".dimmed(),
                        skill_desc
                    );
                    println!();
                    println!("{}", gen.body);
                }
                SkillAction::Delete { name } => {
                    store.delete(&name)?;
                    println!("Deleted skill '{}'.", name);
                }
            }
        }

        // ── Cron ──────────────────────────────────────────────────────
        Commands::Cron { agent: agent_name, action } => {
            let (agent, _config, _paths) = resolve_agent(agent_name).await?;
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

        // ── Instance commands ──────────────────────────────────────────
        Commands::Instance { action } => {
            let backend_url = std::env::var("STARPOD_INSTANCE_BACKEND_URL").ok();

            let Some(backend_url) = backend_url else {
                eprintln!(
                    "  {} Instance backend not configured.",
                    "✗".red().bold()
                );
                eprintln!(
                    "  {} Set env var {}.",
                    "→".dimmed(),
                    "STARPOD_INSTANCE_BACKEND_URL".bright_white()
                );
                std::process::exit(1);
            };

            let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
            let client = InstanceClient::new_with_timeout(&backend_url, api_key, 30)?;

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

        // ── Build ─────────────────────────────────────────────────────
        Commands::Build { agent, skills, output, env_file, force } => {
            let agent_path = std::path::PathBuf::from(&agent);
            if !agent_path.join("agent.toml").is_file() {
                eprintln!(
                    "  {} Blueprint directory {} does not contain agent.toml.",
                    "✗".red().bold(),
                    agent_path.display()
                );
                std::process::exit(1);
            }

            let output_dir = match output {
                Some(p) => std::path::PathBuf::from(p),
                None => std::env::current_dir()?,
            };
            let skills_path = skills.map(std::path::PathBuf::from);
            let env_path = env_file.map(std::path::PathBuf::from);

            starpod_core::build_standalone(
                &agent_path,
                &output_dir,
                skills_path.as_deref(),
                env_path.as_deref(),
                force,
            )?;

            let starpod_dir = output_dir.join(".starpod");
            println!();
            println!(
                "  {} Built standalone agent at {}",
                "✓".green().bold(),
                starpod_dir.display().to_string().bright_white()
            );
            println!(
                "  {} Run {} from {} to start.",
                "→".dimmed(),
                "starpod serve".bright_white(),
                output_dir.display().to_string().bright_white()
            );
            println!();
        }

        // ── Deploy stub ───────────────────────────────────────────────
        Commands::Deploy {
            agent_name,
            zone,
            machine_type,
            no_instance,
            env_file,
        } => {
            // Require backend URL and API key
            let backend_url = std::env::var("STARPOD_INSTANCE_BACKEND_URL").ok();
            let Some(backend_url) = backend_url else {
                eprintln!(
                    "  {} Deploy backend not configured.",
                    "✗".red().bold()
                );
                eprintln!(
                    "  {} Set env var {}.",
                    "→".dimmed(),
                    "STARPOD_INSTANCE_BACKEND_URL".bright_white()
                );
                std::process::exit(1);
            };

            let api_key = std::env::var("STARPOD_API_KEY").ok();
            let Some(api_key) = api_key else {
                eprintln!(
                    "  {} Authentication required.",
                    "✗".red().bold()
                );
                eprintln!(
                    "  {} Set env var {}.",
                    "→".dimmed(),
                    "STARPOD_API_KEY".bright_white()
                );
                std::process::exit(1);
            };

            let client = DeployClient::new(&backend_url, &api_key)?;

            // Resolve workspace paths
            let cwd = std::env::current_dir()?;
            let workspace_root = find_workspace_root(&cwd).unwrap_or_else(|| {
                eprintln!(
                    "  {} Not inside a starpod workspace (no starpod.toml found).",
                    "✗".red().bold()
                );
                std::process::exit(1);
            });

            let agent_dir = workspace_root.join("agents").join(&agent_name);
            if !agent_dir.exists() {
                eprintln!(
                    "  {} Agent '{}' not found in agents/ directory.",
                    "✗".red().bold(),
                    agent_name
                );
                std::process::exit(1);
            }

            let skills_dir = workspace_root.join("skills");

            // Collect env vars from .env file
            let env_path = if let Some(ref p) = env_file {
                std::path::PathBuf::from(p)
            } else {
                workspace_root.join(".env")
            };

            let env_vars = if env_path.exists() {
                parse_env_file(&env_path)?
            } else {
                std::collections::HashMap::new()
            };

            println!(
                "  {} Deploying agent {}...",
                "⟳".bright_cyan(),
                agent_name.bright_white().bold()
            );
            println!(
                "  {} Agent dir:  {}",
                "│".dimmed(),
                agent_dir.display()
            );
            if skills_dir.exists() {
                println!(
                    "  {} Skills dir: {}",
                    "│".dimmed(),
                    skills_dir.display()
                );
            }
            if !env_vars.is_empty() {
                println!(
                    "  {} Secrets:    {} keys from {}",
                    "│".dimmed(),
                    env_vars.len(),
                    env_path.display()
                );
            }
            println!();

            let skills_path = if skills_dir.exists() {
                Some(skills_dir.as_path())
            } else {
                None
            };

            // Track last printed status so we only print on change
            let last_status = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
            let last_status_clone = last_status.clone();

            let on_poll: Option<Box<dyn FnMut(&starpod_instances::deploy::InstanceResponse) + Send>> =
                if !no_instance {
                    Some(Box::new(move |inst| {
                        let mut last = last_status_clone.lock().unwrap();
                        if *last != inst.status {
                            eprint!(
                                "\r  {} Instance: {}                    ",
                                "⟳".bright_cyan(),
                                inst.status.bright_yellow()
                            );
                            *last = inst.status.clone();
                        }
                    }))
                } else {
                    None
                };

            let summary = client
                .deploy(DeployOpts {
                    agent_name: &agent_name,
                    agent_dir: &agent_dir,
                    skills_dir: skills_path,
                    env_vars,
                    create_instance: !no_instance,
                    zone: zone.as_deref(),
                    machine_type: machine_type.as_deref(),
                    on_instance_poll: on_poll,
                })
                .await?;

            // Clear the status line if we were polling
            if !no_instance && summary.instance.is_some() {
                eprint!("\r                                                \r");
            }

            println!(
                "  {} Agent deployed successfully!",
                "✓".green().bold()
            );
            println!(
                "  {} Agent ID:   {}",
                "│".dimmed(),
                summary.agent_id.bright_white()
            );
            println!(
                "  {} Files:      {}",
                "│".dimmed(),
                summary.files_uploaded
            );
            println!(
                "  {} Secrets:    {}",
                "│".dimmed(),
                summary.secrets_set
            );

            if let Some(ref inst) = summary.instance {
                println!();
                println!(
                    "  {} Instance is running!",
                    "✓".green().bold()
                );
                println!(
                    "  {} ID:     {}",
                    "│".dimmed(),
                    inst.id.bright_white()
                );
                println!(
                    "  {} Status: {}",
                    "│".dimmed(),
                    inst.status.bright_green()
                );
                if let Some(ref ip) = inst.ip_address {
                    println!(
                        "  {} IP:     {}",
                        "│".dimmed(),
                        ip.bright_white().bold()
                    );
                }
                if let Some(ref z) = inst.zone {
                    println!(
                        "  {} Zone:   {}",
                        "│".dimmed(),
                        z
                    );
                }
            } else {
                println!(
                    "\n  {} No instance created (use without {} to launch one).",
                    "ℹ".bright_cyan(),
                    "--no-instance".bright_white()
                );
            }
        }

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
