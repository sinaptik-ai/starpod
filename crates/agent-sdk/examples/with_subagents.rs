//! Example: using subagents for specialized tasks.
//!
//! ```bash
//! cargo run --example with_subagents
//! ```

use agent_sdk::types::agent::AgentModel;
use agent_sdk::{query, AgentDefinition, ContentBlock, Message, Options};
use colored::Colorize;
use std::time::Instant;
use tokio_stream::StreamExt;

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max])
    } else {
        s.to_string()
    }
}

fn print_header() {
    println!();
    println!(
        "{}",
        "  ╭──────────────────────────────────────────╮"
            .bright_cyan()
    );
    println!(
        "{}",
        "  │       Agent SDK  ·  Subagent Demo        │"
            .bright_cyan()
    );
    println!(
        "{}",
        "  ╰──────────────────────────────────────────╯"
            .bright_cyan()
    );
    println!();
}

fn print_separator() {
    println!(
        "  {}",
        "─────────────────────────────────────────────".dimmed()
    );
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    print_header();

    let start = Instant::now();
    let mut turn = 0u32;

    let mut stream = query(
        "Review the authentication module for security issues",
        Options::builder()
            .allowed_tools(vec![
                "Read".into(),
                "Grep".into(),
                "Glob".into(),
                "Agent".into(),
            ])
            .agent(
                "code-reviewer",
                AgentDefinition::new(
                    "Expert code review specialist. Use for quality, security, and maintainability reviews.",
                    "You are a code review specialist with expertise in security, performance, and best practices.\n\n\
                     When reviewing code:\n\
                     - Identify security vulnerabilities\n\
                     - Check for performance issues\n\
                     - Verify adherence to coding standards\n\
                     - Suggest specific improvements\n\n\
                     Be thorough but concise in your feedback.",
                )
                .with_tools(vec![
                    "Read".into(),
                    "Grep".into(),
                    "Glob".into(),
                ])
                .with_model(AgentModel::Sonnet),
            )
            .agent(
                "test-runner",
                AgentDefinition::new(
                    "Runs and analyzes test suites. Use for test execution and coverage analysis.",
                    "You are a test execution specialist. Run tests and provide clear analysis of results.",
                )
                .with_tools(vec![
                    "Bash".into(),
                    "Read".into(),
                    "Grep".into(),
                ]),
            )
            .build(),
    );

    while let Some(message) = stream.next().await {
        let message = message?;

        match &message {
            Message::System(sys) => {
                println!(
                    "  {} {}",
                    "●".bright_green(),
                    format!("Session {}", &sys.session_id[..8]).dimmed()
                );
                if let Some(ref model) = sys.model {
                    println!(
                        "  {} Model: {}",
                        "│".dimmed(),
                        model.bright_white()
                    );
                }
                if let Some(ref tools) = sys.tools {
                    println!(
                        "  {} Tools: {}",
                        "│".dimmed(),
                        tools.join(", ").dimmed()
                    );
                }
                if let Some(ref agents) = sys.agents {
                    println!(
                        "  {} Agents: {}",
                        "│".dimmed(),
                        agents
                            .iter()
                            .map(|a| a.bright_yellow().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
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
                                println!();
                                for line in text.lines() {
                                    println!("  {}", line);
                                }
                            }
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            let icon = match name.as_str() {
                                "Read" => "📄",
                                "Grep" => "🔍",
                                "Glob" => "📂",
                                "Bash" => "⚡",
                                "Edit" => "✏️",
                                "Write" => "💾",
                                "Agent" => "🤖",
                                _ => "🔧",
                            };

                            let input_preview = if let Some(path) =
                                input.get("file_path").and_then(|v| v.as_str())
                            {
                                path.to_string()
                            } else if let Some(pattern) =
                                input.get("pattern").and_then(|v| v.as_str())
                            {
                                pattern.to_string()
                            } else if let Some(cmd) =
                                input.get("command").and_then(|v| v.as_str())
                            {
                                truncate(cmd, 60)
                            } else if let Some(prompt) =
                                input.get("prompt").and_then(|v| v.as_str())
                            {
                                truncate(prompt, 60)
                            } else {
                                let s = serde_json::to_string(input).unwrap_or_default();
                                truncate(&s, 80)
                            };

                            println!(
                                "  {} {} {}",
                                icon,
                                name.bright_blue().bold(),
                                input_preview.dimmed()
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
                println!();
                print_separator();

                if result.is_error {
                    println!(
                        "  {} {}",
                        "✗".red().bold(),
                        "Error".red().bold()
                    );
                    for err in &result.errors {
                        println!("    {}", err.red());
                    }
                } else {
                    println!(
                        "  {} {}",
                        "✓".green().bold(),
                        "Complete".green().bold()
                    );
                }

                if let Some(ref text) = result.result {
                    println!();
                    for line in text.lines() {
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
                    result.num_turns,
                    "💰".dimmed(),
                    result.total_cost_usd,
                    "📊".dimmed(),
                    result
                        .usage
                        .as_ref()
                        .map(|u| format!("{}k", u.input_tokens / 1000))
                        .unwrap_or_default()
                        .bright_white(),
                    result
                        .usage
                        .as_ref()
                        .map(|u| format!("{}k", u.output_tokens / 1000))
                        .unwrap_or_default()
                        .bright_white(),
                );
                print_separator();
                println!();
            }
            _ => {}
        }
    }

    Ok(())
}
