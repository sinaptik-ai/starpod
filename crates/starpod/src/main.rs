mod auth;
mod onboarding;

use std::collections::HashMap;
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
    deploy_manifest::{AgentConfigInput, DeployManifest, SkillEnvInput},
    detect_mode, load_agent_config,
};
use starpod_instances::{DeployClient, DeployOpts, InstanceClient, SecretResponse, parse_env_file};

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
            other => Err(format!("unknown format '{}' (expected 'text' or 'json')", other)),
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
#[command(name = "starpod", about = "Starpod — personal AI assistant platform", version)]
struct Cli {
    /// Output format: text (default) or json.
    #[arg(long, global = true, default_value = "text")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

/// Prompt for confirmation. Returns true if the user confirms.
/// Always returns true when `skip` is set (--yes flag).
fn confirm(message: &str, skip: bool) -> bool {
    if skip {
        return true;
    }
    eprint!("  {} [y/N] ", message);
    let mut buf = String::new();
    if std::io::stdin().read_line(&mut buf).is_err() {
        return false;
    }
    let answer = buf.trim().to_lowercase();
    answer == "y" || answer == "yes"
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

    /// Start agent in dev mode (workspace only).
    ///
    /// On first run, applies the blueprint from `agents/<name>/` to create the
    /// instance at `.instances/<name>/`. On subsequent runs, reuses the existing
    /// instance (preserving runtime state). Pass `--build` to force a blueprint
    /// re-apply, which overwrites `config/` and re-syncs skills.
    Dev {
        /// Agent name from agents/ directory.
        agent: String,
        /// Port to serve on (overrides config).
        #[arg(short, long)]
        port: Option<u16>,
        /// Force rebuild from blueprint (overwrites config/).
        #[arg(long)]
        build: bool,
    },

    /// Start the gateway HTTP/WS server (+ Telegram bot if configured).
    Serve {
        /// Agent name (required in workspace mode, optional in single-agent).
        #[arg(short, long)]
        agent: Option<String>,
    },

    /// Send a one-shot chat message.
    ///
    /// If no agent is specified and none can be resolved from the current
    /// directory, creates an ephemeral instance with default settings that
    /// is automatically deleted when the command finishes.
    Chat {
        /// Agent name (if omitted, uses an ephemeral instance).
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
        /// Agent name (can be placed before or after the subcommand).
        #[arg(short, long, global = true)]
        agent: Option<String>,
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Session management.
    Sessions {
        /// Agent name (can be placed before or after the subcommand).
        #[arg(short, long, global = true)]
        agent: Option<String>,
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Skill management.
    Skill {
        /// Agent name (can be placed before or after the subcommand).
        #[arg(short, long, global = true)]
        agent: Option<String>,
        #[command(subcommand)]
        action: SkillAction,
    },

    /// Cron job management.
    Cron {
        /// Agent name (can be placed before or after the subcommand).
        #[arg(short, long, global = true)]
        agent: Option<String>,
        #[command(subcommand)]
        action: CronAction,
    },

    /// Remote instance management.
    Instance {
        #[command(subcommand)]
        action: InstanceCommand,
    },

    /// Manage secrets in the remote secret store.
    Secret {
        #[command(subcommand)]
        action: SecretCommand,
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

    /// Authenticate with the Spawner backend.
    Auth {
        #[command(subcommand)]
        action: AuthCommand,
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
        #[arg(long, default_value = "claude-haiku-4-5")]
        model: String,
    },
    /// List agents in the workspace.
    List,
    /// Push local agent blueprint to the remote (Spawner).
    Push {
        /// Agent name from agents/ directory.
        name: String,
        /// Preview changes without pushing (dry run).
        #[arg(long)]
        dry_run: bool,
        /// Skip confirmation prompts.
        #[arg(long)]
        yes: bool,
    },
    /// Pull remote agent blueprint to the local workspace.
    Pull {
        /// Agent name from agents/ directory.
        name: String,
    },
    /// Show what would change on push or pull (without modifying anything).
    Diff {
        /// Agent name from agents/ directory.
        name: String,
    },
}

// ── Instance subcommands ────────────────────────────────────────────────────

#[derive(Subcommand)]
enum InstanceCommand {
    /// Create a new remote instance (validates deploy.toml if present).
    New {
        /// Agent name from agents/ directory.
        #[arg(short, long)]
        agent: Option<String>,
        /// Instance name (optional, for display purposes).
        #[arg(short, long)]
        name: Option<String>,
        /// Instance description (optional).
        #[arg(short, long)]
        description: Option<String>,
        /// Cloud region.
        #[arg(short, long)]
        region: Option<String>,
        /// Variable overrides (KEY=VALUE).
        #[arg(long = "var")]
        var_overrides: Vec<String>,
        /// Skip confirmations (CI/CD mode). Hard-fails if secrets are missing.
        #[arg(long)]
        yes: bool,
    },
    /// List running instances.
    List,
    /// Permanently destroy an instance and all its runtime state.
    Destroy {
        /// Instance ID.
        id: String,
        /// Skip confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Stop a running instance (preserves disk).
    Stop {
        /// Instance ID.
        id: String,
    },
    /// Start a stopped instance.
    Start {
        /// Instance ID.
        id: String,
    },
    /// Restart a running or stopped instance.
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

// ── Secret subcommands ──────────────────────────────────────────────────────

#[derive(Subcommand)]
enum SecretCommand {
    /// Set a secret value (prompted interactively if --value is omitted).
    Set {
        /// Secret key name (e.g. ANTHROPIC_API_KEY).
        key: String,
        /// Secret value (if omitted, you'll be prompted).
        #[arg(long)]
        value: Option<String>,
        /// Make this secret agent-specific (default: user-global).
        #[arg(long)]
        agent: Option<String>,
    },
    /// List all secrets (keys and hints only, never values).
    List {
        /// Filter to secrets for a specific agent.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Delete a secret by key.
    Delete {
        /// Secret key name.
        key: String,
        /// Delete agent-specific secret (default: user-global).
        #[arg(long)]
        agent: Option<String>,
        /// Skip confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Import secrets from a .env file (interactive).
    Import {
        /// Path to .env file (default: .env in workspace root).
        path: Option<String>,
    },
}

// ── Auth subcommands ───────────────────────────────────────────────────────

#[derive(Subcommand)]
enum AuthCommand {
    /// Log in via the Spawner web UI (opens browser).
    Login {
        /// Spawner URL (env: STARPOD_URL).
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

Given a natural language request, generate a skill definition with these fields:
- **name**: A concise, lowercase identifier using only letters, digits, and hyphens (max 64 chars). Must not start/end with a hyphen or contain consecutive hyphens.
- **description**: 1-2 sentences explaining what the skill does AND when to use it. Use imperative phrasing ("Use this skill when..."). Be "pushy" — explicitly list contexts where the skill applies, including indirect mentions. Max 1024 chars.
- **body**: Markdown instructions the agent follows when the skill is activated. Under 500 lines.
- **env** (optional): Environment requirements. Include ONLY when the skill genuinely needs external API access or user-configurable settings. Do NOT add env for skills that only use built-in tools.
  - `secrets`: API keys, tokens, or credentials the skill needs. Each key maps to `{required: bool, description: string}`. Use UPPER_SNAKE_CASE for key names.
  - `variables`: Configurable settings with sensible defaults. Each key maps to `{default: string, description: string}`. Use UPPER_SNAKE_CASE for key names.

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

Return a JSON object with: `name`, `description`, `body`, and optionally `env`.
"#;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Strip optional markdown code fences from an AI response that should contain
/// raw JSON.  Handles ` ```json ... ``` `, bare ` ``` ... ``` `, and plain JSON.
fn strip_json_fence(raw: &str) -> &str {
    let s = raw.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    s.strip_suffix("```").unwrap_or(s).trim()
}

/// Call the LLM to generate skill description + body + optional env.
async fn generate_skill_body(
    name: &str,
    description: &Option<String>,
    prompt: &Option<String>,
) -> anyhow::Result<(String, String, Option<starpod_skills::SkillEnv>)> {
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
            },
            "env": {
                "type": "object",
                "description": "Environment requirements. Include ONLY when the skill needs external API keys or configurable variables. Omit entirely for skills that only use built-in tools.",
                "properties": {
                    "secrets": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "object",
                            "properties": {
                                "required": { "type": "boolean" },
                                "description": { "type": "string" }
                            }
                        }
                    },
                    "variables": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "object",
                            "properties": {
                                "default": { "type": "string" },
                                "description": { "type": "string" }
                            }
                        }
                    }
                }
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
        if let agent_sdk::Message::Result(result) = msg {
            result_msg = Some(result);
        }
    }

    let result = result_msg
        .ok_or_else(|| anyhow::anyhow!("No result from AI"))?;

    if result.is_error {
        anyhow::bail!("{}", result.errors.join("; "));
    }

    let result_text = result.result.ok_or_else(|| {
        anyhow::anyhow!("No text returned from AI")
    })?;

    #[derive(serde::Deserialize)]
    struct SkillGen {
        description: String,
        body: String,
        env: Option<starpod_skills::SkillEnv>,
    }

    let json_str = strip_json_fence(&result_text);
    let gen: SkillGen = serde_json::from_str(json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse AI response as JSON: {e}"))?;

    let env = gen.env.filter(|e| !e.is_empty());
    Ok((gen.description, gen.body, env))
}

/// Generate deploy.toml into the agent directory by scanning agent.toml + skills.
fn generate_deploy_manifest(
    agent_dir: &std::path::Path,
    skills_dir: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    // Parse agent.toml for model/channel info
    let agent_toml_path = agent_dir.join("agent.toml");
    let (models, telegram_enabled, internet_enabled) = if agent_toml_path.exists() {
        let content = std::fs::read_to_string(&agent_toml_path)?;
        let table: toml::Value = toml::from_str(&content)?;

        let models = table.get("models")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
            .unwrap_or_default();

        let telegram_enabled = table.get("channels")
            .and_then(|c| c.get("telegram"))
            .and_then(|t| t.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let internet_enabled = table.get("internet")
            .and_then(|i| i.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true); // enabled by default

        (models, telegram_enabled, internet_enabled)
    } else {
        (vec!["anthropic/claude-sonnet-4-6".to_string()], false, true)
    };

    // Collect skill env declarations
    let skill_envs = if let Some(sd) = skills_dir.filter(|p| p.exists()) {
        let store = starpod_skills::SkillStore::new(sd)?;
        store.collect_env_by_skill()?
            .into_iter()
            .map(|(name, env)| SkillEnvInput {
                name,
                secrets: env.secrets.into_iter()
                    .map(|(k, v)| (k, v.required, v.description))
                    .collect(),
                variables: env.variables.into_iter()
                    .map(|(k, v)| (k, v.default, v.description))
                    .collect(),
            })
            .collect()
    } else {
        vec![]
    };

    let config = AgentConfigInput { models, telegram_enabled, internet_enabled };
    let deploy_path = agent_dir.join("deploy.toml");
    DeployManifest::generate_and_write(&config, skill_envs, &deploy_path)?;
    Ok(())
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
    // Read models array from workspace config, falling back to provider/model args
    let effective_models: Vec<String> = ws_config
        .as_ref()
        .and_then(|v| v.get("models"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_else(|| vec![format!("{provider}/{model}")]);
    let effective_max_turns = ws_int("max_turns", 30);
    let effective_server_addr = ws_str("server_addr", "127.0.0.1:3000");

    // Format models array as TOML inline array
    let models_toml = format!(
        "[{}]",
        effective_models
            .iter()
            .map(|m| format!("\"{}\"", m))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let agent_toml = format!(
        r#"# Agent configuration for {name}
# This file is self-contained — all settings are here (not inherited from starpod.toml).

agent_name = "{display_name}"
models = {models}
max_turns = {max_turns}
server_addr = "{server_addr}"
# skills = []  # empty = all workspace skills

# max_tokens = 16384
# reasoning_effort = "low"  # low, medium, high
# compaction_model = "{compaction_model}"
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
        models = models_toml,
        max_turns = effective_max_turns,
        server_addr = effective_server_addr,
        compaction_model = effective_models.first().map(|s| s.as_str()).unwrap_or("anthropic/claude-haiku-4-5"),
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

/// Build the cron notification sender from config.
///
/// The Telegram bot itself is managed by the gateway (started/restarted
/// via `AppState::restart_telegram`).
fn build_cron_notifier(
    config: &StarpodConfig,
) -> Option<starpod_cron::NotificationSender> {
    let telegram_token = config.resolved_telegram_token();

    if let Some(ref token) = telegram_token {
        let token = token.clone();
        Some(Arc::new(move |_job_name, _session_id, result_text, _success| {
            let token = token.clone();
            Box::pin(async move {
                // Cron notifications go to all linked telegram users
                // For now, just log — full notification routing will be added later
                tracing::debug!("Cron notification: {}", &result_text[..result_text.len().min(100)]);
                let _ = token;
            })
        }))
    } else {
        None
    }
}

// ── Sync diff display ──────────────────────────────────────────────────────

fn print_sync_diff(diff: &starpod_instances::deploy::SyncManifestResponse, format: OutputFormat, context: &str) {
    if format == OutputFormat::Json {
        let json = serde_json::json!({
            "to_upload": diff.to_upload,
            "to_download": diff.to_download.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "to_delete_remote": diff.to_delete_remote,
            "to_delete_local": diff.to_delete_local,
        });
        println!("{}", serde_json::to_string_pretty(&json).unwrap());
        return;
    }

    let total_changes = diff.to_upload.len()
        + diff.to_download.len()
        + diff.to_delete_remote.len()
        + diff.to_delete_local.len();

    if total_changes == 0 {
        println!("  {} Everything is in sync.", "✓".green().bold());
        return;
    }

    if context == "push" {
        println!("  Dry run — no files will be modified.\n");
    }

    if !diff.to_upload.is_empty() {
        println!("  Local → Remote (would change on push):");
        for path in &diff.to_upload {
            println!("    {} modified  {}", "↑".bright_green(), path);
        }
        println!();
    }

    if !diff.to_download.is_empty() {
        println!("  Remote → Local (would change on pull):");
        for file in &diff.to_download {
            println!("    {} modified  {}", "↓".bright_cyan(), file.path);
        }
        println!();
    }

    if !diff.to_delete_remote.is_empty() {
        println!("  Only on local (would delete from remote on push):");
        for path in &diff.to_delete_remote {
            println!("    {} removed   {}", "-".bright_red(), path);
        }
        println!();
    }

    if !diff.to_delete_local.is_empty() {
        println!("  Only on remote (would delete locally on pull):");
        for path in &diff.to_delete_local {
            println!("    {} removed   {}", "-".bright_red(), path);
        }
        println!();
    }

    println!(
        "  Summary: {} to upload, {} to download, {} remote-only, {} local-only",
        diff.to_upload.len().to_string().bright_green(),
        diff.to_download.len().to_string().bright_cyan(),
        diff.to_delete_remote.len().to_string().bright_red(),
        diff.to_delete_local.len().to_string().bright_red(),
    );
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
    let output_format = cli.format;

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

            let (provider, model, api_key, brave_api_key, first_agent, agent_display) = match answers {
                Some(a) => (
                    a.provider,
                    a.model,
                    a.api_key,
                    a.brave_api_key,
                    a.first_agent_name,
                    a.agent_display_name,
                ),
                None => (
                    "anthropic".to_string(),
                    "claude-haiku-4-5".to_string(),
                    None,
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
                onboarding::generate_env_content_full(&provider, api_key.as_deref(), brave_api_key.as_deref()),
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

                let start_now = dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                    .with_prompt("Start the agent now?")
                    .default(true)
                    .interact()
                    .unwrap_or(false);

                if start_now {
                    println!();
                    let exe = std::env::current_exe().unwrap_or_else(|_| "starpod".into());
                    let mut cmd = std::process::Command::new(&exe);
                    cmd.arg("dev").arg(&agent_name);
                    let status = cmd.status()?;
                    std::process::exit(status.code().unwrap_or(1));
                } else {
                    println!(
                        "  {} Run {} to start.",
                        "→".dimmed(),
                        format!("starpod dev {}", agent_name).bright_white()
                    );
                }
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
                    if output_format == OutputFormat::Json {
                        println!("[]");
                    } else {
                        println!("  No agents/ directory.");
                    }
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

                if output_format == OutputFormat::Json {
                    let json: Vec<serde_json::Value> = agents.iter().map(|name| {
                        let agent_toml = agents_dir.join(name).join("agent.toml");
                        serde_json::json!({
                            "name": name,
                            "has_config": agent_toml.is_file(),
                        })
                    }).collect();
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                } else if agents.is_empty() {
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

            AgentCommand::Push { name, dry_run, yes } => {
                // Resolve auth + workspace (same pattern as deploy)
                let saved = auth::load_credentials();
                let backend_url = std::env::var(auth::SPAWNER_URL_ENV)
                    .ok()
                    .or_else(|| saved.as_ref().map(|c| c.backend_url.clone()))
                    .unwrap_or_else(|| auth::DEFAULT_SPAWNER_URL.to_string());
                let api_key = std::env::var("STARPOD_API_KEY")
                    .ok()
                    .or_else(|| saved.as_ref().map(|c| c.api_key.clone()));
                let Some(api_key) = api_key else {
                    eprintln!("  {} Authentication required. Run {}.", "✗".red().bold(), "starpod auth login".bright_white());
                    std::process::exit(1);
                };

                let cwd = std::env::current_dir()?;
                let workspace_root = find_workspace_root(&cwd).unwrap_or_else(|| {
                    eprintln!("  {} Not inside a starpod workspace.", "✗".red().bold());
                    std::process::exit(1);
                });

                let agent_dir = workspace_root.join("agents").join(&name);
                if !agent_dir.exists() {
                    eprintln!("  {} Agent '{}' not found in agents/ directory.", "✗".red().bold(), name);
                    std::process::exit(1);
                }

                let skills_dir = workspace_root.join("skills");
                let skills_path = if skills_dir.exists() { Some(skills_dir.as_path()) } else { None };

                // Generate deploy.toml before pushing
                generate_deploy_manifest(&agent_dir, skills_path)?;

                let client = DeployClient::new(&backend_url, &api_key)?;

                if dry_run {
                    // Dry run: compute diff and display without pushing
                    println!("  {} Computing diff for {}...", "⟳".bright_cyan(), name.bright_white().bold());
                    let diff = client.diff_agent(&name, &agent_dir, skills_path).await?;
                    print_sync_diff(&diff, output_format, "push");
                } else {
                    // Compute diff first for confirmation
                    let diff = client.diff_agent(&name, &agent_dir, skills_path).await?;

                    // Show and confirm if there are deletions
                    if !diff.to_delete_remote.is_empty() && !yes {
                        println!("\n  Changes:");
                        if !diff.to_upload.is_empty() {
                            println!("    {} {} file(s) to upload", "↑".bright_green(), diff.to_upload.len());
                        }
                        for path in &diff.to_delete_remote {
                            println!("    {} delete  {}", "-".bright_red(), path);
                        }
                        if !confirm("Push these changes?", false) {
                            eprintln!("  {} Aborted.", "✗".red());
                            std::process::exit(0);
                        }
                    }

                    println!("  {} Pushing agent {}...", "⟳".bright_cyan(), name.bright_white().bold());
                    let summary = client.push_agent(&name, &agent_dir, skills_path).await?;

                    if output_format == OutputFormat::Json {
                        let json = serde_json::json!({
                            "uploaded": summary.uploaded,
                            "deleted_remote": summary.deleted_remote,
                            "unchanged": summary.unchanged,
                        });
                        println!("{}", serde_json::to_string_pretty(&json).unwrap());
                    } else {
                        println!("  {} Push complete:", "✓".green().bold());
                        println!("  {} {} uploaded, {} deleted, {} unchanged",
                            "│".dimmed(),
                            summary.uploaded.to_string().bright_green(),
                            summary.deleted_remote.to_string().bright_red(),
                            summary.unchanged.to_string().dimmed(),
                        );
                    }
                }
            }

            AgentCommand::Pull { name } => {
                let saved = auth::load_credentials();
                let backend_url = std::env::var(auth::SPAWNER_URL_ENV)
                    .ok()
                    .or_else(|| saved.as_ref().map(|c| c.backend_url.clone()))
                    .unwrap_or_else(|| auth::DEFAULT_SPAWNER_URL.to_string());
                let api_key = std::env::var("STARPOD_API_KEY")
                    .ok()
                    .or_else(|| saved.as_ref().map(|c| c.api_key.clone()));
                let Some(api_key) = api_key else {
                    eprintln!("  {} Authentication required. Run {}.", "✗".red().bold(), "starpod auth login".bright_white());
                    std::process::exit(1);
                };

                let cwd = std::env::current_dir()?;
                let workspace_root = find_workspace_root(&cwd).unwrap_or_else(|| {
                    eprintln!("  {} Not inside a starpod workspace.", "✗".red().bold());
                    std::process::exit(1);
                });

                let agent_dir = workspace_root.join("agents").join(&name);

                let client = DeployClient::new(&backend_url, &api_key)?;
                println!("  {} Pulling agent {}...", "⟳".bright_cyan(), name.bright_white().bold());

                let summary = client.pull_agent(&name, &agent_dir).await?;

                if output_format == OutputFormat::Json {
                    let json = serde_json::json!({
                        "downloaded": summary.downloaded,
                        "deleted_local": summary.deleted_local,
                        "unchanged": summary.unchanged,
                    });
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                } else {
                    println!("  {} Pull complete:", "✓".green().bold());
                    println!("  {} {} downloaded, {} deleted locally, {} unchanged",
                        "│".dimmed(),
                        summary.downloaded.to_string().bright_green(),
                        summary.deleted_local.to_string().bright_red(),
                        summary.unchanged.to_string().dimmed(),
                    );
                }
            }

            AgentCommand::Diff { name } => {
                let saved = auth::load_credentials();
                let backend_url = std::env::var(auth::SPAWNER_URL_ENV)
                    .ok()
                    .or_else(|| saved.as_ref().map(|c| c.backend_url.clone()))
                    .unwrap_or_else(|| auth::DEFAULT_SPAWNER_URL.to_string());
                let api_key = std::env::var("STARPOD_API_KEY")
                    .ok()
                    .or_else(|| saved.as_ref().map(|c| c.api_key.clone()));
                let Some(api_key) = api_key else {
                    eprintln!("  {} Authentication required. Run {}.", "✗".red().bold(), "starpod auth login".bright_white());
                    std::process::exit(1);
                };

                let cwd = std::env::current_dir()?;
                let workspace_root = find_workspace_root(&cwd).unwrap_or_else(|| {
                    eprintln!("  {} Not inside a starpod workspace.", "✗".red().bold());
                    std::process::exit(1);
                });

                let agent_dir = workspace_root.join("agents").join(&name);
                if !agent_dir.exists() {
                    eprintln!("  {} Agent '{}' not found in agents/ directory.", "✗".red().bold(), name);
                    std::process::exit(1);
                }

                let skills_dir = workspace_root.join("skills");
                let skills_path = if skills_dir.exists() { Some(skills_dir.as_path()) } else { None };

                let client = DeployClient::new(&backend_url, &api_key)?;
                let diff = client.diff_agent(&name, &agent_dir, skills_path).await?;
                print_sync_diff(&diff, output_format, "diff");
            }

        },

        // ── Dev ──────────────────────────────────────────────────────
        Commands::Dev { agent: agent_name, port, build } => {
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

            let blueprint_dir = workspace_root.join("agents").join(&agent_name);
            let instance_dir = workspace_root.join(".instances").join(&agent_name);

            // Validate blueprint exists
            if !blueprint_dir.join("agent.toml").is_file() {
                eprintln!(
                    "Error: No blueprint found for agent '{}'. Run `starpod agent new {}` first.",
                    agent_name, agent_name
                );
                std::process::exit(1);
            }

            // Apply blueprint only on first run or when --build is passed.
            let instance_exists = instance_dir
                .join(".starpod")
                .join("config")
                .join("agent.toml")
                .is_file();

            if !instance_exists || build {
                // Generate deploy.toml BEFORE building so we can validate
                let skills_dir = workspace_root.join("skills");
                let skills_path = if skills_dir.exists() { Some(skills_dir.as_path()) } else { None };
                generate_deploy_manifest(&blueprint_dir, skills_path)?;

                // Validate .env BEFORE creating instance — fail fast, no stale .starpod/
                let deploy_toml = blueprint_dir.join("deploy.toml");
                let env_file = workspace_root.join(".env");
                let env_path = if env_file.exists() { Some(env_file.as_path()) } else { None };
                match starpod_vault::env::validate_env(&deploy_toml, env_path) {
                    Ok(warnings) => {
                        for w in &warnings {
                            println!("  {} {}", "⚠".yellow(), w);
                        }
                    }
                    Err(e) => {
                        eprintln!("  {} {}", "✗".red().bold(), e);
                        std::process::exit(1);
                    }
                }

                // Now safe to create the instance
                starpod_core::apply_blueprint(
                    &blueprint_dir,
                    &instance_dir,
                    &workspace_root,
                    starpod_core::EnvSource::Dev,
                )?;
            } else {
                println!(
                    "  {} Using existing instance. Pass {} to rebuild from blueprint.",
                    "ℹ".bright_cyan().bold(),
                    "--build".bright_white()
                );
            }

            // Resolve paths as Instance mode
            let instance_mode = starpod_core::Mode::Instance {
                instance_root: instance_dir.clone(),
                agent_name: agent_name.clone(),
            };
            let paths = starpod_core::ResolvedPaths::resolve(&instance_mode)?;

            // Populate vault from .env + deploy.toml, then inject into process env
            let deploy_toml = blueprint_dir.join("deploy.toml");
            let env_file = workspace_root.join(".env");
            if deploy_toml.exists() {
                let env_path = if env_file.exists() { Some(env_file.as_path()) } else { None };
                let master_key = starpod_vault::derive_master_key(&paths.db_dir)?;
                let vault = starpod_vault::Vault::new(&paths.db_dir.join("vault.db"), &master_key).await?;
                starpod_vault::env::populate_vault(&deploy_toml, env_path, &vault).await?;
                starpod_vault::env::inject_env_from_vault(&deploy_toml, &vault).await?;
            }

            // In dev mode, load STARPOD_API_KEY from .env into process env so
            // that bootstrap_admin uses the configured key rather than generating
            // a random one. (In prod this is handled by build-time pre-seeding.)
            let dev_api_key = if env_file.exists() {
                let env_map = parse_env_file(&env_file)?;
                if let Some(api_key) = env_map.get("STARPOD_API_KEY") {
                    std::env::set_var("STARPOD_API_KEY", api_key);
                    Some(api_key.clone())
                } else {
                    None
                }
            } else {
                None
            };

            let mut agent_config = starpod_core::load_agent_config(&paths)?;

            // Override port if specified
            if let Some(p) = port {
                agent_config.server_addr = format!("127.0.0.1:{}", p);
            }

            let config = agent_config.clone().into_starpod_config(&paths);
            let addr = config.server_addr.clone();
            let display_name = config.agent_name.clone();

            let agent = Arc::new(StarpodAgent::with_paths(agent_config, paths.clone()).await?);

            // Bootstrap auth store early so Telegram bot + browser auto-login work
            let auth_bootstrap = starpod_gateway::create_auth_store(&paths).await?;
            let auth_store = auth_bootstrap.store.clone();
            let cron_notifier = build_cron_notifier(&config);

            // Resolve the token for the browser: prefer freshly-bootstrapped key,
            // fall back to the .env key (covers subsequent runs where admin exists).
            let browser_token = auth_bootstrap.admin_key.as_ref().or(dev_api_key.as_ref());

            print_header_with_name(&display_name);
            println!("  {} {} → {}", "DEV".bright_yellow().bold(), agent_name.bright_cyan(), instance_dir.display().to_string().dimmed());
            println!("  {} {}", "Server".dimmed(), addr.bright_green());
            if let Some(ref key) = browser_token {
                println!("  {} {}", "API Key".dimmed(), key.bright_cyan());
            }
            print_separator();

            // Open browser with auto-login token if we have one
            if let Some(key) = browser_token {
                let _ = open::that(format!("http://{}?token={}", addr, key));
            } else {
                let _ = open::that(format!("http://{}", addr));
            }

            starpod_gateway::serve_with_agent(agent, config, cron_notifier, paths, Some(auth_store)).await?;
        }

        // ── Serve ─────────────────────────────────────────────────────
        Commands::Serve { agent: agent_name } => {
            // Populate vault from .env + deploy.toml, then inject into process env
            {
                let mode = detect_mode(agent_name.as_deref())?;
                let paths = starpod_core::ResolvedPaths::resolve(&mode)?;
                let deploy_toml = paths.config_dir.join("deploy.toml");
                if deploy_toml.exists() {
                    // Resolve .env from project root
                    let env_file = paths.project_root.join(".env");
                    let env_path = if env_file.exists() { Some(env_file.as_path()) } else { None };
                    let master_key = starpod_vault::derive_master_key(&paths.db_dir)?;
                    let vault = starpod_vault::Vault::new(&paths.db_dir.join("vault.db"), &master_key).await?;
                    starpod_vault::env::populate_vault(&deploy_toml, env_path, &vault).await?;
                    starpod_vault::env::inject_env_from_vault(&deploy_toml, &vault).await?;
                }
            }

            let (agent, config, paths) = resolve_agent(agent_name).await?;
            let addr = config.server_addr.clone();
            let display_name = config.agent_name.clone();
            let telegram_enabled = config.channels.telegram.as_ref().map_or(false, |t| t.enabled);
            let telegram_active = telegram_enabled && config.resolved_telegram_token().is_some();
            let agent = Arc::new(agent);
            let auth = starpod_gateway::create_auth_store(&paths).await.ok().map(|b| b.store);
            let cron_notifier = build_cron_notifier(&config);

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
                config.provider().bright_white()
            );
            println!(
                "  {} {}",
                "Model   ".dimmed(),
                config.model().bright_white()
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

            starpod_gateway::serve_with_agent(agent, config, cron_notifier, paths, auth).await?;
        }

        // ── Chat ──────────────────────────────────────────────────────
        Commands::Chat { agent: agent_name, message } => {
            // Try resolving an existing agent; if none found, create an ephemeral one
            let _ephemeral_guard: Option<tempfile::TempDir>;
            let (agent, config, _paths) = match resolve_agent(agent_name).await {
                Ok(resolved) => {
                    _ephemeral_guard = None;
                    resolved
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
            let (mut stream, session_id, _followup_tx) = agent.chat_stream(&chat_msg).await?;
            let (result_text, result_msg) = process_stream(&mut stream, &start).await?;

            if let Some(ref result) = result_msg {
                agent
                    .finalize_chat(&session_id, &message, &result_text, result, None)
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
            // Also load .env so provider API keys are available for `skill new`.
            let cwd = std::env::current_dir()?;
            let skills_dir = if let Some(ref name) = agent_name {
                // Workspace mode: resolve to instance skills
                let mode = starpod_core::detect_mode_from(Some(name), &cwd)?;
                let paths = starpod_core::ResolvedPaths::resolve(&mode)?;
                let _ = starpod_core::load_agent_config(&paths);
                paths.skills_dir
            } else {
                // Try to detect mode automatically
                match starpod_core::detect_mode(None) {
                    Ok(mode) => {
                        let paths = starpod_core::ResolvedPaths::resolve(&mode)?;
                        let _ = starpod_core::load_agent_config(&paths);
                        paths.skills_dir
                    }
                    Err(_) => {
                        // Fallback: workspace skills/ or .starpod/skills/
                        // Also load .env so provider API keys are available for `skill new`
                        if cwd.join("starpod.toml").is_file() {
                            starpod_core::load_env(&cwd, None);
                            cwd.join("skills")
                        } else if cwd.join(".starpod").is_dir() {
                            starpod_core::load_env(&cwd.join(".starpod"), None);
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
                    let version: String = Input::with_theme(&theme)
                        .with_prompt("Version")
                        .default("0.1.0".to_string())
                        .interact_text()?;
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

                    println!(
                        "  {} Generating skill '{}'...\n",
                        "⚡".bright_yellow(),
                        name.bright_white()
                    );

                    // Try AI generation; fall back to a stub skill on any LLM error.
                    let (skill_desc, skill_body, skill_env) = match generate_skill_body(&name, &description, &prompt).await {
                        Ok((desc, body, env)) => {
                            let desc = description.unwrap_or(desc);
                            (desc, body, env)
                        }
                        Err(e) => {
                            eprintln!(
                                "  {} AI generation failed: {}\n  {} Creating skill with name and description only.\n",
                                "⚠".bright_yellow(),
                                e,
                                "→".dimmed(),
                            );
                            let desc = description.unwrap_or_else(|| format!("Skill: {}", name));
                            (desc, String::new(), None)
                        }
                    };

                    let version_opt = if version.is_empty() { None } else { Some(version.as_str()) };
                    store.create(&name, &skill_desc, version_opt, skill_env.as_ref(), &skill_body)?;

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
                    if !skill_body.is_empty() {
                        println!();
                        println!("{}", skill_body);
                    }
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
                                    triggered_by: Some(j.name.clone()),
                                    model: None,
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

        // ── Auth commands ─────────────────────────────────────────────
        Commands::Auth { action } => {
            match action {
                AuthCommand::Login { url, api_key, email } => {
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
                        // Non-interactive login with API key (CI/headless)
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
                        // Interactive browser-based login
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

                AuthCommand::Logout => {
                    match auth::delete_credentials() {
                        Ok(()) => {
                            println!(
                                "  {} Logged out. Credentials removed.",
                                "✓".green().bold()
                            );
                        }
                        Err(e) => {
                            eprintln!("  {} {}", "✗".red().bold(), e);
                            std::process::exit(1);
                        }
                    }
                }

                AuthCommand::Status => {
                    match auth::load_credentials() {
                        Some(creds) => {
                            if output_format == OutputFormat::Json {
                                let preview = if creds.api_key.len() > 12 {
                                    format!("{}...{}", &creds.api_key[..8], &creds.api_key[creds.api_key.len()-4..])
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
                                    format!("{}...{}", &creds.api_key[..8], &creds.api_key[creds.api_key.len()-4..])
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
                    }
                }
            }
        }

        // ── Instance commands ──────────────────────────────────────────
        Commands::Instance { action } => {
            let saved = auth::load_credentials();
            let backend_url = std::env::var(auth::SPAWNER_URL_ENV)
                .ok()
                .or_else(|| saved.as_ref().map(|c| c.backend_url.clone()))
                .unwrap_or_else(|| auth::DEFAULT_SPAWNER_URL.to_string());

            let api_key = std::env::var("STARPOD_API_KEY")
                .ok()
                .or_else(|| saved.as_ref().map(|c| c.api_key.clone()));
            let client = InstanceClient::new_with_timeout(&backend_url, api_key, 30)?;

            match action {
                InstanceCommand::New { agent: agent_name, name, description, region, var_overrides, yes } => {
                    // Need the DeployClient for the deploy-config API
                    let api_key_str = std::env::var("STARPOD_API_KEY")
                        .ok()
                        .or_else(|| saved.as_ref().map(|c| c.api_key.clone()));
                    let api_key_str = match api_key_str {
                        Some(k) => k,
                        None => {
                            eprintln!("  {} Not authenticated. Run: starpod auth login", "✗".red());
                            std::process::exit(1);
                        }
                    };
                    let deploy_client = DeployClient::new(&backend_url, &api_key_str)?;

                    // Step 1: Find or push the agent
                    let agent_name = match agent_name {
                        Some(n) => n,
                        None => {
                            eprintln!("  {} Please specify --agent <name>", "✗".red());
                            std::process::exit(1);
                        }
                    };

                    println!("  {} Checking agent {}...", "⟳".bright_cyan(), agent_name.bright_white());

                    let agents = deploy_client.list_agents().await?;
                    let agent = match agents.iter().find(|a| a.name == agent_name) {
                        Some(a) => {
                            println!("  {} Agent found on remote", "✓".green());
                            a.clone()
                        }
                        None => {
                            // Try to push local blueprint
                            let ws_agents_dir = std::path::PathBuf::from("agents").join(&agent_name);
                            if !ws_agents_dir.join("agent.toml").exists() {
                                eprintln!("  {} Agent '{}' not found locally or on remote.", "✗".red(), agent_name);
                                std::process::exit(1);
                            }

                            if !yes {
                                eprint!("  Agent '{}' not found on remote. Push local blueprint? [Y/n] ", agent_name);
                                let mut buf = String::new();
                                std::io::stdin().read_line(&mut buf)?;
                                let answer = buf.trim().to_lowercase();
                                if !answer.is_empty() && answer != "y" && answer != "yes" {
                                    eprintln!("  {} Aborted.", "✗".red());
                                    std::process::exit(1);
                                }
                            }

                            println!("  {} Pushing blueprint...", "⟳".bright_cyan());
                            let summary = deploy_client.push_agent(&agent_name, &ws_agents_dir, None).await?;
                            println!("  {} Agent created ({} files pushed)", "✓".green().bold(), summary.uploaded);

                            let agents = deploy_client.list_agents().await?;
                            agents.into_iter().find(|a| a.name == agent_name).unwrap()
                        }
                    };

                    // Step 2: Check deploy.toml readiness
                    let deploy_config = deploy_client.get_deploy_config(&agent.id).await?;

                    if let Some(ref config) = deploy_config {
                        println!("\n  {} Validating deploy.toml...", "⟳".bright_cyan());

                        // Show secrets status
                        println!("\n  Secrets:");
                        for s in &config.secrets {
                            let status = if s.present {
                                let scope = s.scope.as_deref().unwrap_or("unknown");
                                let hint = s.hint.as_deref().unwrap_or("****");
                                format!("{} ({}, hint: ...{})", "✓".green(), scope, hint)
                            } else if s.required {
                                format!("{} missing ({})", "✗".red(), "required".bright_red())
                            } else {
                                format!("{} missing ({})", "–".dimmed(), "optional".dimmed())
                            };
                            println!("    {:<24} {}", s.key, status);
                        }

                        // Parse local .env once for both required and optional secret prompts
                        let env_path = std::path::PathBuf::from(".env");
                        let local_env = if env_path.exists() {
                            Some(parse_env_file(&env_path)?)
                        } else {
                            None
                        };

                        // Handle missing required secrets
                        if !config.missing_required.is_empty() {
                            println!(
                                "\n  {} {} required secret(s) missing: {}",
                                "⚠".yellow(),
                                config.missing_required.len(),
                                config.missing_required.join(", ")
                            );

                            // Try to find them in local .env
                            if local_env.is_some() && !yes {
                                let local_env = local_env.as_ref().unwrap();
                                let pushable = config.missing_required_in_env(local_env);

                                if !pushable.is_empty() {
                                    println!(
                                        "\n  Found in local .env: {}",
                                        pushable.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
                                    );
                                    for key in &pushable {
                                        let value = &local_env[key.as_str()];
                                        eprint!("  ? Push {} to remote as agent-specific secret? [Y/n] ", key.bright_white());
                                        let mut buf = String::new();
                                        std::io::stdin().read_line(&mut buf)?;
                                        let answer = buf.trim().to_lowercase();
                                        if answer.is_empty() || answer == "y" || answer == "yes" {
                                            match deploy_client.set_secret(&agent.id, key, value).await {
                                                Ok(s) => {
                                                    println!(
                                                        "    {} {} pushed (hint: ••••{})",
                                                        "✓".green(),
                                                        key,
                                                        s.hint.as_deref().unwrap_or("****")
                                                    );
                                                }
                                                Err(e) => {
                                                    eprintln!("    {} Failed to push {}: {}", "✗".red(), key, e);
                                                }
                                            }
                                        }
                                    }

                                    // Re-check readiness
                                    let recheck = deploy_client.get_deploy_config(&agent.id).await?;
                                    if let Some(ref rc) = recheck {
                                        if !rc.ready {
                                            eprintln!(
                                                "\n  {} Cannot create instance: still missing required secrets: {}",
                                                "✗".red().bold(),
                                                rc.missing_required.join(", ")
                                            );
                                            eprintln!("  Run: starpod secret set <KEY> --agent {}", agent_name);
                                            std::process::exit(1);
                                        }
                                    }
                                } else {
                                    eprintln!("\n  {} Cannot create instance: {} required secret(s) missing.", "✗".red().bold(), config.missing_required.len());
                                    eprintln!("  Run: starpod secret set <KEY> --agent {}", agent_name);
                                    std::process::exit(1);
                                }
                            } else if yes {
                                // CI mode: hard fail, never auto-push
                                eprintln!(
                                    "\n  {} Cannot create instance: {} required secret(s) missing from deploy.toml: {}",
                                    "✗".red().bold(),
                                    config.missing_required.len(),
                                    config.missing_required.join(", ")
                                );
                                std::process::exit(1);
                            } else {
                                eprintln!("\n  {} Cannot create instance: {} required secret(s) missing.", "✗".red().bold(), config.missing_required.len());
                                eprintln!("  Run: starpod secret set <KEY> --agent {}", agent_name);
                                eprintln!("  Or create a .env file with the missing values.");
                                std::process::exit(1);
                            }
                        } else {
                            println!("\n  {} All required secrets present.", "✓".green().bold());
                        }

                        // Handle optional secrets available in local .env
                        if !yes {
                            if let Some(ref local_env) = local_env {
                                let optional_pushable = config.missing_optional_in_env(local_env);

                                if !optional_pushable.is_empty() {
                                    println!("\n  Found optional secrets in local .env:");
                                    for key in &optional_pushable {
                                        let value = &local_env[key.as_str()];
                                        eprint!("  ? Push {} to remote? [y/N] ", key.bright_white());
                                        let mut buf = String::new();
                                        std::io::stdin().read_line(&mut buf)?;
                                        let answer = buf.trim().to_lowercase();
                                        if answer == "y" || answer == "yes" {
                                            match deploy_client.set_secret(&agent.id, key, value).await {
                                                Ok(resp) => {
                                                    println!(
                                                        "    {} {} pushed (hint: ••••{})",
                                                        "✓".green(),
                                                        key,
                                                        resp.hint.as_deref().unwrap_or("****")
                                                    );
                                                }
                                                Err(e) => {
                                                    eprintln!("    {} Failed to push {}: {}", "✗".red(), key, e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Show variables
                        if !config.variables.is_empty() {
                            println!("\n  Variables (from deploy.toml):");
                            for var in &config.variables {
                                let val = var.default.as_deref().unwrap_or("");
                                println!("    {} = \"{}\"", var.key.dimmed(), val);
                            }
                        }
                    } else {
                        println!(
                            "\n  {} No deploy configuration found for '{}'.",
                            "⚠".yellow(),
                            agent_name.bright_white()
                        );
                        println!(
                            "  Configure secrets and variables at:\n  {}",
                            format!("{}/agents/{}", backend_url, agent.id).bright_cyan().underline()
                        );
                        if !yes {
                            eprint!("\n  Continue without deploy configuration? [y/N] ");
                            let mut buf = String::new();
                            std::io::stdin().read_line(&mut buf)?;
                            let answer = buf.trim().to_lowercase();
                            if answer != "y" && answer != "yes" {
                                std::process::exit(0);
                            }
                        }
                    }

                    // Step 3: Parse variable overrides
                    let overrides: HashMap<String, String> = var_overrides
                        .iter()
                        .filter_map(|s| {
                            s.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                        })
                        .collect();

                    if !overrides.is_empty() {
                        println!("\n  Variable overrides:");
                        for (k, v) in &overrides {
                            println!("    {} = \"{}\" (override)", k, v);
                        }
                    }

                    // Step 4: Create the instance
                    println!("\n  {} Creating instance...", "⟳".bright_cyan());

                    let _variable_overrides = if overrides.is_empty() { None } else { Some(overrides) };

                    match deploy_client.create_instance(
                        &agent.id,
                        name.as_deref(),
                        description.as_deref(),
                        region.as_deref(),
                        None, // machine_type
                    ).await {
                        Ok(inst) => {
                            println!(
                                "  {} Instance created (id: {})",
                                "✓".green().bold(),
                                inst.id.bright_white()
                            );

                            // Wait for instance to become running (up to 15 min)
                            let last_status = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
                            let last_status_clone = last_status.clone();
                            let on_poll = move |inst: &starpod_instances::deploy::InstanceResponse| {
                                let mut last = last_status_clone.lock().unwrap();
                                if *last != inst.status {
                                    eprint!(
                                        "\r  {} Status: {}                    ",
                                        "⟳".bright_cyan(),
                                        inst.status.bright_yellow()
                                    );
                                    *last = inst.status.clone();
                                }
                            };

                            match deploy_client.wait_for_instance_ready(
                                &inst.id,
                                std::time::Duration::from_secs(900),
                                on_poll,
                            ).await {
                                Ok(ready) => {
                                    eprint!("\r                                                \r");
                                    println!("  {} Instance is running!", "✓".green().bold());
                                    println!("  {} Status: {}", "│".dimmed(), ready.status.bright_green());
                                    if let Some(ref ip) = ready.ip_address {
                                        println!("  {} IP:     {}", "│".dimmed(), ip.bright_white().bold());
                                    }
                                    if let Some(ref z) = ready.zone {
                                        println!("  {} Zone:   {}", "│".dimmed(), z);
                                    }
                                    if let Some(ref url) = ready.web_url {
                                        println!("  {} Web UI: {}", "│".dimmed(), url.bright_cyan().underline());
                                    }
                                    if let Some(ref key) = ready.starpod_api_key {
                                        println!("  {} Key:    {}", "│".dimmed(), key.dimmed());
                                    }
                                    if let Some(ref url) = ready.direct_url {
                                        println!();
                                        println!("  {} Open directly: {}", "→".bright_cyan().bold(), url.bright_cyan().underline());
                                    }
                                }
                                Err(e) => {
                                    eprint!("\r                                                \r");
                                    eprintln!("  {} Instance failed to reach running state: {}", "✗".red(), e);
                                    std::process::exit(1);
                                }
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
                            if output_format == OutputFormat::Json {
                                let json: Vec<serde_json::Value> = instances.iter().map(|inst| {
                                    serde_json::json!({
                                        "id": inst.id,
                                        "id_short": &inst.id[..8.min(inst.id.len())],
                                        "name": inst.name,
                                        "description": inst.description,
                                        "status": format!("{}", inst.status),
                                        "agent_id": inst.agent_id,
                                        "zone": inst.zone,
                                        "machine_type": inst.machine_type,
                                        "ip": inst.ip_address,
                                        "web_url": inst.web_url,
                                        "direct_url": inst.direct_url,
                                        "starpod_api_key": inst.starpod_api_key,
                                        "error": inst.error_message,
                                        "created_at": inst.created_at,
                                    })
                                }).collect();
                                println!("{}", serde_json::to_string_pretty(&json).unwrap());
                            } else if instances.is_empty() {
                                println!("  {} No instances found.", "ℹ".bright_cyan());
                            } else {
                                println!();
                                for inst in &instances {
                                    let id_short = &inst.id[..8.min(inst.id.len())];
                                    let agent_short = &inst.agent_id[..8.min(inst.agent_id.len())];
                                    let status_str = format!("{}", inst.status);
                                    let status_colored = match status_str.as_str() {
                                        "running" => status_str.bright_green(),
                                        "pending" | "provisioning" => status_str.bright_yellow(),
                                        "stopped" | "stopping" => status_str.dimmed(),
                                        "error" => status_str.bright_red(),
                                        _ => status_str.normal(),
                                    };
                                    if let Some(ref name) = inst.name {
                                        println!(
                                            "  {}  {}  {}",
                                            id_short.bright_white().bold(),
                                            name.bright_white(),
                                            status_colored,
                                        );
                                    } else {
                                        println!(
                                            "  {}  {}",
                                            id_short.bright_white().bold(),
                                            status_colored,
                                        );
                                    }
                                    println!(
                                        "  {}  Agent   {}",
                                        "│".dimmed(),
                                        agent_short,
                                    );
                                    if let Some(ref zone) = inst.zone {
                                        println!(
                                            "  {}  Zone    {}",
                                            "│".dimmed(),
                                            zone.dimmed(),
                                        );
                                    }
                                    if let Some(ref ip) = inst.ip_address {
                                        println!(
                                            "  {}  IP      {}",
                                            "│".dimmed(),
                                            ip,
                                        );
                                    }
                                    if let Some(ref url) = inst.direct_url {
                                        println!(
                                            "  {}  Web UI  {}",
                                            "│".dimmed(),
                                            url.bright_cyan().underline(),
                                        );
                                    }
                                    if let Some(ref err) = inst.error_message {
                                        println!(
                                            "  {}  Error   {}",
                                            "│".dimmed(),
                                            err.bright_red(),
                                        );
                                    }
                                    println!();
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} Failed to list instances: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Destroy { id, yes } => {
                    let id = client.resolve_id(&id).await.unwrap_or_else(|e| {
                        eprintln!("  {} {}", "✗".red(), e);
                        std::process::exit(1);
                    });
                    let id_short = &id[..8.min(id.len())];
                    if !confirm(
                        &format!(
                            "This will permanently destroy instance {} and all its runtime state.\n  All memory, sessions, and data will be lost. Continue?",
                            id_short
                        ),
                        yes,
                    ) {
                        eprintln!("  {} Aborted.", "✗".red());
                        std::process::exit(0);
                    }
                    match client.destroy_instance(&id).await {
                        Ok(()) => println!("  {} Destroyed instance {}.", "✓".green().bold(), id_short),
                        Err(e) => {
                            eprintln!("  {} Failed to destroy instance {}: {}", "✗".red(), id_short, e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Stop { id } => {
                    let id = client.resolve_id(&id).await.unwrap_or_else(|e| {
                        eprintln!("  {} {}", "✗".red(), e);
                        std::process::exit(1);
                    });
                    match client.stop_instance(&id).await {
                        Ok(()) => println!("  {} Stopped instance {}.", "✓".green().bold(), &id[..8.min(id.len())]),
                        Err(e) => {
                            eprintln!("  {} Failed to stop instance: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Start { id } => {
                    let id = client.resolve_id(&id).await.unwrap_or_else(|e| {
                        eprintln!("  {} {}", "✗".red(), e);
                        std::process::exit(1);
                    });
                    match client.start_instance(&id).await {
                        Ok(()) => println!("  {} Started instance {}.", "✓".green().bold(), &id[..8.min(id.len())]),
                        Err(e) => {
                            eprintln!("  {} Failed to start instance: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Restart { id } => {
                    let id = client.resolve_id(&id).await.unwrap_or_else(|e| {
                        eprintln!("  {} {}", "✗".red(), e);
                        std::process::exit(1);
                    });
                    match client.restart_instance(&id).await {
                        Ok(()) => println!("  {} Restarted instance {}.", "✓".green().bold(), &id[..8.min(id.len())]),
                        Err(e) => {
                            eprintln!("  {} Failed to restart instance: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }

                InstanceCommand::Logs { id, tail } => {
                    let id = client.resolve_id(&id).await.unwrap_or_else(|e| {
                        eprintln!("  {} {}", "✗".red(), e);
                        std::process::exit(1);
                    });
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
                    let id = client.resolve_id(&id).await.unwrap_or_else(|e| {
                        eprintln!("  {} {}", "✗".red(), e);
                        std::process::exit(1);
                    });
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
                    let id = client.resolve_id(&id).await.unwrap_or_else(|e| {
                        eprintln!("  {} {}", "✗".red(), e);
                        std::process::exit(1);
                    });
                    match client.get_health(&id).await {
                        Ok(health) => {
                            if output_format == OutputFormat::Json {
                                let json = serde_json::json!({
                                    "instance_id": id,
                                    "cpu_percent": health.cpu_percent,
                                    "memory_mb": health.memory_mb,
                                    "disk_mb": health.disk_mb,
                                    "uptime_secs": health.uptime_secs,
                                    "last_heartbeat": health.last_heartbeat,
                                });
                                println!("{}", serde_json::to_string_pretty(&json).unwrap());
                            } else {
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
                        }
                        Err(e) => {
                            eprintln!("  {} Failed to get health: {}", "✗".red(), e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }

        // ── Secret ────────────────────────────────────────────────────
        Commands::Secret { action } => {
            let saved = auth::load_credentials();
            let backend_url = std::env::var(auth::SPAWNER_URL_ENV)
                .ok()
                .or_else(|| saved.as_ref().map(|c| c.backend_url.clone()))
                .unwrap_or_else(|| auth::DEFAULT_SPAWNER_URL.to_string());
            let api_key = std::env::var("STARPOD_API_KEY")
                .ok()
                .or_else(|| saved.as_ref().map(|c| c.api_key.clone()));
            let api_key = match api_key {
                Some(k) => k,
                None => {
                    eprintln!("  {} Not authenticated. Run: starpod auth login", "✗".red());
                    std::process::exit(1);
                }
            };
            let deploy_client = DeployClient::new(&backend_url, &api_key)?;

            match action {
                SecretCommand::Set { key, value, agent: agent_name } => {
                    // Get value from --value flag or prompt interactively
                    let secret_value = match value {
                        Some(v) => v,
                        None => {
                            eprint!("  Enter value for {}: ", key.bright_white());
                            let mut buf = String::new();
                            std::io::stdin().read_line(&mut buf)?;
                            buf.trim().to_string()
                        }
                    };

                    if secret_value.is_empty() {
                        eprintln!("  {} Secret value cannot be empty.", "✗".red());
                        std::process::exit(1);
                    }

                    if let Some(ref agent_name) = agent_name {
                        // Agent-scoped secret
                        let agents = deploy_client.list_agents().await?;
                        let agent = agents.iter().find(|a| a.name == *agent_name);
                        match agent {
                            Some(a) => {
                                match deploy_client.set_secret(&a.id, &key, &secret_value).await {
                                    Ok(s) => {
                                        println!(
                                            "  {} {} set (agent: {}, hint: ••••{})",
                                            "✓".green().bold(),
                                            key.bright_white(),
                                            agent_name,
                                            s.hint.as_deref().unwrap_or("****")
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!("  {} Failed to set secret: {}", "✗".red(), e);
                                        std::process::exit(1);
                                    }
                                }
                            }
                            None => {
                                eprintln!("  {} Agent '{}' not found on remote.", "✗".red(), agent_name);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        // User-global secret
                        match deploy_client.set_user_secret(&key, &secret_value).await {
                            Ok(s) => {
                                println!(
                                    "  {} {} set (user-global, hint: ••••{})",
                                    "✓".green().bold(),
                                    key.bright_white(),
                                    s.hint.as_deref().unwrap_or("****")
                                );
                            }
                            Err(e) => {
                                eprintln!("  {} Failed to set secret: {}", "✗".red(), e);
                                std::process::exit(1);
                            }
                        }
                    }
                }

                SecretCommand::List { agent: agent_name } => {
                    let format_secrets = |secrets: &[SecretResponse]| {
                        if output_format == OutputFormat::Json {
                            let json: Vec<serde_json::Value> = secrets.iter().map(|s| {
                                serde_json::json!({
                                    "key": s.key,
                                    "hint": s.hint.as_deref().map(|h| format!("••••{}", h)),
                                    "scope": if s.agent_id.is_some() { "agent" } else { "global" },
                                    "updated_at": &s.created_at,
                                })
                            }).collect();
                            println!("{}", serde_json::to_string_pretty(&json).unwrap());
                        } else {
                            println!(
                                "  {:<28} {:<10} {:<12} {}",
                                "KEY".dimmed(),
                                "SCOPE".dimmed(),
                                "HINT".dimmed(),
                                "UPDATED".dimmed()
                            );
                            for s in secrets {
                                let scope = if s.agent_id.is_some() { "agent" } else { "global" };
                                let hint = s.hint.as_deref().map(|h| format!("••••{}", h)).unwrap_or_else(|| "••••••••".into());
                                println!(
                                    "  {:<28} {:<10} {:<12} {}",
                                    s.key,
                                    scope,
                                    hint,
                                    &s.created_at[..10]
                                );
                            }
                        }
                    };

                    if let Some(ref agent_name) = agent_name {
                        let agents = deploy_client.list_agents().await?;
                        let agent = agents.iter().find(|a| a.name == *agent_name);
                        match agent {
                            Some(a) => {
                                let secrets = deploy_client.list_agent_secrets(&a.id).await?;
                                if secrets.is_empty() && output_format != OutputFormat::Json {
                                    println!("  {} No secrets for agent '{}'.", "ℹ".bright_cyan(), agent_name);
                                } else {
                                    format_secrets(&secrets);
                                }
                            }
                            None => {
                                eprintln!("  {} Agent '{}' not found.", "✗".red(), agent_name);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        let secrets = deploy_client.list_user_secrets().await?;
                        if secrets.is_empty() && output_format != OutputFormat::Json {
                            println!("  {} No user-global secrets found.", "ℹ".bright_cyan());
                        } else {
                            format_secrets(&secrets);
                        }
                    }
                }

                SecretCommand::Delete { key, agent: agent_name, yes } => {
                    if !confirm(
                        &format!("Delete secret {}? Instances using this secret will fail on next restart.", key),
                        yes,
                    ) {
                        eprintln!("  {} Aborted.", "✗".red());
                        std::process::exit(0);
                    }

                    if let Some(ref agent_name) = agent_name {
                        let agents = deploy_client.list_agents().await?;
                        let agent = agents.iter().find(|a| a.name == *agent_name);
                        match agent {
                            Some(a) => {
                                let secrets = deploy_client.list_agent_secrets(&a.id).await?;
                                let secret = secrets.iter().find(|s| s.key == key);
                                match secret {
                                    Some(s) => {
                                        deploy_client.delete_agent_secret(&a.id, &s.id).await?;
                                        println!("  {} Deleted {} (agent: {})", "✓".green().bold(), key.bright_white(), agent_name);
                                    }
                                    None => {
                                        eprintln!("  {} Secret '{}' not found for agent '{}'.", "✗".red(), key, agent_name);
                                        std::process::exit(1);
                                    }
                                }
                            }
                            None => {
                                eprintln!("  {} Agent '{}' not found.", "✗".red(), agent_name);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        let secrets = deploy_client.list_user_secrets().await?;
                        let secret = secrets.iter().find(|s| s.key == key);
                        match secret {
                            Some(s) => {
                                deploy_client.delete_user_secret(&s.id).await?;
                                println!("  {} Deleted {} (user-global)", "✓".green().bold(), key.bright_white());
                            }
                            None => {
                                eprintln!("  {} Secret '{}' not found.", "✗".red(), key);
                                std::process::exit(1);
                            }
                        }
                    }
                }

                SecretCommand::Import { path } => {
                    let env_path = path
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| std::path::PathBuf::from(".env"));

                    if !env_path.exists() {
                        eprintln!("  {} File not found: {}", "✗".red(), env_path.display());
                        std::process::exit(1);
                    }

                    let env_vars = parse_env_file(&env_path)?;
                    if env_vars.is_empty() {
                        println!("  {} No variables found in {}", "ℹ".bright_cyan(), env_path.display());
                        return Ok(());
                    }

                    println!("  Found {} entries in {}:", env_vars.len(), env_path.display());
                    let mut count = 0;
                    for (key, value) in &env_vars {
                        eprint!("  ? Push {} as user-global secret? [Y/n] ", key.bright_white());
                        let mut buf = String::new();
                        std::io::stdin().read_line(&mut buf)?;
                        let answer = buf.trim().to_lowercase();
                        if answer.is_empty() || answer == "y" || answer == "yes" {
                            match deploy_client.set_user_secret(key, value).await {
                                Ok(s) => {
                                    println!(
                                        "    {} {} pushed (hint: ••••{})",
                                        "✓".green(),
                                        key,
                                        s.hint.as_deref().unwrap_or("****")
                                    );
                                    count += 1;
                                }
                                Err(e) => {
                                    eprintln!("    {} Failed to push {}: {}", "✗".red(), key, e);
                                }
                            }
                        } else {
                            println!("    {} Skipped {}", "–".dimmed(), key);
                        }
                    }
                    println!("  {} {} secret(s) imported.", "✓".green().bold(), count);
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

            // Generate deploy.toml and validate BEFORE building
            generate_deploy_manifest(&agent_path, skills_path.as_deref())?;

            let deploy_toml = agent_path.join("deploy.toml");
            let env_ref = env_path.as_deref();
            match starpod_vault::env::validate_env(&deploy_toml, env_ref) {
                Ok(warnings) => {
                    for w in &warnings {
                        println!("  {} {}", "⚠".yellow(), w);
                    }
                }
                Err(e) => {
                    eprintln!("  {} {}", "✗".red().bold(), e);
                    std::process::exit(1);
                }
            }

            starpod_core::build_standalone(
                &agent_path,
                &output_dir,
                skills_path.as_deref(),
                env_path.as_deref(),
                force,
            )?;

            let starpod_dir = output_dir.join(".starpod");

            let db_dir = starpod_dir.join("db");

            // Seal secrets from .env into vault.db so `starpod serve` can
            // inject them without needing the .env file on disk.
            let deploy_toml_built = starpod_dir.join("config").join("deploy.toml");
            if deploy_toml_built.exists() {
                let master_key = starpod_vault::derive_master_key(&db_dir)?;
                let vault = starpod_vault::Vault::new(&db_dir.join("vault.db"), &master_key).await?;
                let result = starpod_vault::env::populate_vault(
                    &deploy_toml_built,
                    env_path.as_deref(),
                    &vault,
                ).await?;
                if result.secrets_count > 0 || result.variables_count > 0 {
                    println!(
                        "  {} Sealed {} secret(s) and {} variable(s) into vault",
                        "✓".green().bold(),
                        result.secrets_count,
                        result.variables_count,
                    );
                }
                for w in &result.warnings {
                    println!("  {} {}", "⚠".yellow(), w);
                }
            }

            // Pre-seed the admin user with STARPOD_API_KEY from .env so that
            // `starpod serve` finds the admin already bootstrapped and uses
            // the known key instead of generating a random one.
            if let Some(ref env_p) = env_path {
                if env_p.exists() {
                    let env_map = parse_env_file(env_p)?;
                    if let Some(api_key) = env_map.get("STARPOD_API_KEY") {
                        let core_db = starpod_db::CoreDb::new(&db_dir).await?;
                        let auth = starpod_auth::AuthStore::from_pool(core_db.pool().clone());
                        if let Some((admin, _)) = auth.bootstrap_admin(Some(api_key)).await? {
                            println!(
                                "  {} Admin user pre-seeded (id: {})",
                                "✓".green().bold(),
                                &admin.id[..8],
                            );
                        }
                    }
                }
            }

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
            // Resolve backend URL and API key: env vars > saved credentials > default
            let saved = auth::load_credentials();
            let backend_url = std::env::var(auth::SPAWNER_URL_ENV)
                .ok()
                .or_else(|| saved.as_ref().map(|c| c.backend_url.clone()))
                .unwrap_or_else(|| auth::DEFAULT_SPAWNER_URL.to_string());

            let api_key = std::env::var("STARPOD_API_KEY")
                .ok()
                .or_else(|| saved.as_ref().map(|c| c.api_key.clone()));
            let Some(api_key) = api_key else {
                eprintln!(
                    "  {} Authentication required.",
                    "✗".red().bold()
                );
                eprintln!(
                    "  {} Run {} to authenticate.",
                    "→".dimmed(),
                    "starpod auth login".bright_white()
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
                HashMap::new()
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

            // Generate deploy.toml before deploying
            generate_deploy_manifest(&agent_dir, skills_path)?;

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
                    instance_name: None,
                    instance_description: None,
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
            triggered_by: None,
            model: None,
        };
        let (mut stream, session_id, _followup_tx) = agent.chat_stream(&chat_msg).await?;
        let (result_text, result_msg) = process_stream(&mut stream, &start).await?;

        if let Some(ref result) = result_msg {
            agent.finalize_chat(&session_id, line, &result_text, result, None).await;
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
        // Just fences with no content between them
        assert_eq!(strip_json_fence("```json\n```"), "");
    }

    #[test]
    fn strip_json_fence_bare_fences_only() {
        assert_eq!(strip_json_fence("```\n```"), "");
    }

    #[test]
    fn strip_json_fence_preserves_inner_backticks() {
        // JSON containing backtick strings shouldn't be corrupted
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
        // Single backticks are not a fence
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
