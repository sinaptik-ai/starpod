use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use orion_agent::OrionAgent;
use orion_core::{ChatMessage, OrionConfig};

#[derive(Parser)]
#[command(name = "orion", about = "Orion — personal AI assistant", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the gateway HTTP/WS server.
    Serve,

    /// Send a one-shot chat message.
    Chat {
        /// The message to send.
        message: String,
    },

    /// Start an interactive REPL session.
    Repl,

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
}

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let config = OrionConfig::load().await?;

    match cli.command {
        Commands::Serve => {
            orion_gateway::serve(config).await?;
        }

        Commands::Chat { message } => {
            let agent = OrionAgent::new(config)?;
            let response = agent
                .chat(ChatMessage {
                    text: message,
                    user_id: None,
                    channel_id: Some("cli".into()),
                    attachments: Vec::new(),
                })
                .await?;

            println!("{}", response.text);

            if let Some(usage) = &response.usage {
                eprintln!(
                    "\n[tokens: {}in/{}out, cost: ${:.4}]",
                    usage.input_tokens, usage.output_tokens, usage.cost_usd
                );
            }
        }

        Commands::Repl => {
            run_repl(config).await?;
        }

        Commands::Memory { action } => {
            let agent = OrionAgent::new(config)?;
            match action {
                MemoryAction::Search { query, limit } => {
                    let results = agent.memory().search(&query, limit)?;
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
                    agent.memory().reindex()?;
                    println!("Memory index rebuilt.");
                }
            }
        }

        Commands::Vault { action } => {
            let agent = OrionAgent::new(config)?;
            match action {
                VaultAction::Get { key } => match agent.vault().get(&key)? {
                    Some(value) => println!("{}", value),
                    None => println!("No value found for key: {}", key),
                },
                VaultAction::Set { key, value } => {
                    agent.vault().set(&key, &value)?;
                    println!("Stored '{}'.", key);
                }
                VaultAction::Delete { key } => {
                    agent.vault().delete(&key)?;
                    println!("Deleted '{}'.", key);
                }
                VaultAction::List => {
                    let keys = agent.vault().list_keys()?;
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
            let agent = OrionAgent::new(config)?;
            match action {
                SessionAction::List { limit } => {
                    let sessions = agent.session_mgr().list_sessions(limit)?;
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
    }

    Ok(())
}

/// Interactive REPL mode.
async fn run_repl(config: OrionConfig) -> anyhow::Result<()> {
    let agent = OrionAgent::new(config)?;

    println!("Orion REPL — type your message, or 'exit' to quit.\n");

    let mut rl = rustyline::DefaultEditor::new()?;

    loop {
        let line = match rl.readline("you> ") {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted | rustyline::error::ReadlineError::Eof) => {
                println!("Goodbye!");
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
            println!("Goodbye!");
            break;
        }

        rl.add_history_entry(line)?;

        let response = agent
            .chat(ChatMessage {
                text: line.to_string(),
                user_id: None,
                channel_id: Some("repl".into()),
                attachments: Vec::new(),
            })
            .await?;

        println!("\norion> {}\n", response.text);
    }

    Ok(())
}
