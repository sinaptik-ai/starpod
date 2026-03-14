use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

use agent_sdk::{ContentBlock, Message};

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
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();

    debug!("WebSocket client connected");

    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(WsMessage::Text(text)) => text,
            Ok(WsMessage::Close(_)) => {
                debug!("WebSocket client disconnected");
                break;
            }
            Ok(_) => continue,
            Err(e) => {
                error!(error = %e, "WebSocket receive error");
                break;
            }
        };

        let client_msg: ClientMessage = match serde_json::from_str(&msg) {
            Ok(m) => m,
            Err(e) => {
                let _ = send_msg(
                    &mut sender,
                    &ServerMessage::Error {
                        message: format!("Invalid message format: {}", e),
                    },
                )
                .await;
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
                // Build ChatMessage for channel-aware session routing
                let chat_msg = orion_core::ChatMessage {
                    text: text.clone(),
                    user_id,
                    channel_id: channel_id.or(Some("main".into())),
                    channel_session_key,
                    attachments: Vec::new(),
                };

                // Start streaming chat
                let (mut stream, session_id) = match state.agent.chat_stream(&chat_msg).await {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = send_msg(
                            &mut sender,
                            &ServerMessage::Error {
                                message: format!("Chat error: {}", e),
                            },
                        )
                        .await;
                        continue;
                    }
                };

                // Save user message
                let _ = state.agent.session_mgr().save_message(&session_id, "user", &text).await;

                // Send stream_start
                if !send_msg(
                    &mut sender,
                    &ServerMessage::StreamStart {
                        session_id: session_id.clone(),
                    },
                )
                .await
                {
                    break;
                }

                let mut result_text = String::new();

                // Stream messages to the client
                while let Some(msg_result) = stream.next().await {
                    match msg_result {
                        Ok(Message::Assistant(assistant)) => {
                            for block in &assistant.content {
                                match block {
                                    ContentBlock::Text { text } => {
                                        if !text.is_empty() {
                                            if !result_text.is_empty() {
                                                result_text.push('\n');
                                            }
                                            result_text.push_str(text);

                                            if !send_msg(
                                                &mut sender,
                                                &ServerMessage::TextDelta {
                                                    text: text.clone(),
                                                },
                                            )
                                            .await
                                            {
                                                return;
                                            }
                                        }
                                    }
                                    ContentBlock::ToolUse { name, input, .. } => {
                                        // Save tool_use as a message
                                        let tool_json = serde_json::json!({
                                            "type": "tool_use",
                                            "name": name,
                                            "input": input,
                                        });
                                        let _ = state.agent.session_mgr().save_message(
                                            &session_id, "tool_use",
                                            &serde_json::to_string(&tool_json).unwrap_or_default(),
                                        ).await;

                                        if !send_msg(
                                            &mut sender,
                                            &ServerMessage::ToolUse {
                                                name: name.clone(),
                                                input: input.clone(),
                                            },
                                        )
                                        .await
                                        {
                                            return;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Ok(Message::User(user)) => {
                            for block in &user.content {
                                if let ContentBlock::ToolResult {
                                    content, is_error, ..
                                } = block
                                {
                                    let content_str = content
                                        .as_str()
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| {
                                            serde_json::to_string(content).unwrap_or_default()
                                        });

                                    // Truncate tool results for the WS stream
                                    let preview = if content_str.len() > 500 {
                                        format!("{}...", &content_str[..500])
                                    } else {
                                        content_str
                                    };

                                    // Save tool_result as a message
                                    let tool_result_json = serde_json::json!({
                                        "type": "tool_result",
                                        "content": &preview,
                                        "is_error": is_error.unwrap_or(false),
                                    });
                                    let _ = state.agent.session_mgr().save_message(
                                        &session_id, "tool_result",
                                        &serde_json::to_string(&tool_result_json).unwrap_or_default(),
                                    ).await;

                                    if !send_msg(
                                        &mut sender,
                                        &ServerMessage::ToolResult {
                                            content: preview,
                                            is_error: is_error.unwrap_or(false),
                                        },
                                    )
                                    .await
                                    {
                                        return;
                                    }
                                }
                            }
                        }
                        Ok(Message::Result(result)) => {
                            if result_text.is_empty() {
                                if let Some(text) = &result.result {
                                    result_text = text.clone();
                                }
                            }

                            // Finalize chat (record usage + daily log)
                            state
                                .agent
                                .finalize_chat(&session_id, &text, &result_text, &result)
                                .await;

                            // Save assistant message
                            if !result_text.is_empty() {
                                let _ = state.agent.session_mgr().save_message(&session_id, "assistant", &result_text).await;
                            }

                            let _ = send_msg(
                                &mut sender,
                                &ServerMessage::StreamEnd {
                                    session_id: session_id.clone(),
                                    num_turns: result.num_turns,
                                    cost_usd: result.total_cost_usd,
                                    input_tokens: result
                                        .usage
                                        .as_ref()
                                        .map(|u| u.input_tokens)
                                        .unwrap_or(0),
                                    output_tokens: result
                                        .usage
                                        .as_ref()
                                        .map(|u| u.output_tokens)
                                        .unwrap_or(0),
                                    is_error: result.is_error,
                                    errors: result.errors.clone(),
                                },
                            )
                            .await;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            error!(error = %e, "Stream error");
                            let _ = send_msg(
                                &mut sender,
                                &ServerMessage::Error {
                                    message: format!("Stream error: {}", e),
                                },
                            )
                            .await;
                            break;
                        }
                    }
                }
            }
        }
    }
}
