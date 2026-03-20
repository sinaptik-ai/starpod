use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error};

use agent_sdk::{ContentBlock, Message};
use starpod_core::FollowupMode;

use crate::AppState;

/// Build WebSocket routes.
pub fn ws_routes() -> Router<Arc<AppState>> {
    Router::new().route("/ws", get(ws_handler))
}

/// Optional query params for WS upgrade (auth token).
#[derive(Debug, Deserialize, Default)]
struct WsQuery {
    #[serde(default)]
    token: Option<String>,
}

/// WebSocket upgrade handler.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(params): Query<WsQuery>,
) -> impl IntoResponse {
    // Authenticate via token query param
    let has_users = state.auth.has_users().await.unwrap_or(false);
    if has_users {
        let token = match &params.token {
            Some(t) => t.as_str(),
            None => {
                return axum::http::Response::builder()
                    .status(401)
                    .body(axum::body::Body::from("Missing token parameter"))
                    .unwrap()
                    .into_response();
            }
        };
        match state.auth.authenticate_api_key(token).await {
            Ok(Some(user)) => {
                let user = Some(user);
                return ws.max_frame_size(1024 * 1024)
                    .max_message_size(1024 * 1024)
                    .on_upgrade(move |socket| handle_socket(socket, state, user))
                    .into_response();
            }
            Ok(None) => {
                return axum::http::Response::builder()
                    .status(401)
                    .body(axum::body::Body::from("Invalid token"))
                    .unwrap()
                    .into_response();
            }
            Err(_) => {
                return axum::http::Response::builder()
                    .status(500)
                    .body(axum::body::Body::from("Auth error"))
                    .unwrap()
                    .into_response();
            }
        }
    }

    ws.max_frame_size(1024 * 1024) // 1 MB
        .max_message_size(1024 * 1024) // 1 MB
        .on_upgrade(move |socket| handle_socket(socket, state, None))
        .into_response()
}

/// A file attachment sent over WebSocket as base64.
#[derive(Debug, Deserialize)]
struct WsAttachment {
    file_name: String,
    mime_type: String,
    /// Base64-encoded file data.
    data: String,
}

/// Client → Server message.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum ClientMessage {
    #[serde(rename = "message")]
    Message {
        text: String,
        #[serde(default)]
        user_id: Option<String>,
        #[serde(default)]
        channel_id: Option<String>,
        #[serde(default)]
        channel_session_key: Option<String>,
        #[serde(default)]
        attachments: Vec<WsAttachment>,
    },
}

/// Server → Client streaming messages.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    /// Stream started — includes session info.
    #[serde(rename = "stream_start")]
    StreamStart { session_id: String },

    /// Text delta from the assistant.
    #[serde(rename = "text_delta")]
    TextDelta { text: String },

    /// Tool use started.
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    /// Tool result returned.
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },

    /// Stream completed with final stats.
    #[serde(rename = "stream_end")]
    StreamEnd {
        session_id: String,
        num_turns: u32,
        cost_usd: f64,
        input_tokens: u64,
        output_tokens: u64,
        is_error: bool,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        errors: Vec<String>,
    },

    /// Error message.
    #[serde(rename = "error")]
    Error { message: String },

    /// Cron job or heartbeat completed — pushed to all connected clients.
    ///
    /// The frontend uses this to show a toast notification and refresh the
    /// session sidebar. If `session_id` is non-empty, clicking the toast
    /// navigates to the cron job's session transcript.
    #[serde(rename = "notification")]
    Notification {
        job_name: String,
        session_id: String,
        result_preview: String,
        success: bool,
    },
}

/// Send a ServerMessage over the WebSocket. Returns false if the send failed.
async fn send_msg(
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    msg: &ServerMessage,
) -> bool {
    let json = serde_json::to_string(msg).unwrap();
    sender.send(WsMessage::Text(json.into())).await.is_ok()
}

/// Handle an individual WebSocket connection with streaming.
///
/// Supports two followup modes:
/// - **Inject**: Messages arriving during an active stream are sent through the
///   followup channel and integrated into the next agent loop iteration.
/// - **Queue**: Messages are buffered and dispatched as new agent loops after
///   the current stream finishes.
async fn handle_socket(socket: WebSocket, state: Arc<AppState>, auth_user: Option<starpod_auth::User>) {
    let (mut sender, mut receiver) = socket.split();
    let followup_mode = state.config.read().unwrap().followup_mode;
    let mut events_rx = state.events_tx.subscribe();

    debug!("WebSocket client connected");

    // Active stream state — holds the followup sender (inject mode) or
    // queued messages (queue mode) while a stream is running.
    let mut active_followup_tx: Option<mpsc::UnboundedSender<String>> = None;
    let mut queued_messages: Vec<starpod_core::ChatMessage> = Vec::new();

    loop {
        // If there is no active stream but we have queued messages (queue mode),
        // start a new stream for the next queued message batch.
        if active_followup_tx.is_none() && !queued_messages.is_empty() {
            let batch = std::mem::take(&mut queued_messages);
            let combined_text = batch.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n\n");
            // Use the channel info from the first message
            let first = &batch[0];
            let chat_msg = starpod_core::ChatMessage {
                text: combined_text.clone(),
                user_id: first.user_id.clone(),
                channel_id: first.channel_id.clone(),
                channel_session_key: first.channel_session_key.clone(),
                attachments: Vec::new(),
            };

            let (mut stream, session_id, _followup_tx) = match state.agent.chat_stream(&chat_msg).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = send_msg(&mut sender, &ServerMessage::Error {
                        message: format!("Chat error: {}", e),
                    }).await;
                    continue;
                }
            };

            let _ = state.agent.session_mgr().save_message(&session_id, "user", &combined_text).await;

            if !send_msg(&mut sender, &ServerMessage::StreamStart {
                session_id: session_id.clone(),
            }).await {
                break;
            }

            // Consume this queued batch stream (no followup injection needed here
            // since we're replaying buffered messages after the main stream finished).
            process_stream(&mut stream, &mut sender, &state, &session_id, &combined_text).await;
            continue;
        }

        // Wait for the next WS message from the client OR a broadcast event.
        let msg = tokio::select! {
            ws_msg = receiver.next() => {
                match ws_msg {
                    Some(Ok(WsMessage::Text(text))) => text,
                    Some(Ok(WsMessage::Close(_))) => {
                        debug!("WebSocket client disconnected");
                        break;
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        error!(error = %e, "WebSocket receive error");
                        break;
                    }
                    None => break,
                }
            }
            event = events_rx.recv() => {
                match event {
                    Ok(crate::GatewayEvent::CronComplete { job_name, session_id, result_preview, success }) => {
                        if !send_msg(&mut sender, &ServerMessage::Notification {
                            job_name, session_id, result_preview, success,
                        }).await {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        debug!(skipped = n, "WS client lagged behind broadcast events");
                    }
                    Err(_) => break, // channel closed
                }
                continue;
            }
        };

        let client_msg: ClientMessage = match serde_json::from_str(&msg) {
            Ok(m) => m,
            Err(e) => {
                let _ = send_msg(&mut sender, &ServerMessage::Error {
                    message: format!("Invalid message format: {}", e),
                }).await;
                continue;
            }
        };

        match client_msg {
            ClientMessage::Message {
                text,
                user_id,
                channel_id,
                channel_session_key,
                attachments,
            } => {
                // Validate and convert attachments
                let ws_config = state.config.read().unwrap().clone();
                let att_config = &ws_config.attachments;
                let mut chat_attachments = Vec::new();
                for att in attachments {
                    let raw_size = att.data.len() * 3 / 4;
                    if let Err(reason) = att_config.validate(&att.file_name, raw_size) {
                        let _ = send_msg(
                            &mut sender,
                            &ServerMessage::Error { message: reason },
                        )
                        .await;
                        continue;
                    }
                    chat_attachments.push(starpod_core::Attachment {
                        file_name: att.file_name,
                        mime_type: att.mime_type,
                        data: att.data,
                    });
                }

                // Build ChatMessage for channel-aware session routing
                // Use authenticated user_id — client-provided user_id is ignored
                let effective_user_id = auth_user.as_ref().map(|u| u.id.clone()).or(user_id);
                let chat_msg = starpod_core::ChatMessage {
                    text: text.clone(),
                    user_id: effective_user_id,
                    channel_id: channel_id.or(Some("main".into())),
                    channel_session_key,
                    attachments: chat_attachments,
                };

                // If a stream is already active, handle according to followup_mode
                if let Some(ref followup_tx) = active_followup_tx {
                    match followup_mode {
                        FollowupMode::Inject => {
                            debug!(text = %text, "Injecting followup into active agent loop");
                            let _ = followup_tx.send(text);
                        }
                        FollowupMode::Queue => {
                            debug!(text = %text, "Queuing message for after current stream");
                            queued_messages.push(chat_msg);
                        }
                    }
                    continue;
                }

                // No active stream — start a new one
                let (mut stream, session_id, followup_tx) =
                    match state.agent.chat_stream(&chat_msg).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = send_msg(&mut sender, &ServerMessage::Error {
                                message: format!("Chat error: {}", e),
                            }).await;
                            continue;
                        }
                    };

                let _ = state.agent.session_mgr().save_message(&session_id, "user", &text).await;

                if !send_msg(&mut sender, &ServerMessage::StreamStart {
                    session_id: session_id.clone(),
                }).await {
                    break;
                }

                active_followup_tx = Some(followup_tx);

                // Process the stream, concurrently listening for new WS messages
                let stream_done = process_stream_with_followups(
                    &mut stream,
                    &mut sender,
                    &mut receiver,
                    &mut events_rx,
                    &state,
                    &session_id,
                    &text,
                    followup_mode,
                    active_followup_tx.as_ref().unwrap(),
                    &auth_user,
                    &mut queued_messages,
                ).await;

                active_followup_tx = None;

                if !stream_done {
                    // WS was closed during streaming
                    break;
                }
            }
        }
    }
}

/// Process a stream to completion, concurrently accepting new WS messages
/// for followup injection or queuing.
async fn process_stream_with_followups(
    stream: &mut agent_sdk::Query,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    receiver: &mut futures::stream::SplitStream<WebSocket>,
    events_rx: &mut tokio::sync::broadcast::Receiver<crate::GatewayEvent>,
    state: &Arc<AppState>,
    session_id: &str,
    user_text: &str,
    followup_mode: FollowupMode,
    followup_tx: &mpsc::UnboundedSender<String>,
    auth_user: &Option<starpod_auth::User>,
    queued_messages: &mut Vec<starpod_core::ChatMessage>,
) -> bool {
    let mut result_text = String::new();

    loop {
        tokio::select! {
            // Branch 1: Next message from the agent stream
            stream_msg = StreamExt::next(stream) => {
                match stream_msg {
                    Some(Ok(msg)) => {
                        let action = handle_stream_message(
                            msg, sender, state, session_id, user_text, &mut result_text,
                        ).await;
                        match action {
                            StreamAction::Continue => {}
                            StreamAction::Done => return true,
                            StreamAction::Disconnected => return false,
                        }
                    }
                    Some(Err(e)) => {
                        error!(error = %e, "Stream error");
                        // Save any accumulated assistant text before reporting the error
                        if !result_text.is_empty() {
                            let _ = state.agent.session_mgr().save_message(
                                session_id, "assistant", &result_text,
                            ).await;
                        }
                        let _ = send_msg(sender, &ServerMessage::StreamEnd {
                            session_id: session_id.to_string(),
                            num_turns: 0,
                            cost_usd: 0.0,
                            input_tokens: 0,
                            output_tokens: 0,
                            is_error: true,
                            errors: vec![format!("Stream error: {}", e)],
                        }).await;
                        return true;
                    }
                    None => {
                        // Stream ended without a Result message — save any
                        // accumulated text and notify the client so the UI
                        // cursor stops blinking.
                        if !result_text.is_empty() {
                            let _ = state.agent.session_mgr().save_message(
                                session_id, "assistant", &result_text,
                            ).await;
                        }
                        let _ = send_msg(sender, &ServerMessage::StreamEnd {
                            session_id: session_id.to_string(),
                            num_turns: 0,
                            cost_usd: 0.0,
                            input_tokens: 0,
                            output_tokens: 0,
                            is_error: false,
                            errors: Vec::new(),
                        }).await;
                        return true;
                    }
                }
            }

            // Branch 2: New message from the WebSocket client
            ws_msg = StreamExt::next(receiver) => {
                match ws_msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        if let Ok(ClientMessage::Message { text, user_id, channel_id, channel_session_key, attachments: _ }) = serde_json::from_str::<ClientMessage>(&text) {
                            match followup_mode {
                                FollowupMode::Inject => {
                                    debug!(text = %text, "Injecting followup into active agent loop");
                                    let _ = followup_tx.send(text);
                                }
                                FollowupMode::Queue => {
                                    debug!(text = %text, "Queuing message for after current stream");
                                    let effective_user_id = auth_user.as_ref().map(|u| u.id.clone()).or(user_id);
                                    queued_messages.push(starpod_core::ChatMessage {
                                        text,
                                        user_id: effective_user_id,
                                        channel_id: channel_id.or(Some("main".into())),
                                        channel_session_key,
                                        attachments: Vec::new(),
                                    });
                                }
                            }
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) => {
                        debug!("WebSocket client disconnected during stream");
                        return false;
                    }
                    Some(Err(e)) => {
                        error!(error = %e, "WebSocket receive error during stream");
                        return false;
                    }
                    Some(Ok(_)) => {}
                    None => return false,
                }
            }

            // Branch 3: Broadcast event (cron notification)
            event = events_rx.recv() => {
                match event {
                    Ok(crate::GatewayEvent::CronComplete { job_name, session_id: sid, result_preview, success }) => {
                        if !send_msg(sender, &ServerMessage::Notification {
                            job_name, session_id: sid, result_preview, success,
                        }).await {
                            return false;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        debug!(skipped = n, "WS client lagged behind broadcast events during stream");
                    }
                    Err(_) => {} // channel closed, ignore during stream
                }
            }
        }
    }
}

/// Result of handling a single stream message.
enum StreamAction {
    Continue,
    Done,
    Disconnected,
}

/// Handle a single message from the agent stream, forwarding to the WS client.
async fn handle_stream_message(
    msg: Message,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    state: &Arc<AppState>,
    session_id: &str,
    user_text: &str,
    result_text: &mut String,
) -> StreamAction {
    match msg {
        Message::Assistant(assistant) => {
            for block in &assistant.content {
                match block {
                    ContentBlock::Text { text } => {
                        if !text.is_empty() {
                            if !result_text.is_empty() {
                                result_text.push('\n');
                            }
                            result_text.push_str(text);

                            if !send_msg(sender, &ServerMessage::TextDelta {
                                text: text.clone(),
                            }).await {
                                return StreamAction::Disconnected;
                            }
                        }
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        let tool_json = serde_json::json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        });
                        let _ = state.agent.session_mgr().save_message(
                            session_id, "tool_use",
                            &serde_json::to_string(&tool_json).unwrap_or_default(),
                        ).await;

                        if !send_msg(sender, &ServerMessage::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        }).await {
                            return StreamAction::Disconnected;
                        }
                    }
                    _ => {}
                }
            }
        }
        Message::User(user) => {
            for block in &user.content {
                if let ContentBlock::ToolResult { tool_use_id, content, is_error } = block {
                    let content_str = content
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| serde_json::to_string(content).unwrap_or_default());

                    let preview = if content_str.len() > 500 {
                        let mut end = 500;
                        while end > 0 && !content_str.is_char_boundary(end) { end -= 1; }
                        format!("{}...", &content_str[..end])
                    } else {
                        content_str
                    };

                    let tool_result_json = serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": &preview,
                        "is_error": is_error.unwrap_or(false),
                    });
                    let _ = state.agent.session_mgr().save_message(
                        session_id, "tool_result",
                        &serde_json::to_string(&tool_result_json).unwrap_or_default(),
                    ).await;

                    if !send_msg(sender, &ServerMessage::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: preview,
                        is_error: is_error.unwrap_or(false),
                    }).await {
                        return StreamAction::Disconnected;
                    }
                }
            }
        }
        Message::Result(result) => {
            if result_text.is_empty() {
                if let Some(text) = &result.result {
                    *result_text = text.clone();
                }
            }

            // Send StreamEnd immediately so the client stops showing the loading state.
            let _ = send_msg(sender, &ServerMessage::StreamEnd {
                session_id: session_id.to_string(),
                num_turns: result.num_turns,
                cost_usd: result.total_cost_usd,
                input_tokens: result.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
                output_tokens: result.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
                is_error: result.is_error,
                errors: result.errors.clone(),
            }).await;

            // Finalize in background so we don't block the client.
            let agent = Arc::clone(&state.agent);
            let sid = session_id.to_string();
            let ut = user_text.to_string();
            let rt = result_text.clone();
            tokio::spawn(async move {
                agent.finalize_chat(&sid, &ut, &rt, &result).await;
                if !rt.is_empty() {
                    let _ = agent.session_mgr().save_message(&sid, "assistant", &rt).await;
                }
            });

            return StreamAction::Done;
        }
        _ => {}
    }
    StreamAction::Continue
}

/// Process a stream to completion (no concurrent WS listening — used for queued batch replay).
async fn process_stream(
    stream: &mut agent_sdk::Query,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    state: &Arc<AppState>,
    session_id: &str,
    user_text: &str,
) {
    let mut result_text = String::new();
    while let Some(msg_result) = StreamExt::next(stream).await {
        match msg_result {
            Ok(msg) => {
                let action = handle_stream_message(
                    msg, sender, state, session_id, user_text, &mut result_text,
                ).await;
                match action {
                    StreamAction::Continue => {}
                    StreamAction::Done | StreamAction::Disconnected => return,
                }
            }
            Err(e) => {
                error!(error = %e, "Stream error");
                if !result_text.is_empty() {
                    let _ = state.agent.session_mgr().save_message(
                        session_id, "assistant", &result_text,
                    ).await;
                }
                let _ = send_msg(sender, &ServerMessage::StreamEnd {
                    session_id: session_id.to_string(),
                    num_turns: 0,
                    cost_usd: 0.0,
                    input_tokens: 0,
                    output_tokens: 0,
                    is_error: true,
                    errors: vec![format!("Stream error: {}", e)],
                }).await;
                return;
            }
        }
    }
    // Stream ended without a Result message — send stream_end so the UI
    // cursor stops and any accumulated text is persisted.
    if !result_text.is_empty() {
        let _ = state.agent.session_mgr().save_message(
            session_id, "assistant", &result_text,
        ).await;
    }
    let _ = send_msg(sender, &ServerMessage::StreamEnd {
        session_id: session_id.to_string(),
        num_turns: 0,
        cost_usd: 0.0,
        input_tokens: 0,
        output_tokens: 0,
        is_error: false,
        errors: Vec::new(),
    }).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_use_serializes_with_id() {
        let msg = ServerMessage::ToolUse {
            id: "toolu_abc123".into(),
            name: "Read".into(),
            input: serde_json::json!({"path": "/tmp/file.txt"}),
        };
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&msg).unwrap(),
        )
        .unwrap();

        assert_eq!(json["type"], "tool_use");
        assert_eq!(json["id"], "toolu_abc123");
        assert_eq!(json["name"], "Read");
        assert_eq!(json["input"]["path"], "/tmp/file.txt");
    }

    #[test]
    fn tool_result_serializes_with_tool_use_id() {
        let msg = ServerMessage::ToolResult {
            tool_use_id: "toolu_abc123".into(),
            content: "file contents here".into(),
            is_error: false,
        };
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&msg).unwrap(),
        )
        .unwrap();

        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["tool_use_id"], "toolu_abc123");
        assert_eq!(json["content"], "file contents here");
        assert_eq!(json["is_error"], false);
    }

    #[test]
    fn tool_result_error_serializes_correctly() {
        let msg = ServerMessage::ToolResult {
            tool_use_id: "toolu_xyz789".into(),
            content: "permission denied".into(),
            is_error: true,
        };
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&msg).unwrap(),
        )
        .unwrap();

        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["tool_use_id"], "toolu_xyz789");
        assert_eq!(json["is_error"], true);
    }

    #[test]
    fn parallel_tool_uses_have_distinct_ids_in_json() {
        let tool_a = ServerMessage::ToolUse {
            id: "toolu_aaa".into(),
            name: "Read".into(),
            input: serde_json::json!({"path": "a.txt"}),
        };
        let tool_b = ServerMessage::ToolUse {
            id: "toolu_bbb".into(),
            name: "Read".into(),
            input: serde_json::json!({"path": "b.txt"}),
        };
        let result_a = ServerMessage::ToolResult {
            tool_use_id: "toolu_aaa".into(),
            content: "contents a".into(),
            is_error: false,
        };
        let result_b = ServerMessage::ToolResult {
            tool_use_id: "toolu_bbb".into(),
            content: "contents b".into(),
            is_error: false,
        };

        let ja: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&tool_a).unwrap()).unwrap();
        let jb: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&tool_b).unwrap()).unwrap();
        let ra: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&result_a).unwrap()).unwrap();
        let rb: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&result_b).unwrap()).unwrap();

        // Tool uses carry distinct IDs
        assert_ne!(ja["id"], jb["id"]);
        assert_eq!(ja["id"], "toolu_aaa");
        assert_eq!(jb["id"], "toolu_bbb");

        // Results reference the correct tool
        assert_eq!(ra["tool_use_id"], "toolu_aaa");
        assert_eq!(rb["tool_use_id"], "toolu_bbb");
    }

    #[test]
    fn notification_serializes_correctly() {
        let msg = ServerMessage::Notification {
            job_name: "daily-summary".into(),
            session_id: "sess-abc-123".into(),
            result_preview: "No critical errors found today.".into(),
            success: true,
        };
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&msg).unwrap(),
        )
        .unwrap();

        assert_eq!(json["type"], "notification");
        assert_eq!(json["job_name"], "daily-summary");
        assert_eq!(json["session_id"], "sess-abc-123");
        assert_eq!(json["result_preview"], "No critical errors found today.");
        assert_eq!(json["success"], true);
    }

    #[test]
    fn notification_failure_serializes_correctly() {
        let msg = ServerMessage::Notification {
            job_name: "broken-job".into(),
            session_id: "".into(),
            result_preview: "connection refused".into(),
            success: false,
        };
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&msg).unwrap(),
        )
        .unwrap();

        assert_eq!(json["type"], "notification");
        assert_eq!(json["job_name"], "broken-job");
        assert_eq!(json["session_id"], "");
        assert_eq!(json["success"], false);
    }
}
