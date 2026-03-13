use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

use orion_core::ChatMessage;

use crate::AppState;

/// Build WebSocket routes.
pub fn ws_routes() -> Router<Arc<AppState>> {
    Router::new().route("/ws", get(ws_handler))
}

/// WebSocket upgrade handler.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Client → Server message.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "message")]
    Message {
        text: String,
        #[serde(default)]
        user_id: Option<String>,
        #[serde(default)]
        channel_id: Option<String>,
    },
}

/// Server → Client message.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "response")]
    Response {
        text: String,
        session_id: String,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
    },
}

/// Handle an individual WebSocket connection.
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
            Ok(_) => continue, // Ignore binary, ping, pong
            Err(e) => {
                error!(error = %e, "WebSocket receive error");
                break;
            }
        };

        let client_msg: ClientMessage = match serde_json::from_str(&msg) {
            Ok(m) => m,
            Err(e) => {
                let err = ServerMessage::Error {
                    message: format!("Invalid message format: {}", e),
                };
                let _ = sender
                    .send(WsMessage::Text(serde_json::to_string(&err).unwrap().into()))
                    .await;
                continue;
            }
        };

        match client_msg {
            ClientMessage::Message {
                text,
                user_id,
                channel_id,
            } => {
                let chat_msg = ChatMessage {
                    text,
                    user_id,
                    channel_id,
                    attachments: Vec::new(),
                };

                let response = match state.agent.chat(chat_msg).await {
                    Ok(resp) => ServerMessage::Response {
                        text: resp.text,
                        session_id: resp.session_id,
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Chat error: {}", e),
                    },
                };

                if sender
                    .send(WsMessage::Text(
                        serde_json::to_string(&response).unwrap().into(),
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}
