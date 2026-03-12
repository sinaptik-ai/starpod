//! Example: session management - resume and fork.
//!
//! ```bash
//! cargo run --example sessions
//! ```

use agent_sdk::{query, Message, Options};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // First query: analyze code and capture session ID
    let mut session_id = None;

    let mut stream = query(
        "Analyze the auth module and suggest improvements",
        Options::builder()
            .allowed_tools(vec![
                "Read".into(),
                "Glob".into(),
                "Grep".into(),
            ])
            .build(),
    );

    while let Some(message) = stream.next().await {
        let message = message?;
        if let Message::Result(result) = &message {
            session_id = Some(result.session_id.clone());
            if let Some(ref text) = result.result {
                println!("[first query] {}", text);
            }
        }
    }

    // Second query: resume with full context from the first
    if let Some(sid) = session_id {
        println!("\n[resuming session: {}]\n", sid);

        let mut stream = query(
            "Now implement the refactoring you suggested",
            Options::builder()
                .resume(&sid)
                .allowed_tools(vec![
                    "Read".into(),
                    "Edit".into(),
                    "Write".into(),
                    "Glob".into(),
                    "Grep".into(),
                ])
                .build(),
        );

        while let Some(message) = stream.next().await {
            let message = message?;
            if let Message::Result(result) = &message {
                if let Some(ref text) = result.result {
                    println!("[resumed] {}", text);
                }
            }
        }
    }

    Ok(())
}
