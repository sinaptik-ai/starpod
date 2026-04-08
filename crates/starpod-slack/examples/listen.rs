//! Smoke test for the Slack Socket Mode client.
//!
//! Reads `SLACK_APP_TOKEN` from the environment, opens a Socket Mode
//! connection, and logs every envelope Slack sends. This runs the exact
//! production reconnect loop — nothing is mocked.
//!
//! Usage:
//!
//! ```sh
//! export SLACK_APP_TOKEN=xapp-1-A0...-your-token
//! RUST_LOG=starpod_slack=debug,info \
//!   cargo run --example listen -p starpod-slack
//! ```
//!
//! Then in Slack:
//!
//! - DM the bot, or
//! - `@mention` the bot in a channel the bot is a member of, or
//! - run a `/slash` command registered for the app.
//!
//! You should see `events_api` / `slash_commands` / `interactive` envelopes
//! appear in your terminal within a second of each Slack interaction.

use std::sync::Arc;

use starpod_slack::{run_with_handler, LoggingHandler, SlackError};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default to verbose logging so the smoke test actually shows
    // something useful. Users can override with RUST_LOG.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "starpod_slack=debug,info".into()),
        )
        .init();

    let app_token = std::env::var("SLACK_APP_TOKEN").map_err(|_| {
        "SLACK_APP_TOKEN is not set. Create a Slack app with Socket Mode \
         enabled, generate an app-level token with the `connections:write` \
         scope, and export it as SLACK_APP_TOKEN before running this example."
    })?;

    let handler = Arc::new(LoggingHandler);

    tracing::info!("starting slack socket mode listener — Ctrl+C to stop");

    match run_with_handler(&app_token, handler).await {
        Ok(()) => unreachable!("run_with_handler only returns on a non-retriable error"),
        Err(SlackError::InvalidAppToken(reason)) => {
            eprintln!("\nERROR: invalid app token — {reason}");
            eprintln!("       app-level tokens start with `xapp-`, not `xoxb-`.");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("\nERROR: {e}");
            std::process::exit(1);
        }
    }
}
