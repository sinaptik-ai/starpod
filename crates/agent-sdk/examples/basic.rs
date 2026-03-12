//! Basic example: run a simple agent query.
//!
//! ```bash
//! cargo run --example basic
//! ```

use agent_sdk::{query, Message, Options};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt::init();

    // Create a query with basic read-only tools
    let mut stream = query(
        "What files are in this directory?",
        Options::builder()
            .allowed_tools(vec!["Bash".into(), "Glob".into()])
            .build(),
    );

    // Stream messages as they arrive
    while let Some(message) = stream.next().await {
        let message = message?;

        match &message {
            Message::System(sys) => {
                println!("[system] Session: {}", sys.session_id);
            }
            Message::Assistant(assistant) => {
                for block in &assistant.content {
                    if let agent_sdk::ContentBlock::Text { text } = block {
                        print!("{}", text);
                    }
                }
            }
            Message::Result(result) => {
                if let Some(ref text) = result.result {
                    println!("\n[result] {}", text);
                }
                println!(
                    "[cost] ${:.4} | [turns] {}",
                    result.total_cost_usd, result.num_turns
                );
            }
            _ => {}
        }
    }

    Ok(())
}
