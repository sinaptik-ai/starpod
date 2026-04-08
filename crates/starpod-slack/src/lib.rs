//! Slack Socket Mode bot interface for Starpod.
//!
//! This crate is the Milestone 1 foundation of Starpod's Slack integration.
//! It speaks Slack's [Socket Mode] protocol end-to-end:
//!
//! 1. Calls `apps.connections.open` with an `xapp-...` app-level token to
//!    obtain a single-use `wss://` URL.
//! 2. Opens a WebSocket to that URL.
//! 3. Receives envelopes (`hello`, `events_api`, `slash_commands`,
//!    `interactive`, `disconnect`), acks the ones that need it, and hands
//!    them to a user-supplied [`EnvelopeHandler`].
//! 4. Reconnects on `disconnect` envelopes (immediately, no backoff) and on
//!    transport failures (exponential backoff capped at 30s).
//!
//! [Socket Mode]: https://docs.slack.dev/apis/events-api/using-socket-mode/
//!
//! ## Why Socket Mode?
//!
//! Spawner deploys per-user Starpod instances on ephemeral GCP VMs that do
//! not have stable inbound URLs. Socket Mode lets each instance open an
//! outbound WebSocket to Slack and receive `app_mention` events without any
//! Spawner-side webhook ingress, signature verification, or tunnel
//! infrastructure. The same code path works in `starpod dev` on a laptop
//! and in production on a VM with no configuration changes.
//!
//! ## Milestone 1 scope
//!
//! Milestone 1 ships only the transport: connect, receive, log, reconnect.
//! The default [`LoggingHandler`] emits a structured log line per envelope
//! and nothing else. Milestone 2 will add a real handler that dispatches
//! `events_api` envelopes to an agent turn and posts the reply via
//! `chat.postMessage`.
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use starpod_slack::{run_with_handler, LoggingHandler};
//!
//! # async fn doc() -> Result<(), Box<dyn std::error::Error>> {
//! let app_token = std::env::var("SLACK_APP_TOKEN")?;
//! let handler = Arc::new(LoggingHandler);
//! run_with_handler(&app_token, handler).await?;
//! # Ok(())
//! # }
//! ```
//!
//! `run_with_handler` only returns on a non-retriable error (e.g. an
//! invalid token). Transient failures are retried internally.

mod client;
mod connections;
mod dedup;
mod envelope;
mod error;
mod format;
mod handler;
mod socket;

use std::sync::Arc;
use std::time::Duration;

use starpod_agent::StarpodAgent;
use starpod_auth::AuthStore;
use tracing::{error, info, warn};

pub use client::{AuthTestResponse, ChatPostMessageArgs, SlackWebClient};
pub use connections::{SlackApi, SLACK_API_BASE};
pub use dedup::DedupStore;
pub use envelope::{
    Ack, ConnectionInfo, Disconnect, Envelope, EventsApi, Hello, Interactive, SlashCommands,
};
pub use error::{Result, SlackError};
pub use format::{markdown_to_mrkdwn, split_for_slack, MAX_SLACK_MESSAGE_LEN};
pub use handler::{handle_event, AgentHandler, HandlerState};
pub use socket::{run_once, DisconnectReason, EnvelopeHandler, LoggingHandler};

/// Backoff schedule applied to retriable connection failures.
///
/// Per the design doc: 1s → 2s → 4s → 8s → 16s → 30s (cap). The schedule
/// resets to the first entry after any successful connection (defined as
/// "received at least one envelope or returned `SlackRequested`").
///
/// Slack-initiated `disconnect` envelopes do NOT consume a backoff slot —
/// they reconnect immediately because Slack proactively recycles WebSocket
/// containers and a backoff there would just drop events.
const BACKOFF_SCHEDULE: &[Duration] = &[
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
];

/// Run the Slack bot with a pre-built agent and auth store — the entry
/// point the starpod runtime calls.
///
/// This is the Slack peer of
/// [`starpod_telegram::run_with_agent_and_auth`]. It wires:
///
/// 1. `SlackWebClient` (bound to `SLACK_BOT_TOKEN`) for outbound calls.
/// 2. `auth.test` at startup to fetch `bot_user_id` + `team_id`.
///    If this fails, the function returns a non-retriable error so the
///    gateway surfaces a clear message to the operator.
/// 3. `DedupStore` backed by the agent's `core.db` pool + a background
///    sweeper task.
/// 4. [`AgentHandler`] as the receive-loop handler — it spawns one task
///    per `events_api` envelope so slow LLM turns don't block acks.
/// 5. [`run_with_handler`] as the persistent reconnect loop.
///
/// Returns only on a non-retriable error (invalid tokens, auth.test
/// failure). Transient transport failures are retried internally.
pub async fn run_with_agent_and_auth(
    agent: Arc<StarpodAgent>,
    auth: Arc<AuthStore>,
    app_token: String,
    bot_token: String,
) -> starpod_core::Result<()> {
    // 1. Build the Web API client and validate the bot token by calling
    //    auth.test. This fails fast on invalid/revoked tokens.
    let web = SlackWebClient::new(bot_token);
    let auth_info = web.auth_test().await.map_err(|e| {
        starpod_core::StarpodError::Channel(format!(
            "slack auth.test failed — check SLACK_BOT_TOKEN: {e}"
        ))
    })?;
    info!(
        team = %auth_info.team,
        team_id = %auth_info.team_id,
        bot_user_id = %auth_info.user_id,
        "slack bot authenticated"
    );

    // 2. Open the dedup store on the agent's core.db pool.
    let dedup = DedupStore::new(agent.core_db().pool().clone());
    let sweeper_handle = dedup.clone().spawn_sweeper();

    // 3. Build handler state, mirroring telegram's config lookup.
    let stream_mode = agent
        .config()
        .channels
        .slack
        .as_ref()
        .map(|s| s.stream_mode.clone())
        .unwrap_or_else(|| "final_only".to_string());

    let state = HandlerState {
        agent: agent.clone(),
        auth: auth.clone(),
        web,
        dedup,
        bot_user_id: auth_info.user_id,
        team_id: auth_info.team_id,
        stream_mode,
    };

    let handler = Arc::new(AgentHandler { state });

    // 4. Run the reconnect loop until a non-retriable error.
    let result = run_with_handler(&app_token, handler).await;

    // Shut down the dedup sweeper before returning.
    sweeper_handle.abort();

    result.map_err(Into::into)
}

/// Run the Slack Socket Mode bot until a non-retriable error occurs.
///
/// This is the public entry point most callers want. It loops forever:
///
/// 1. Call `apps.connections.open` to get a fresh `wss://` URL.
/// 2. Connect and run the receive loop via [`run_once`].
/// 3. On `DisconnectReason::SlackRequested`, reconnect immediately.
/// 4. On `DisconnectReason::PeerClosed` or any retriable error, sleep
///    according to [`BACKOFF_SCHEDULE`] and reconnect.
/// 5. On a non-retriable error (per [`SlackError::is_retriable`]), return
///    the error to the caller.
///
/// The `app_token` must be a Slack app-level token (`xapp-...`) with the
/// `connections:write` scope. Bot tokens (`xoxb-...`) are rejected
/// client-side without making a network call.
///
/// The `handler` is invoked exactly once per envelope, after the envelope
/// has been acked to Slack. See [`EnvelopeHandler`] for the contract.
pub async fn run_with_handler<H>(app_token: &str, handler: Arc<H>) -> Result<()>
where
    H: EnvelopeHandler,
{
    run_with_handler_using(SlackApi::new(), app_token, handler).await
}

/// Same as [`run_with_handler`] but with a caller-supplied [`SlackApi`]
/// client.
///
/// Used by the integration test to point both the HTTP and WebSocket
/// endpoints at a local mock server. Production callers should prefer
/// [`run_with_handler`].
pub async fn run_with_handler_using<H>(
    api: SlackApi,
    app_token: &str,
    handler: Arc<H>,
) -> Result<()>
where
    H: EnvelopeHandler,
{
    let mut backoff_index: usize = 0;

    loop {
        // Step 1: obtain a fresh wss:// URL. Single-use, ~30s validity, so
        // we always re-call this on every reconnect rather than caching.
        let ws_url = match api.open_connection(app_token).await {
            Ok(url) => url,
            Err(e) => {
                if !e.is_retriable() {
                    error!(error = %e, "slack connect failed with non-retriable error");
                    return Err(e);
                }
                let delay = backoff_delay(backoff_index);
                warn!(
                    error = %e,
                    backoff_secs = delay.as_secs(),
                    "slack apps.connections.open failed, retrying"
                );
                tokio::time::sleep(delay).await;
                backoff_index = (backoff_index + 1).min(BACKOFF_SCHEDULE.len() - 1);
                continue;
            }
        };

        // Step 2: run one connection lifetime.
        match run_once(&ws_url, handler.clone()).await {
            Ok(DisconnectReason::SlackRequested) => {
                // Slack told us to reconnect — this is the happy path for
                // container refreshes. No backoff, reset the schedule.
                info!("slack reconnect requested by server, opening new connection");
                backoff_index = 0;
            }
            Ok(DisconnectReason::PeerClosed) => {
                // Clean close without a `disconnect` envelope. Treat as a
                // transient blip and back off — usually means a network
                // path issue rather than a Slack-side recycle.
                let delay = backoff_delay(backoff_index);
                warn!(
                    backoff_secs = delay.as_secs(),
                    "slack websocket closed without disconnect envelope, reconnecting"
                );
                tokio::time::sleep(delay).await;
                backoff_index = (backoff_index + 1).min(BACKOFF_SCHEDULE.len() - 1);
            }
            Err(e) => {
                if !e.is_retriable() {
                    error!(error = %e, "slack receive loop failed with non-retriable error");
                    return Err(e);
                }
                let delay = backoff_delay(backoff_index);
                warn!(
                    error = %e,
                    backoff_secs = delay.as_secs(),
                    "slack receive loop failed, reconnecting"
                );
                tokio::time::sleep(delay).await;
                backoff_index = (backoff_index + 1).min(BACKOFF_SCHEDULE.len() - 1);
            }
        }
    }
}

/// Look up the backoff duration for the current attempt index, clamped to
/// the last entry in [`BACKOFF_SCHEDULE`].
fn backoff_delay(index: usize) -> Duration {
    BACKOFF_SCHEDULE
        .get(index)
        .copied()
        .unwrap_or_else(|| *BACKOFF_SCHEDULE.last().expect("schedule is non-empty"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule_is_monotonic_and_capped() {
        let mut prev = Duration::ZERO;
        for d in BACKOFF_SCHEDULE {
            assert!(*d >= prev, "schedule must be non-decreasing");
            prev = *d;
        }
        assert_eq!(
            *BACKOFF_SCHEDULE.last().unwrap(),
            Duration::from_secs(30),
            "cap must be 30s per the design doc"
        );
    }

    #[test]
    fn backoff_delay_clamps_to_last_entry() {
        let last = *BACKOFF_SCHEDULE.last().unwrap();
        assert_eq!(backoff_delay(0), BACKOFF_SCHEDULE[0]);
        assert_eq!(backoff_delay(BACKOFF_SCHEDULE.len() - 1), last);
        assert_eq!(backoff_delay(999), last);
    }

    #[test]
    fn backoff_starts_at_one_second() {
        assert_eq!(BACKOFF_SCHEDULE[0], Duration::from_secs(1));
    }
}
