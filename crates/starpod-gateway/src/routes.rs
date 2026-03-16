use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use regex::Regex;
use serde::{Deserialize, Serialize};

use starpod_core::{ChatMessage, ChatResponse};

use crate::AppState;

/// Build API routes.
pub fn api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/chat", post(chat_handler))
        .route("/api/frame-check", get(frame_check_handler))
        .route("/api/sessions", get(list_sessions_handler))
        .route("/api/sessions/{id}", get(get_session_handler))
        .route("/api/sessions/{id}/messages", get(get_session_messages_handler))
        .route("/api/memory/search", get(memory_search_handler))
        .route("/api/memory/reindex", post(reindex_handler))
        .route("/api/instances", get(list_instances_handler))
        .route("/api/instances", post(create_instance_handler))
        .route("/api/instances/{id}", get(get_instance_handler))
        .route("/api/instances/{id}", axum::routing::delete(delete_instance_handler))
        .route("/api/instances/{id}/pause", post(pause_instance_handler))
        .route("/api/instances/{id}/restart", post(restart_instance_handler))
        .route("/api/instances/{id}/health", get(instance_health_handler))
        .route("/api/health", get(health_handler))
}

// ── Frame-check endpoint ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FrameCheckQuery {
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FrameCheckResponse {
    frameable: bool,
    reason: String,
    #[serde(rename = "ogImage")]
    og_image: String,
    #[serde(rename = "ogTitle")]
    og_title: String,
}

async fn frame_check_handler(
    Query(params): Query<FrameCheckQuery>,
) -> Json<FrameCheckResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .unwrap_or_default();

    let resp = match client.get(&params.url).send().await {
        Ok(r) => r,
        Err(e) => {
            return Json(FrameCheckResponse {
                frameable: false,
                reason: e.to_string(),
                og_image: String::new(),
                og_title: String::new(),
            });
        }
    };

    let xfo = resp
        .headers()
        .get("x-frame-options")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let mut frameable = true;
    let mut reason = String::new();

    if xfo == "deny" || xfo == "sameorigin" {
        frameable = false;
        reason = format!("X-Frame-Options: {}", xfo);
    }

    if csp.contains("frame-ancestors") {
        if let Some(caps) = Regex::new(r"frame-ancestors\s+([^;]+)")
            .unwrap()
            .captures(&csp)
        {
            let val = caps[1].trim();
            if !val.contains('*') {
                frameable = false;
                reason = format!("CSP frame-ancestors: {}", val);
            }
        }
    }

    let mut og_image = String::new();
    let mut og_title = String::new();

    if !frameable {
        if let Ok(html) = resp.text().await {
            let img_re = Regex::new(
                r#"<meta[^>]*property=["']og:image["'][^>]*content=["']([^"']+)["']"#,
            )
            .unwrap();
            let img_re2 = Regex::new(
                r#"<meta[^>]*content=["']([^"']+)["'][^>]*property=["']og:image["']"#,
            )
            .unwrap();
            if let Some(caps) = img_re.captures(&html).or_else(|| img_re2.captures(&html)) {
                og_image = caps[1].to_string();
            }

            let title_re = Regex::new(
                r#"<meta[^>]*property=["']og:title["'][^>]*content=["']([^"']+)["']"#,
            )
            .unwrap();
            let title_re2 = Regex::new(
                r#"<meta[^>]*content=["']([^"']+)["'][^>]*property=["']og:title["']"#,
            )
            .unwrap();
            if let Some(caps) = title_re
                .captures(&html)
                .or_else(|| title_re2.captures(&html))
            {
                og_title = caps[1].to_string();
            }
        }
    }

    Json(FrameCheckResponse {
        frameable,
        reason,
        og_image,
        og_title,
    })
}

// ── Chat endpoint ────────────────────────────────────────────────────────

/// Request body for chat endpoint.
#[derive(Debug, Deserialize)]
struct ChatRequest {
    text: String,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    channel_session_key: Option<String>,
}

/// Chat endpoint — POST /api/chat
async fn chat_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;

    let message = ChatMessage {
        text: req.text,
        user_id: req.user_id,
        channel_id: req.channel_id,
        channel_session_key: req.channel_session_key,
        attachments: Vec::new(),
    };

    match state.agent.chat(message).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Chat error: {}", e),
            }),
        )),
    }
}

/// Query params for session list.
#[derive(Debug, Deserialize)]
struct ListSessionsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

/// List sessions — GET /api/sessions
async fn list_sessions_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<ListSessionsQuery>,
) -> Result<Json<Vec<starpod_session::SessionMeta>>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;

    match state.agent.session_mgr().list_sessions(params.limit).await {
        Ok(sessions) => Ok(Json(sessions)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Session list error: {}", e),
            }),
        )),
    }
}

/// Get session — GET /api/sessions/:id
async fn get_session_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Option<starpod_session::SessionMeta>>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;

    match state.agent.session_mgr().get_session(&id).await {
        Ok(session) => Ok(Json(session)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Session get error: {}", e),
            }),
        )),
    }
}

/// Get session messages — GET /api/sessions/:id/messages
async fn get_session_messages_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<starpod_session::SessionMessage>>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;

    match state.agent.session_mgr().get_messages(&id).await {
        Ok(messages) => Ok(Json(messages)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Get messages error: {}", e),
            }),
        )),
    }
}

/// Query params for memory search.
#[derive(Debug, Deserialize)]
struct MemorySearchQuery {
    q: String,
    #[serde(default = "default_search_limit")]
    limit: usize,
}

fn default_search_limit() -> usize {
    10
}

/// Search result for API response.
#[derive(Debug, Serialize)]
struct SearchResultResponse {
    source: String,
    text: String,
    line_start: usize,
    line_end: usize,
}

/// Memory search — GET /api/memory/search?q=...
async fn memory_search_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<MemorySearchQuery>,
) -> Result<Json<Vec<SearchResultResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;

    match state.agent.memory().search(&params.q, params.limit).await {
        Ok(results) => {
            let response: Vec<SearchResultResponse> = results
                .into_iter()
                .map(|r| SearchResultResponse {
                    source: r.source,
                    text: r.text,
                    line_start: r.line_start,
                    line_end: r.line_end,
                })
                .collect();
            Ok(Json(response))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Search error: {}", e),
            }),
        )),
    }
}

/// Reindex memory — POST /api/memory/reindex
async fn reindex_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;

    match state.agent.memory().reindex().await {
        Ok(()) => Ok(Json(serde_json::json!({"status": "ok"}))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Reindex error: {}", e),
            }),
        )),
    }
}

/// Health check — GET /api/health
async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))
}

/// Error response body.
#[derive(Debug, Serialize, Deserialize)]
struct ErrorResponse {
    error: String,
}

// ── Instance routes ──────────────────────────────────────────────────────

fn get_instance_client(state: &AppState) -> Result<starpod_instances::InstanceClient, (StatusCode, Json<ErrorResponse>)> {
    let config = state.config.read().unwrap();
    let backend_url = std::env::var("STARPOD_INSTANCE_BACKEND_URL").ok().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Instance backend not configured (set STARPOD_INSTANCE_BACKEND_URL)".into(),
            }),
        )
    })?;
    let api_key = config.resolved_api_key();
    starpod_instances::InstanceClient::new(&backend_url, api_key).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Instance client error: {}", e),
            }),
        )
    })
}

/// List instances — GET /api/instances
async fn list_instances_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<starpod_instances::Instance>>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;
    let client = get_instance_client(&state)?;

    client.list_instances().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("List instances error: {}", e),
            }),
        )
    })
}

/// Create instance — POST /api/instances
async fn create_instance_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<starpod_instances::CreateInstanceRequest>,
) -> Result<(StatusCode, Json<starpod_instances::Instance>), (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;
    let client = get_instance_client(&state)?;

    client.create_instance(&req).await.map(|inst| (StatusCode::CREATED, Json(inst))).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Create instance error: {}", e),
            }),
        )
    })
}

/// Get instance — GET /api/instances/:id
async fn get_instance_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<starpod_instances::Instance>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;
    let client = get_instance_client(&state)?;

    client.get_instance(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Get instance error: {}", e),
            }),
        )
    })
}

/// Delete (kill) instance — DELETE /api/instances/:id
async fn delete_instance_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;
    let client = get_instance_client(&state)?;

    client.kill_instance(&id).await.map(|_| StatusCode::NO_CONTENT).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Kill instance error: {}", e),
            }),
        )
    })
}

/// Pause instance — POST /api/instances/:id/pause
async fn pause_instance_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;
    let client = get_instance_client(&state)?;

    client.pause_instance(&id).await.map(|_| Json(serde_json::json!({"status": "paused"}))).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Pause instance error: {}", e),
            }),
        )
    })
}

/// Restart instance — POST /api/instances/:id/restart
async fn restart_instance_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;
    let client = get_instance_client(&state)?;

    client.restart_instance(&id).await.map(|_| Json(serde_json::json!({"status": "restarted"}))).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Restart instance error: {}", e),
            }),
        )
    })
}

/// Instance health — GET /api/instances/:id/health
async fn instance_health_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<starpod_instances::HealthInfo>, (StatusCode, Json<ErrorResponse>)> {
    check_api_key(&state, &headers)?;
    let client = get_instance_client(&state)?;

    client.get_health(&id).await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Instance health error: {}", e),
            }),
        )
    })
}

// ── Frame-check unit tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn frame_check_router() -> Router {
        Router::new().route("/api/frame-check", get(frame_check_handler))
    }

    async fn get_frame_check(url: &str) -> FrameCheckResponse {
        let app = frame_check_router();
        let uri = format!("/api/frame-check?url={}", urlencoding::encode(url));
        let req = Request::builder()
            .uri(&uri)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn frameable_site() {
        // httpbin allows framing (no X-Frame-Options)
        let r = get_frame_check("https://httpbin.org/html").await;
        assert!(r.frameable, "httpbin should be frameable");
    }

    #[tokio::test]
    async fn non_frameable_site_with_xfo() {
        // GitHub sets X-Frame-Options: deny
        let r = get_frame_check("https://github.com").await;
        assert!(!r.frameable, "github.com should not be frameable");
        assert!(
            r.reason.to_lowercase().contains("x-frame-options")
                || r.reason.to_lowercase().contains("frame-ancestors"),
            "reason should mention header: {}",
            r.reason
        );
    }

    #[tokio::test]
    async fn non_frameable_extracts_og() {
        // YouTube blocks framing and has og:image + og:title
        let r = get_frame_check("https://www.youtube.com").await;
        assert!(!r.frameable);
        assert!(!r.og_title.is_empty(), "should extract og:title from YouTube");
    }

    #[tokio::test]
    async fn unreachable_url_returns_not_frameable() {
        let r = get_frame_check("http://localhost:1").await;
        assert!(!r.frameable);
        assert!(!r.reason.is_empty());
    }

    #[tokio::test]
    async fn missing_url_param_returns_error() {
        let app = frame_check_router();
        let req = Request::builder()
            .uri("/api/frame-check")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

/// Check X-API-Key header if an API key is configured.
fn check_api_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if let Some(expected) = &state.api_key {
        let provided = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok());

        match provided {
            Some(key) if key == expected => Ok(()),
            _ => Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Invalid or missing API key".into(),
                }),
            )),
        }
    } else {
        Ok(()) // No API key configured — allow all requests
    }
}
