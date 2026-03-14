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
use orion_core::FollowupMode;

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
    // Check API key if configured
    if let Some(expected) = &state.api_key {
        match &params.token {
            Some(token) if token == expected => {}
            _ => {
                return axum::http::Response::builder()
                    .status(401)
                    .body(axum::body::Body::from("Unauthorized"))
                    .unwrap()
                    .into_response();
            }
        }
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
        .into_response()
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
        name: String,
        input: serde_json::Value,
    },

    /// Tool result returned.
    #[serde(rename = "tool_result")]
    ToolResult { content: String, is_error: bool },

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
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let followup_mode = state.config.followup_mode;

    debug!("WebSocket client connected");

    // Active stream state — holds the followup sender (inject mode) or
    // queued messages (queue mode) while a stream is running.
    let mut active_followup_tx: Option<mpsc::UnboundedSender<String>> = None;
    let mut queued_messages: Vec<orion_core::ChatMessage> = Vec::new();

    loop {
        // If there is no active stream but we have queued messages (queue mode),
        // start a new stream for the next queued message batch.
        if active_followup_tx.is_none() && !queued_messages.is_empty() {
            let batch = std::mem::take(&mut queued_messages);
            let combined_text = batch.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n\n");
            // Use the channel info from the first message
            let first = &batch[0];
            let chat_msg = orion_core::ChatMessage {
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

        // Wait for the next WS message from the client.
        let msg = match receiver.next().await {
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
            } => {
                let chat_msg = orion_core::ChatMessage {
                    text: text.clone(),
                    user_id,
                    channel_id: channel_id.or(Some("main".into())),
                    channel_session_key,
                    attachments: Vec::new(),
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
                    &state,
                    &session_id,
                    &text,
                    followup_mode,
                    active_followup_tx.as_ref().unwrap(),
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
    state: &Arc<AppState>,
    session_id: &str,
    user_text: &str,
    followup_mode: FollowupMode,
    followup_tx: &mpsc::UnboundedSender<String>,
    queued_messages: &mut Vec<orion_core::ChatMessage>,
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
                        let _ = send_msg(sender, &ServerMessage::Error {
                            message: format!("Stream error: {}", e),
                        }).await;
                        return true;
                    }
                    None => {
                        // Stream ended without a Result message
                        return true;
                    }
                }
            }

            // Branch 2: New message from the WebSocket client
            ws_msg = StreamExt::next(receiver) => {
                match ws_msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        if let Ok(ClientMessage::Message { text, user_id, channel_id, channel_session_key }) = serde_json::from_str::<ClientMessage>(&text) {
                            match followup_mode {
                                FollowupMode::Inject => {
                                    debug!(text = %text, "Injecting followup into active agent loop");
                                    let _ = followup_tx.send(text);
                                }
                                FollowupMode::Queue => {
                                    debug!(text = %text, "Queuing message for after current stream");
                                    queued_messages.push(orion_core::ChatMessage {
                                        text,
                                        user_id,
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
                    ContentBlock::ToolUse { name, input, .. } => {
                        let tool_json = serde_json::json!({
                            "type": "tool_use",
                            "name": name,
                            "input": input,
                        });
                        let _ = state.agent.session_mgr().save_message(
                            session_id, "tool_use",
                            &serde_json::to_string(&tool_json).unwrap_or_default(),
                        ).await;

                        if !send_msg(sender, &ServerMessage::ToolUse {
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
                if let ContentBlock::ToolResult { content, is_error, .. } = block {
                    let content_str = content
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| serde_json::to_string(content).unwrap_or_default());

                    let preview = if content_str.len() > 500 {
                        format!("{}...", &content_str[..500])
                    } else {
                        content_str
                    };

                    let tool_result_json = serde_json::json!({
                        "type": "tool_result",
                        "content": &preview,
                        "is_error": is_error.unwrap_or(false),
                    });
                    let _ = state.agent.session_mgr().save_message(
                        session_id, "tool_result",
                        &serde_json::to_string(&tool_result_json).unwrap_or_default(),
                    ).await;

                    if !send_msg(sender, &ServerMessage::ToolResult {
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

            state.agent.finalize_chat(session_id, user_text, result_text, &result).await;

            if !result_text.is_empty() {
                let _ = state.agent.session_mgr().save_message(session_id, "assistant", result_text).await;
            }

            let _ = send_msg(sender, &ServerMessage::StreamEnd {
                session_id: session_id.to_string(),
                num_turns: result.num_turns,
                cost_usd: result.total_cost_usd,
                input_tokens: result.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
                output_tokens: result.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
                is_error: result.is_error,
                errors: result.errors.clone(),
            }).await;

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
                let _ = send_msg(sender, &ServerMessage::Error {
                    message: format!("Stream error: {}", e),
                }).await;
                return;
            }
        }
    }
}
