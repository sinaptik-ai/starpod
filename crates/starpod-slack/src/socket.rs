//! Slack Socket Mode WebSocket receive loop.
//!
//! This module is pure transport: it takes a `wss://` URL (obtained
//! separately from `apps.connections.open`) and runs one connection
//! lifetime — connect, receive envelopes, ack the ones that need acking,
//! return when Slack sends `disconnect` or the connection drops.
//!
//! ## Why this is split out
//!
//! Keeping the receive loop separate from the HTTP connect step makes the
//! integration test trivial: the test spins up a local WebSocket server,
//! constructs a `ws://127.0.0.1:port/...` URL, and calls
//! [`run_once`] directly. No mock HTTP layer needed.
//!
//! ## Acknowledgement contract
//!
//! Slack requires every `events_api`, `slash_commands`, and `interactive`
//! envelope to be acknowledged within ~3 seconds by sending
//! `{"envelope_id": "..."}` back over the same WebSocket. We do this
//! synchronously inside the receive loop, BEFORE handing the envelope to
//! the user-supplied handler, so a slow or panicking handler cannot cause
//! us to miss the ack window. The handler runs after the ack, so if it
//! fails the only consequence is that the agent didn't respond — Slack
//! will not retry the same event again.
//!
//! For Milestone 1 the handler is purely informational (logging only) and
//! the ack-then-handle ordering is overkill, but it's the right ordering
//! for Milestone 2 when the handler will actually run an LLM turn that can
//! take 10+ seconds.

use std::sync::Arc;

use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::envelope::{Ack, Envelope};
use crate::error::{Result, SlackError};

/// Outcome of a single connection lifetime.
///
/// Returned by [`run_once`] so the outer reconnect loop can distinguish a
/// graceful Slack-initiated reconnect from a transport error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Slack sent a `disconnect` envelope. Reconnect immediately, no backoff.
    SlackRequested,
    /// The peer closed the WebSocket cleanly without a `disconnect` envelope.
    /// Treated as a transient event — outer loop reconnects with backoff.
    PeerClosed,
}

/// Trait for the user-supplied envelope handler.
///
/// Milestone 1 ships [`LoggingHandler`] which just emits a structured log
/// for every envelope. Milestone 2 will introduce a real handler that
/// dispatches `events_api` envelopes to an agent turn.
///
/// The handler is invoked **after** the envelope has been acked to Slack,
/// so a slow or failing handler cannot cause Slack to retry. The handler
/// runs on the same task as the receive loop in Milestone 1; Milestone 2
/// will move it onto a spawned task per envelope.
pub trait EnvelopeHandler: Send + Sync + 'static {
    /// Process one envelope. Errors are logged but not propagated — the
    /// receive loop continues so a single bad event cannot kill the bot.
    fn handle(&self, envelope: Envelope);
}

/// Default handler used by [`run_once`] when no custom handler is supplied.
///
/// Logs the variant name plus, for `events_api` envelopes, the `event_id`
/// and the inner event type — enough to verify end-to-end delivery without
/// surfacing message text in logs.
#[derive(Debug, Default, Clone, Copy)]
pub struct LoggingHandler;

impl EnvelopeHandler for LoggingHandler {
    fn handle(&self, envelope: Envelope) {
        match envelope {
            Envelope::Hello(hello) => {
                if hello.num_connections > 1 {
                    warn!(
                        num_connections = hello.num_connections,
                        "slack hello: multiple active connections share this app token \
                         — events will be distributed unpredictably across processes"
                    );
                } else {
                    info!(
                        num_connections = hello.num_connections,
                        app_id = hello.connection_info.as_ref().map(|c| c.app_id.as_str()),
                        "slack hello"
                    );
                }
            }
            Envelope::EventsApi(e) => {
                let event_id = e
                    .payload
                    .get("event_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let event_type = e
                    .payload
                    .get("event")
                    .and_then(|v| v.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let team_id = e
                    .payload
                    .get("team_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                info!(
                    event_id,
                    event_type,
                    team_id,
                    retry_attempt = e.retry_attempt,
                    "slack events_api received"
                );
            }
            Envelope::SlashCommands(s) => {
                debug!(envelope_id = %s.envelope_id, "slack slash_commands received (not handled in phase 1)");
            }
            Envelope::Interactive(i) => {
                debug!(envelope_id = %i.envelope_id, "slack interactive received (not handled in phase 1)");
            }
            Envelope::Disconnect(d) => {
                info!(reason = ?d.reason, "slack disconnect");
            }
            Envelope::Unknown => {
                debug!("slack unknown envelope type");
            }
        }
    }
}

/// Run one connection lifetime: connect, receive, return.
///
/// This function does not implement reconnect logic — that lives in
/// [`crate::run_with_handler`]. It returns:
///
/// - `Ok(DisconnectReason::SlackRequested)` when Slack sends a graceful
///   `disconnect` envelope. The outer loop should reconnect immediately.
/// - `Ok(DisconnectReason::PeerClosed)` when the WebSocket closes cleanly
///   without a prior `disconnect`. The outer loop should reconnect with
///   backoff.
/// - `Err(_)` for transport-level failures (TLS errors, malformed frames,
///   ping timeouts). The outer loop should reconnect with backoff and may
///   surface the error via metrics.
///
/// The `handler` is invoked exactly once per envelope, after that envelope
/// has been acked to Slack (where applicable).
pub async fn run_once<H>(ws_url: &str, handler: Arc<H>) -> Result<DisconnectReason>
where
    H: EnvelopeHandler,
{
    debug!(ws_url = %scrub_ticket(ws_url), "slack socket connecting");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(ws_url).await?;
    debug!("slack socket connected");

    while let Some(frame) = ws.next().await {
        let msg = match frame {
            Ok(m) => m,
            Err(e) => {
                error!(error = %e, "slack socket frame error");
                return Err(SlackError::WebSocket(e));
            }
        };

        match msg {
            Message::Text(text) => {
                // Parse the envelope. Forward-compat: unknown envelope
                // types deserialize to Envelope::Unknown rather than
                // erroring out.
                let envelope: Envelope = match serde_json::from_str(&text) {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(
                            error = %e,
                            text_len = text.len(),
                            "slack envelope failed to parse, dropping frame"
                        );
                        continue;
                    }
                };

                debug!(
                    envelope = envelope.variant_name(),
                    "slack envelope received"
                );

                // Ack ackable envelopes BEFORE calling the handler so a
                // slow handler can never miss the ~3s ack window.
                if let Some(envelope_id) = envelope.envelope_id() {
                    let ack_json = match Ack::new(envelope_id).to_json() {
                        Ok(s) => s,
                        Err(e) => {
                            // Effectively impossible — the ack struct is
                            // string-only. Log and continue.
                            error!(error = %e, "failed to serialize slack ack");
                            continue;
                        }
                    };
                    if let Err(e) = ws.send(Message::Text(ack_json.into())).await {
                        error!(error = %e, envelope_id, "failed to send slack ack");
                        return Err(SlackError::WebSocket(e));
                    }
                }

                // `disconnect` is the one variant that controls the loop
                // lifetime — return so the outer reconnect loop can open a
                // fresh connection.
                if matches!(envelope, Envelope::Disconnect(_)) {
                    handler.handle(envelope);
                    return Ok(DisconnectReason::SlackRequested);
                }

                handler.handle(envelope);
            }

            Message::Binary(bytes) => {
                // Slack does not send binary frames. Log at debug and skip.
                debug!(len = bytes.len(), "slack binary frame ignored");
            }

            // tungstenite handles ping/pong automatically when the auto_pong
            // config is enabled (the default in 0.28). We still match these
            // arms explicitly so the compiler tells us if a future version
            // adds new Message variants we need to handle.
            Message::Ping(_) | Message::Pong(_) => {
                debug!("slack ws keepalive frame");
            }

            Message::Close(frame) => {
                debug!(?frame, "slack ws close frame");
                return Ok(DisconnectReason::PeerClosed);
            }

            Message::Frame(_) => {
                // Raw frame, only seen in low-level tungstenite usage.
                debug!("slack raw frame ignored");
            }
        }
    }

    // Stream ended without a Close frame.
    Ok(DisconnectReason::PeerClosed)
}

/// Replace any `?ticket=...` query parameter in the WebSocket URL with a
/// placeholder before logging.
///
/// Slack's Socket Mode connect URLs include a single-use ticket in the
/// query string that grants access to the connection. We never want it in
/// logs even though it's short-lived.
fn scrub_ticket(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            let pairs: Vec<(String, String)> = parsed
                .query_pairs()
                .map(|(k, v)| {
                    let key = k.into_owned();
                    let value = if key == "ticket" {
                        "<scrubbed>".to_string()
                    } else {
                        v.into_owned()
                    };
                    (key, value)
                })
                .collect();
            parsed.query_pairs_mut().clear();
            for (k, v) in pairs {
                parsed.query_pairs_mut().append_pair(&k, &v);
            }
            parsed.to_string()
        }
        Err(_) => "<unparseable-url>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_ticket_replaces_value() {
        let scrubbed = scrub_ticket("wss://wss-primary.slack.com/link/?ticket=SECRET&app_id=A1");
        assert!(!scrubbed.contains("SECRET"));
        assert!(
            scrubbed.contains("ticket=%3Cscrubbed%3E") || scrubbed.contains("ticket=<scrubbed>")
        );
        assert!(scrubbed.contains("app_id=A1"));
    }

    #[test]
    fn scrub_ticket_handles_no_query() {
        let scrubbed = scrub_ticket("wss://wss-primary.slack.com/link/");
        assert!(scrubbed.starts_with("wss://wss-primary.slack.com/link/"));
    }

    #[test]
    fn scrub_ticket_handles_invalid_url() {
        let scrubbed = scrub_ticket("not a url");
        assert_eq!(scrubbed, "<unparseable-url>");
    }

    #[test]
    fn logging_handler_does_not_panic_on_any_variant() {
        // Smoke test: hand every envelope variant to the logging handler
        // and confirm none of them panic. Output is captured by the test
        // runner.
        let h = LoggingHandler;
        h.handle(Envelope::Hello(crate::envelope::Hello {
            num_connections: 1,
            connection_info: None,
            debug_info: None,
        }));
        h.handle(Envelope::Hello(crate::envelope::Hello {
            num_connections: 3, // triggers the warn! branch
            connection_info: None,
            debug_info: None,
        }));
        h.handle(Envelope::EventsApi(crate::envelope::EventsApi {
            envelope_id: "e1".into(),
            payload: serde_json::json!({
                "team_id": "T1",
                "event_id": "Ev1",
                "event": { "type": "app_mention" }
            }),
            accepts_response_payload: false,
            retry_attempt: 0,
            retry_reason: None,
        }));
        h.handle(Envelope::SlashCommands(crate::envelope::SlashCommands {
            envelope_id: "s1".into(),
            payload: serde_json::Value::Null,
            accepts_response_payload: false,
        }));
        h.handle(Envelope::Interactive(crate::envelope::Interactive {
            envelope_id: "i1".into(),
            payload: serde_json::Value::Null,
            accepts_response_payload: false,
        }));
        h.handle(Envelope::Disconnect(crate::envelope::Disconnect {
            reason: Some("refresh_requested".into()),
            debug_info: None,
        }));
        h.handle(Envelope::Unknown);
    }
}
