//! Handler module — the logic that runs between the Slack WebSocket
//! receive loop and the Starpod agent runtime.
//!
//! The pipeline is:
//!
//! ```text
//!   events_api envelope (parsed)
//!       │
//!       ▼ [already acked by socket.rs]
//!   extract inner event
//!       │
//!       ▼  drop if not app_mention or message.im
//!   filter + self-loop guard (bot_user_id)
//!       │
//!       ▼  dedup by Slack event_id
//!   SQLite INSERT OR IGNORE
//!       │
//!       ▼  derive {team}:{channel}:{thread_ts} session key
//!   build ChatMessage
//!       │
//!       ▼  agent.chat_stream()
//!   collect assistant text
//!       │
//!       ▼  markdown → mrkdwn
//!   chat.postMessage (in thread)
//!       │
//!       ▼  record usage
//!   agent.finalize_chat
//! ```
//!
//! Every step is guarded: any failure is logged with enough structured
//! context (`team_id`, `channel`, `event_id`) to be grep-able, and the
//! handler never propagates a panic back to the receive loop.

use std::sync::Arc;

use agent_sdk::{ContentBlock, Message};
use base64::Engine;
use serde_json::Value;
use starpod_agent::StarpodAgent;
use starpod_auth::AuthStore;
use starpod_core::{Attachment, AttachmentsConfig, ChatMessage};
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

use crate::client::{ChatPostMessageArgs, SlackWebClient};
use crate::dedup::DedupStore;
use crate::envelope::{Envelope, EventsApi};
use crate::format::{markdown_to_mrkdwn, split_for_slack};
use crate::socket::EnvelopeHandler;

/// Shared state for the Slack handler. Cloned into every per-event
/// spawned task so a slow LLM turn does not block the receive loop.
#[derive(Clone)]
pub struct HandlerState {
    pub agent: Arc<StarpodAgent>,
    #[allow(dead_code)] // reserved for per-user auth in phase 2
    pub auth: Arc<AuthStore>,
    pub web: SlackWebClient,
    pub dedup: DedupStore,
    /// This bot's own Slack user ID, fetched from `auth.test` at startup.
    /// Used to drop self-authored messages (loop prevention).
    pub bot_user_id: String,
    /// Team ID the bot is installed into. Used as a safety net for
    /// messages that don't carry `team_id` in the outer payload.
    pub team_id: String,
    /// Stream mode from `channels.slack.stream_mode` in config.
    pub stream_mode: String,
}

/// Entry point called once per decoded `events_api` envelope.
///
/// Always returns `Ok(())` — failures are logged, never propagated,
/// because this runs in a detached task and there's nothing useful the
/// caller could do with an error.
pub async fn handle_event(state: HandlerState, envelope: EventsApi) {
    let event_id = extract_event_id(&envelope.payload)
        .unwrap_or("unknown")
        .to_string();
    let team_id = extract_team_id(&envelope.payload)
        .unwrap_or(state.team_id.as_str())
        .to_string();

    // ── 1. Dedup ───────────────────────────────────────────────────────
    match state.dedup.insert_if_new(&event_id).await {
        Ok(false) => {
            debug!(event_id = %event_id, "slack dedup hit, dropping duplicate");
            return;
        }
        Ok(true) => {}
        Err(e) => {
            // Don't drop the event on a dedup failure — better to
            // double-process than miss a message.
            warn!(error = %e, event_id = %event_id, "slack dedup insert failed, continuing");
        }
    }

    // ── 2. Extract and filter the inner event ─────────────────────────
    let Some(event) = envelope.payload.get("event") else {
        debug!(event_id = %event_id, "events_api envelope has no 'event' field");
        return;
    };

    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let subtype = event.get("subtype").and_then(|v| v.as_str());
    let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
    let channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let ts = event.get("ts").and_then(|v| v.as_str()).unwrap_or("");
    let thread_ts = event.get("thread_ts").and_then(|v| v.as_str());
    let channel_type = event.get("channel_type").and_then(|v| v.as_str());

    // Self-loop guard — drop anything the bot posted itself.
    if user == state.bot_user_id || subtype == Some("bot_message") {
        debug!(event_id = %event_id, "dropping self-authored slack event");
        return;
    }

    // Phase-1 routing: `app_mention` in channels and `message` in DMs.
    // Everything else is dropped silently.
    let keep = match event_type {
        "app_mention" => true,
        "message" => matches!(channel_type, Some("im")),
        _ => false,
    };
    if !keep {
        debug!(
            event_id = %event_id,
            event_type,
            channel_type,
            "slack event filtered out (not app_mention or message.im)"
        );
        return;
    }

    // Slack `files` array on the event payload — present whenever the
    // user uploaded one or more files alongside the message (with or
    // without a caption). For DMs and app_mentions both, files appear
    // here as JSON objects with `id`, `name`, `mimetype`, `size`,
    // `url_private_download` (preferred), and `url_private`.
    let files: Vec<serde_json::Value> = event
        .get("files")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if channel.is_empty() {
        debug!(event_id = %event_id, "slack event missing channel");
        return;
    }
    if text.is_empty() && files.is_empty() {
        debug!(event_id = %event_id, "slack event has no text and no files, dropping");
        return;
    }

    // ── 3. Derive session key ─────────────────────────────────────────
    // `{team}:{channel}:{thread_root_ts}` — threaded replies all map to
    // the same session; top-level messages start a new thread session
    // keyed by their own ts.
    let thread_root = thread_ts.unwrap_or(ts);
    let session_key = format!("{}:{}:{}", team_id, channel, thread_root);

    info!(
        event_id = %event_id,
        team_id = %team_id,
        channel,
        session_key = %session_key,
        file_count = files.len(),
        "slack event accepted, running agent turn"
    );

    // ── 4. Download attachments (if any) ──────────────────────────────
    // We download eagerly here, before kicking off the agent stream, so
    // any failure surfaces as a clear Slack reply rather than a
    // mysteriously empty agent turn. Files are validated against the
    // agent's [attachments] config (size limit + extension allowlist).
    let agent_config = state.agent.config();
    let att_config = &agent_config.attachments;
    let attachments = download_slack_files(&state.web, &files, att_config, &event_id).await;

    // ── 5. Build ChatMessage ──────────────────────────────────────────
    // Pass the user's message as plain text (matching the Telegram
    // handler). All the routing metadata for slack lives on
    // `channel_id` / `channel_session_key` / `user_id`, not in any
    // wrapper around the user text.
    let sanitized = strip_bot_mention(text, &state.bot_user_id);

    let chat_msg = ChatMessage {
        text: sanitized.clone(),
        user_id: Some(user.to_string()),
        channel_id: Some("slack".into()),
        channel_session_key: Some(session_key.clone()),
        attachments,
        triggered_by: None,
        model: None,
    };

    // Copy the originals we still need after `chat_msg` is consumed.
    let user_text = sanitized.clone();
    let reply_channel = channel.to_string();
    let reply_thread = thread_root.to_string();

    // ── 6. Run the agent turn ─────────────────────────────────────────
    let (mut stream, session_id, _followup_tx, _out_attachments) =
        match state.agent.chat_stream(&chat_msg).await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, event_id = %event_id, "slack: failed to start agent stream");
                let _ = post_plain(
                    &state.web,
                    &reply_channel,
                    &reply_thread,
                    &format!("Sorry, an error occurred: {e}"),
                )
                .await;
                return;
            }
        };

    let mut last_assistant_text = String::new();
    let mut all_text = String::new();
    let mut result_msg = None;

    while let Some(msg_result) = stream.next().await {
        match msg_result {
            Ok(Message::Assistant(assistant)) => {
                let t = extract_assistant_text(&assistant.content);
                if !t.is_empty() {
                    last_assistant_text = t.clone();
                    if !all_text.is_empty() {
                        all_text.push('\n');
                    }
                    all_text.push_str(&t);
                    if state.stream_mode == "all_messages" {
                        // Post each assistant message as it arrives.
                        post_reply(&state.web, &reply_channel, &reply_thread, &t).await;
                    }
                }
            }
            Ok(Message::Result(result)) => {
                if last_assistant_text.is_empty() {
                    if let Some(t) = &result.result {
                        last_assistant_text = t.clone();
                        if all_text.is_empty() {
                            all_text = t.clone();
                        }
                    }
                }
                result_msg = Some(result);
            }
            Ok(_) => {}
            Err(e) => {
                error!(error = %e, event_id = %event_id, "slack: agent stream error");
                let _ = post_plain(
                    &state.web,
                    &reply_channel,
                    &reply_thread,
                    &format!("Sorry, an error occurred: {e}"),
                )
                .await;
                return;
            }
        }
    }

    // ── 7. Post the final reply (final_only mode) ─────────────────────
    if state.stream_mode != "all_messages" {
        if last_assistant_text.is_empty() {
            post_reply(&state.web, &reply_channel, &reply_thread, "(no response)").await;
        } else {
            post_reply(
                &state.web,
                &reply_channel,
                &reply_thread,
                &last_assistant_text,
            )
            .await;
        }
    } else if all_text.is_empty() {
        post_reply(&state.web, &reply_channel, &reply_thread, "(no response)").await;
    }

    // ── 8. Finalize usage bookkeeping ─────────────────────────────────
    if let Some(ref result) = result_msg {
        state
            .agent
            .finalize_chat(&session_id, &user_text, &all_text, result, Some(user))
            .await;
    }
}

/// Post an assistant reply in a Slack thread, converting from markdown
/// to mrkdwn and splitting long messages into multiple chunks.
async fn post_reply(web: &SlackWebClient, channel: &str, thread_ts: &str, text: &str) {
    let converted = markdown_to_mrkdwn(text);
    for chunk in split_for_slack(&converted) {
        let args = ChatPostMessageArgs::new(channel, &chunk).in_thread(thread_ts);
        if let Err(e) = web.chat_post_message(args).await {
            warn!(error = %e, channel, "slack chat.postMessage failed");
        }
    }
}

/// Post a plain-text reply (used for error messages so we don't try to
/// mrkdwn-escape untrusted error strings).
async fn post_plain(
    web: &SlackWebClient,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> crate::error::Result<String> {
    let args = ChatPostMessageArgs::new(channel, text).in_thread(thread_ts);
    web.chat_post_message(args).await
}

/// Strip the leading `<@BOT_USER_ID>` mention (and optional whitespace)
/// from a text so the agent doesn't see its own user id as part of the
/// message.
fn strip_bot_mention(text: &str, bot_user_id: &str) -> String {
    let needle = format!("<@{}>", bot_user_id);
    text.replacen(&needle, "", 1).trim().to_string()
}

/// Download every file referenced in a Slack `event.files` array,
/// validate it against the agent's [attachments] config, base64-encode
/// the bytes, and return a `Vec<Attachment>` ready to attach to a
/// `ChatMessage`.
///
/// Failures are logged with `warn!` (size limit, scope missing, network
/// error) and the offending file is skipped — we never fail the whole
/// turn just because one file couldn't be loaded. The agent will still
/// see whatever attachments succeeded plus the user's text.
async fn download_slack_files(
    web: &SlackWebClient,
    files: &[Value],
    att_config: &AttachmentsConfig,
    event_id: &str,
) -> Vec<Attachment> {
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        // Slack returns `url_private_download` (forces a Content-
        // Disposition: attachment header) and `url_private` (inline).
        // The download endpoint is preferred — both work with the bot
        // token, but the download URL is the canonical one.
        let url = file
            .get("url_private_download")
            .or_else(|| file.get("url_private"))
            .and_then(|v| v.as_str());
        let Some(url) = url else {
            warn!(
                event_id,
                "slack file has no url_private_download / url_private, skipping"
            );
            continue;
        };

        let file_name = file
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("file")
            .to_string();
        let mime_type = file
            .get("mimetype")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream")
            .to_string();
        let declared_size = file.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        // Cheap pre-check against the size limit using the size Slack
        // already gave us, so we don't waste a download on a file the
        // agent will reject anyway.
        if declared_size > 0 {
            if let Err(reason) = att_config.validate(&file_name, declared_size) {
                warn!(event_id, file = %file_name, "{}", reason);
                continue;
            }
        }

        let bytes = match web.download_file(url).await {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    event_id,
                    file = %file_name,
                    error = %e,
                    "slack file download failed (check files:read scope and re-install the app)"
                );
                continue;
            }
        };

        // Re-validate against the actual byte length (the declared size
        // is sometimes 0 for screenshots / freshly uploaded files).
        if let Err(reason) = att_config.validate(&file_name, bytes.len()) {
            warn!(event_id, file = %file_name, "{}", reason);
            continue;
        }

        out.push(Attachment {
            file_name,
            mime_type,
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        });
    }
    out
}

/// Collect all assistant text blocks into a single string, ignoring
/// tool-use blocks.
fn extract_assistant_text(content: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in content {
        if let ContentBlock::Text { text } = block {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
        }
    }
    out
}

fn extract_event_id(payload: &Value) -> Option<&str> {
    payload.get("event_id").and_then(|v| v.as_str())
}

fn extract_team_id(payload: &Value) -> Option<&str> {
    payload.get("team_id").and_then(|v| v.as_str())
}

/// Production [`EnvelopeHandler`] that dispatches `events_api` envelopes
/// to the agent runner in a detached task.
///
/// - `Hello` envelopes are logged.
/// - `EventsApi` envelopes spawn a `handle_event` task so slow LLM turns
///   do NOT block the receive loop (which would risk missing Slack's
///   ~3-second ack window — though we always ack before handing off).
/// - `SlashCommands` / `Interactive` are logged and dropped (phase 1).
/// - `Disconnect` is handled by the socket loop itself; the handler sees
///   it only for logging.
///
/// The struct owns `HandlerState` so cloning it is cheap — every field
/// inside `HandlerState` is an `Arc` or an inexpensive copy.
pub struct AgentHandler {
    pub state: HandlerState,
}

impl EnvelopeHandler for AgentHandler {
    fn handle(&self, envelope: Envelope) {
        match envelope {
            Envelope::EventsApi(e) => {
                let state = self.state.clone();
                tokio::spawn(async move {
                    handle_event(state, e).await;
                });
            }
            Envelope::Hello(h) => {
                if h.num_connections > 1 {
                    warn!(
                        num_connections = h.num_connections,
                        "slack hello: multiple active connections share this app token"
                    );
                } else {
                    info!(
                        num_connections = h.num_connections,
                        "slack hello — ready to receive events"
                    );
                }
            }
            Envelope::SlashCommands(s) => {
                debug!(envelope_id = %s.envelope_id, "slack slash_commands ignored (phase 1)");
            }
            Envelope::Interactive(i) => {
                debug!(envelope_id = %i.envelope_id, "slack interactive ignored (phase 1)");
            }
            Envelope::Disconnect(d) => {
                info!(reason = ?d.reason, "slack disconnect envelope received");
            }
            Envelope::Unknown => {
                debug!("slack unknown envelope type, ignored");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_bot_mention_removes_leading_mention() {
        let text = "<@U12345> what's my schedule?";
        assert_eq!(strip_bot_mention(text, "U12345"), "what's my schedule?");
    }

    #[test]
    fn strip_bot_mention_leaves_other_mentions_alone() {
        let text = "<@U12345> ping <@U99999>";
        assert_eq!(strip_bot_mention(text, "U12345"), "ping <@U99999>");
    }

    #[test]
    fn strip_bot_mention_no_mention_is_idempotent() {
        assert_eq!(strip_bot_mention("hello", "U12345"), "hello");
    }

    #[test]
    fn extract_event_id_from_payload() {
        let v = serde_json::json!({ "event_id": "Ev_ABC" });
        assert_eq!(extract_event_id(&v), Some("Ev_ABC"));
    }

    #[test]
    fn extract_team_id_from_payload() {
        let v = serde_json::json!({ "team_id": "T_XYZ" });
        assert_eq!(extract_team_id(&v), Some("T_XYZ"));
    }

    #[test]
    fn extract_assistant_text_concatenates_text_blocks() {
        let blocks = vec![
            ContentBlock::Text {
                text: "line one".into(),
            },
            ContentBlock::Text {
                text: "line two".into(),
            },
        ];
        assert_eq!(extract_assistant_text(&blocks), "line one\nline two");
    }
}
