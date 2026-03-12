//! Example: connecting to MCP servers for external tool access.
//!
//! ```bash
//! GITHUB_TOKEN=your_token cargo run --example with_mcp
//! ```

use agent_sdk::{query, McpServerConfig, Message, Options};
use agent_sdk::mcp::McpStdioServerConfig;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let github_token = std::env::var("GITHUB_TOKEN")
        .expect("GITHUB_TOKEN environment variable required");

    let mut stream = query(
        "List the 3 most recent issues in anthropics/claude-code",
        Options::builder()
            .mcp_server(
                "github",
                McpServerConfig::Stdio(
                    McpStdioServerConfig::new("npx")
                        .with_args(vec![
                            "-y".into(),
                            "@modelcontextprotocol/server-github".into(),
                        ])
                        .with_env("GITHUB_TOKEN", &github_token),
                ),
            )
            .allowed_tools(vec!["mcp__github__list_issues".into()])
            .build(),
    );

    while let Some(message) = stream.next().await {
        let message = message?;

        match &message {
            Message::System(sys) => {
                if let Some(ref servers) = sys.mcp_servers {
                    for server in servers {
                        println!("[mcp] {} - {}", server.name, server.status);
                    }
                }
            }
            Message::Result(result) => {
                if let Some(ref text) = result.result {
                    println!("{}", text);
                }
            }
            _ => {}
        }
    }

    Ok(())
}
