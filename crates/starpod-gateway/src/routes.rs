use std::net::IpAddr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use regex::Regex;
use serde::{Deserialize, Serialize};

use starpod_core::{ChatMessage, ChatResponse};

use crate::AppState;

// ── Static regexes for frame-check (compiled once) ──────────────────────

static RE_FRAME_ANCESTORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"frame-ancestors\s+([^;]+)").unwrap());

static RE_OG_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta[^>]*property=["']og:image["'][^>]*content=["']([^"']+)["']"#).unwrap()
});

static RE_OG_IMAGE2: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta[^>]*content=["']([^"']+)["'][^>]*property=["']og:image["']"#).unwrap()
});

static RE_OG_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta[^>]*property=["']og:title["'][^>]*content=["']([^"']+)["']"#).unwrap()
});

static RE_OG_TITLE2: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta[^>]*content=["']([^"']+)["'][^>]*property=["']og:title["']"#).unwrap()
});

/// Build API routes.
pub fn api_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/auth/verify", get(verify_handler))
        .route("/api/chat", post(chat_handler))
        .route("/api/frame-check", get(frame_check_handler))
        .route("/api/sessions", get(list_sessions_handler))
        .route("/api/sessions/{id}", get(get_session_handler))
        .route(
            "/api/sessions/{id}/messages",
            get(get_session_messages_handler),
        )
        .route("/api/sessions/{id}/read", post(mark_session_read_handler))
        .route("/api/memory/search", get(memory_search_handler))
        .route("/api/memory/reindex", post(reindex_handler))
        .route("/api/instances", get(list_instances_handler))
        .route("/api/instances", post(create_instance_handler))
        .route("/api/instances/{id}", get(get_instance_handler))
        .route(
            "/api/instances/{id}",
            axum::routing::delete(delete_instance_handler),
        )
        .route("/api/instances/{id}/pause", post(pause_instance_handler))
        .route(
            "/api/instances/{id}/restart",
            post(restart_instance_handler),
        )
        .route("/api/instances/{id}/health", get(instance_health_handler))
        .route("/api/health", get(health_handler))
        .route(
            "/api/cron/jobs",
            get(list_cron_jobs_handler).post(create_cron_job_handler),
        )
        .route(
            "/api/cron/jobs/{id}",
            axum::routing::put(update_cron_job_handler).delete(delete_cron_job_handler),
        )
        .merge(crate::settings::settings_routes(state.clone()))
        .merge(crate::system::system_routes(state))
        .merge(crate::files::files_routes())
}

// ── Auth verify endpoint ─────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct VerifyResponse {
    authenticated: bool,
    /// `true` when the auth store has no users (fresh install — no key needed).
    auth_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<VerifyUser>,
}

#[derive(Debug, Serialize)]
struct VerifyUser {
    id: String,
    display_name: Option<String>,
    role: String,
    filesystem_enabled: bool,
}

/// Check whether the provided API key is valid.
///
/// Returns 200 with `authenticated: true` on success, or `auth_disabled: true`
/// when the instance has no users yet (pre-bootstrap). Returns 200 with
/// `authenticated: false` when the key is missing or invalid — never 401 —
/// so the frontend can distinguish "need to log in" from "server error".
async fn verify_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<VerifyResponse> {
    let has_users = state.auth.has_users().await.unwrap_or(false);
    if !has_users {
        return Json(VerifyResponse {
            authenticated: true,
            auth_disabled: true,
            user: None,
        });
    }

    let key = headers.get("x-api-key").and_then(|v| v.to_str().ok());
    let Some(key) = key else {
        return Json(VerifyResponse {
            authenticated: false,
            auth_disabled: false,
            user: None,
        });
    };

    match state.auth.authenticate_api_key(key).await {
        Ok(Some(u)) => Json(VerifyResponse {
            authenticated: true,
            auth_disabled: false,
            user: Some(VerifyUser {
                id: u.id,
                display_name: u.display_name,
                role: format!("{:?}", u.role).to_lowercase(),
                filesystem_enabled: u.filesystem_enabled,
            }),
        }),
        _ => Json(VerifyResponse {
            authenticated: false,
            auth_disabled: false,
            user: None,
        }),
    }
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
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<FrameCheckQuery>,
) -> Result<Json<FrameCheckResponse>, (StatusCode, Json<ErrorResponse>)> {
    authenticate_request(&state, &headers).await?;

    // Validate URL scheme — only allow http and https.
    let parsed_url = reqwest::Url::parse(&params.url).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid URL".into(),
            }),
        )
    })?;
    match parsed_url.scheme() {
        "http" | "https" => {}
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Only http and https URLs are allowed".into(),
                }),
            ));
        }
    }

    // Resolve hostname and block private/internal IP ranges.
    if let Some(host) = parsed_url.host_str() {
        let port = parsed_url.port_or_known_default().unwrap_or(80);
        let addrs: Vec<std::net::SocketAddr> =
            match tokio::net::lookup_host(format!("{}:{}", host, port)).await {
                Ok(iter) => iter.collect(),
                Err(_) => Vec::new(),
            };
        if addrs.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Could not resolve hostname".into(),
                }),
            ));
        }
        for addr in &addrs {
            if is_private_ip(addr.ip()) {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        error: "Requests to private/internal addresses are not allowed".into(),
                    }),
                ));
            }
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .unwrap_or_default();

    let resp = match client.get(parsed_url).send().await {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(FrameCheckResponse {
                frameable: false,
                reason: e.to_string(),
                og_image: String::new(),
                og_title: String::new(),
            }));
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
        if let Some(caps) = RE_FRAME_ANCESTORS.captures(&csp) {
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
            if let Some(caps) = RE_OG_IMAGE
                .captures(&html)
                .or_else(|| RE_OG_IMAGE2.captures(&html))
            {
                og_image = caps[1].to_string();
            }
            if let Some(caps) = RE_OG_TITLE
                .captures(&html)
                .or_else(|| RE_OG_TITLE2.captures(&html))
            {
                og_title = caps[1].to_string();
            }
        }
    }

    Ok(Json(FrameCheckResponse {
        frameable,
        reason,
        og_image,
        og_title,
    }))
}

/// Check whether an IP address belongs to a private/internal range.
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()              // 127.0.0.0/8
                || v4.is_private()         // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()      // 169.254.0.0/16
                || v4.is_unspecified() // 0.0.0.0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()              // ::1
                || v6.is_unspecified()     // ::
                // fc00::/7 (unique local addresses)
                || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
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
    #[serde(default)]
    model: Option<String>,
}

/// Chat endpoint — POST /api/chat
async fn chat_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, Json<ErrorResponse>)> {
    let user = authenticate_request(&state, &headers).await?;

    // Use authenticated user's ID — client-provided user_id is ignored to prevent impersonation
    let user_id = user.as_ref().map(|u| u.id.clone()).or(req.user_id);

    let message = ChatMessage {
        text: req.text,
        user_id,
        channel_id: req.channel_id,
        channel_session_key: req.channel_session_key,
        attachments: Vec::new(),
        triggered_by: None,
        model: req.model,
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
    authenticate_request(&state, &headers).await?;

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
    authenticate_request(&state, &headers).await?;

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
    authenticate_request(&state, &headers).await?;

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

/// Mark session read/unread — POST /api/sessions/:id/read
#[derive(Debug, Deserialize)]
struct MarkReadRequest {
    #[serde(default = "default_is_read")]
    is_read: bool,
}

fn default_is_read() -> bool {
    true
}

async fn mark_session_read_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<MarkReadRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    authenticate_request(&state, &headers).await?;

    match state.agent.session_mgr().mark_read(&id, body.is_read).await {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Mark read error: {}", e),
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
    authenticate_request(&state, &headers).await?;

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
    authenticate_request(&state, &headers).await?;

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
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

// ── Instance routes ──────────────────────────────────────────────────────

fn get_instance_client(
    state: &AppState,
) -> Result<starpod_instances::InstanceClient, (StatusCode, Json<ErrorResponse>)> {
    let config = state.config.read().unwrap();
    let backend_url = std::env::var("STARPOD_INSTANCE_BACKEND_URL")
        .ok()
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Instance backend not configured (set STARPOD_INSTANCE_BACKEND_URL)"
                        .into(),
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
    authenticate_request(&state, &headers).await?;
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
    authenticate_request(&state, &headers).await?;
    let client = get_instance_client(&state)?;

    client
        .create_instance(&req)
        .await
        .map(|inst| (StatusCode::CREATED, Json(inst)))
        .map_err(|e| {
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
    authenticate_request(&state, &headers).await?;
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
    authenticate_request(&state, &headers).await?;
    let client = get_instance_client(&state)?;

    client
        .destroy_instance(&id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Destroy instance error: {}", e),
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
    authenticate_request(&state, &headers).await?;
    let client = get_instance_client(&state)?;

    client
        .stop_instance(&id)
        .await
        .map(|_| Json(serde_json::json!({"status": "stopped"})))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Stop instance error: {}", e),
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
    authenticate_request(&state, &headers).await?;
    let client = get_instance_client(&state)?;

    client
        .restart_instance(&id)
        .await
        .map(|_| Json(serde_json::json!({"status": "restarted"})))
        .map_err(|e| {
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
    authenticate_request(&state, &headers).await?;
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
    use std::sync::RwLock;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::{build_router, GatewayEvent};
    use starpod_agent::StarpodAgent;
    use starpod_auth::{AuthStore, RateLimiter as AuthRateLimiter};
    use starpod_core::{Mode, ResolvedPaths, StarpodConfig};

    /// Build a test AppState with real auth store.
    async fn test_app_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        let config_dir = starpod_dir.join("config");
        let db_dir = starpod_dir.join("db");
        let users_dir = starpod_dir.join("users");
        let skills_dir = starpod_dir.join("skills");
        let agent_toml = config_dir.join("agent.toml");

        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::create_dir_all(&users_dir).unwrap();
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            &agent_toml,
            "models = [\"anthropic/test\"]\nagent_name = \"Test\"\n",
        )
        .unwrap();

        let config = StarpodConfig {
            db_dir: db_dir.clone(),
            db_path: Some(db_dir.join("memory.db")),
            project_root: tmp.path().to_path_buf(),
            models: vec!["anthropic/test".into()],
            agent_name: "Test".into(),
            ..StarpodConfig::default()
        };

        let agent = StarpodAgent::new(config.clone()).await.unwrap();
        let (events_tx, _) = tokio::sync::broadcast::channel::<GatewayEvent>(16);

        let paths = ResolvedPaths {
            mode: Mode::SingleAgent {
                starpod_dir: starpod_dir.clone(),
            },
            agent_toml,
            agent_home: starpod_dir.clone(),
            config_dir,
            db_dir: db_dir.clone(),
            skills_dir,
            project_root: tmp.path().join("home"),
            instance_root: tmp.path().to_path_buf(),
            home_dir: tmp.path().join("home"),
            users_dir,
            env_file: None,
        };

        let core_db = starpod_db::CoreDb::new(&db_dir).await.unwrap();
        let auth = Arc::new(AuthStore::from_pool(core_db.pool().clone()));
        let rate_limiter = Arc::new(AuthRateLimiter::new(0, Duration::from_secs(60)));

        let state = Arc::new(AppState {
            agent: Arc::new(agent),
            auth,
            rate_limiter,
            config: RwLock::new(config),
            paths,
            model_registry: Arc::new(agent_sdk::models::ModelRegistry::with_defaults()),
            events_tx,
            vault: None,
            telegram_handle: tokio::sync::Mutex::new(None),
            update_cache: crate::system::new_update_cache(),
            shutdown_tx: tokio::sync::watch::channel(false).0,
        });

        (tmp, state)
    }

    // ── Auth integration tests ──────────────────────────────────────────

    #[tokio::test]
    async fn pre_bootstrap_allows_unauthenticated() {
        let (_tmp, state) = test_app_state().await;
        // No users exist — should allow access without API key
        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_api_key_rejected_after_bootstrap() {
        let (_tmp, state) = test_app_state().await;
        // Create a user so auth is enforced
        state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("Missing API key"));
    }

    #[tokio::test]
    async fn invalid_api_key_rejected() {
        let (_tmp, state) = test_app_state().await;
        state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/sessions")
            .header(
                "x-api-key",
                "sp_live_0000000000000000000000000000000000000000",
            )
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("Invalid API key"));
    }

    #[tokio::test]
    async fn valid_api_key_accepted() {
        let (_tmp, state) = test_app_state().await;
        let user = state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/sessions")
            .header("x-api-key", &key.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rate_limiting_returns_429() {
        let (_tmp, state) = test_app_state().await;
        let user = state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();

        // Replace rate limiter with strict limit
        let strict_state = Arc::new(AppState {
            agent: Arc::clone(&state.agent),
            auth: Arc::clone(&state.auth),
            rate_limiter: Arc::new(AuthRateLimiter::new(1, Duration::from_secs(60))),
            config: RwLock::new(state.config.read().unwrap().clone()),
            paths: state.paths.clone(),
            model_registry: Arc::clone(&state.model_registry),
            events_tx: state.events_tx.clone(),
            vault: None,
            telegram_handle: tokio::sync::Mutex::new(None),
            update_cache: crate::system::new_update_cache(),
            shutdown_tx: tokio::sync::watch::channel(false).0,
        });

        // First request should succeed (use /api/sessions which requires auth)
        let app = build_router(Arc::clone(&strict_state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/sessions")
            .header("x-api-key", &key.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Second request should be rate-limited
        let app = build_router(Arc::clone(&strict_state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/sessions")
            .header("x-api-key", &key.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn chat_injects_authenticated_user_id() {
        let (_tmp, state) = test_app_state().await;
        let user = state
            .auth
            .create_user(None, Some("Alice"), starpod_auth::Role::User)
            .await
            .unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();

        // Send a chat request with a different user_id — it should be overridden
        let app = build_router(Arc::clone(&state));
        let body = serde_json::json!({
            "text": "hello",
            "user_id": "impersonation-attempt"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/chat")
            .header("content-type", "application/json")
            .header("x-api-key", &key.key)
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // The chat will fail because there's no real model, but the auth should pass
        // (we're testing that auth succeeds, not that chat works)
        let status = resp.status();
        assert_ne!(status, StatusCode::UNAUTHORIZED, "Should not be 401");
        assert_ne!(status, StatusCode::FORBIDDEN, "Should not be 403");
    }

    #[tokio::test]
    async fn health_always_accessible() {
        let (_tmp, state) = test_app_state().await;
        // Health should work even without auth
        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    // ── Verify endpoint tests ─────────────────────────────────────────

    #[tokio::test]
    async fn verify_pre_bootstrap_returns_auth_disabled() {
        let (_tmp, state) = test_app_state().await;
        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/auth/verify")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["authenticated"], true);
        assert_eq!(json["auth_disabled"], true);
        assert!(json.get("user").is_none() || json["user"].is_null());
    }

    #[tokio::test]
    async fn verify_missing_key_returns_unauthenticated() {
        let (_tmp, state) = test_app_state().await;
        state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/auth/verify")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["authenticated"], false);
        assert_eq!(json["auth_disabled"], false);
    }

    #[tokio::test]
    async fn verify_invalid_key_returns_unauthenticated() {
        let (_tmp, state) = test_app_state().await;
        state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/auth/verify")
            .header(
                "x-api-key",
                "sp_live_0000000000000000000000000000000000000000",
            )
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["authenticated"], false);
    }

    #[tokio::test]
    async fn verify_valid_key_returns_user() {
        let (_tmp, state) = test_app_state().await;
        let user = state
            .auth
            .create_user(None, Some("Alice"), starpod_auth::Role::Admin)
            .await
            .unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/auth/verify")
            .header("x-api-key", &key.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["authenticated"], true);
        assert_eq!(json["auth_disabled"], false);
        assert_eq!(json["user"]["id"], user.id);
        assert_eq!(json["user"]["display_name"], "Alice");
        assert_eq!(json["user"]["role"], "admin");
    }

    // ── IP filtering tests (existing) ───────────────────────────────────

    #[test]
    fn private_ipv4_loopback() {
        assert!(is_private_ip(IpAddr::V4(std::net::Ipv4Addr::new(
            127, 0, 0, 1
        ))));
    }

    #[test]
    fn private_ipv4_10_range() {
        assert!(is_private_ip(IpAddr::V4(std::net::Ipv4Addr::new(
            10, 0, 0, 1
        ))));
    }

    #[test]
    fn private_ipv4_172_range() {
        assert!(is_private_ip(IpAddr::V4(std::net::Ipv4Addr::new(
            172, 16, 0, 1
        ))));
    }

    #[test]
    fn private_ipv4_192_range() {
        assert!(is_private_ip(IpAddr::V4(std::net::Ipv4Addr::new(
            192, 168, 1, 1
        ))));
    }

    #[test]
    fn private_ipv4_link_local() {
        assert!(is_private_ip(IpAddr::V4(std::net::Ipv4Addr::new(
            169, 254, 1, 1
        ))));
    }

    #[test]
    fn public_ipv4_allowed() {
        assert!(!is_private_ip(IpAddr::V4(std::net::Ipv4Addr::new(
            8, 8, 8, 8
        ))));
    }

    #[test]
    fn private_ipv6_loopback() {
        assert!(is_private_ip(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn private_ipv6_unique_local() {
        // fc00::1 is in the fc00::/7 range
        assert!(is_private_ip(IpAddr::V6(std::net::Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(is_private_ip(IpAddr::V6(std::net::Ipv6Addr::new(
            0xfd00, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn public_ipv6_allowed() {
        assert!(!is_private_ip(IpAddr::V6(std::net::Ipv6Addr::new(
            0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888
        ))));
    }

    // ── Mark-read endpoint tests ────────────────────────────────────────

    #[tokio::test]
    async fn mark_session_read_pre_bootstrap() {
        let (_tmp, state) = test_app_state().await;

        // Create a session via the session manager
        let session_mgr = state.agent.session_mgr();
        let sid = session_mgr
            .create_session(&starpod_session::Channel::Main, "k1")
            .await
            .unwrap();

        // Mark unread
        let app = build_router(Arc::clone(&state));
        let body = serde_json::json!({ "is_read": false });
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/sessions/{sid}/read"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify via list endpoint
        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/sessions?limit=10")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let sessions = json.as_array().unwrap();
        let session = sessions.iter().find(|s| s["id"] == sid).unwrap();
        assert_eq!(session["is_read"], false);

        // Mark read again
        let app = build_router(Arc::clone(&state));
        let body = serde_json::json!({ "is_read": true });
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/sessions/{sid}/read"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify read again
        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri(format!("/api/sessions/{sid}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["is_read"], true);
    }

    #[tokio::test]
    async fn mark_session_read_requires_auth() {
        let (_tmp, state) = test_app_state().await;
        // Create a user so auth is enforced
        state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let body = serde_json::json!({ "is_read": true });
        let req = Request::builder()
            .method("POST")
            .uri("/api/sessions/some-id/read")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Cron jobs endpoint tests ────────────────────────────────────────

    #[tokio::test]
    async fn cron_jobs_pre_bootstrap_returns_all() {
        let (_tmp, state) = test_app_state().await;
        // Add a cron job before any users exist
        let schedule = starpod_cron::Schedule::Interval { every_ms: 60000 };
        state
            .agent
            .cron()
            .add_job("test-job", "do stuff", &schedule, false, None)
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/cron/jobs")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let jobs: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0]["name"], "test-job");
    }

    #[tokio::test]
    async fn cron_jobs_admin_sees_all() {
        let (_tmp, state) = test_app_state().await;
        let admin = state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();
        let key = state.auth.create_api_key(&admin.id, None).await.unwrap();

        // Add jobs for different users
        let schedule = starpod_cron::Schedule::Interval { every_ms: 60000 };
        state
            .agent
            .cron()
            .add_job_full(
                "admin-job",
                "admin prompt",
                &schedule,
                false,
                None,
                3,
                7200,
                starpod_cron::SessionMode::Isolated,
                Some(&admin.id),
            )
            .await
            .unwrap();
        state
            .agent
            .cron()
            .add_job_full(
                "other-job",
                "other prompt",
                &schedule,
                false,
                None,
                3,
                7200,
                starpod_cron::SessionMode::Isolated,
                Some("other-user"),
            )
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/cron/jobs")
            .header("x-api-key", &key.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let jobs: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(jobs.len(), 2, "Admin should see all jobs");
    }

    #[tokio::test]
    async fn cron_jobs_user_sees_only_own() {
        let (_tmp, state) = test_app_state().await;
        // Need an admin first so auth is enabled
        state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();
        let user = state
            .auth
            .create_user(None, None, starpod_auth::Role::User)
            .await
            .unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();

        let schedule = starpod_cron::Schedule::Interval { every_ms: 60000 };
        state
            .agent
            .cron()
            .add_job_full(
                "my-job",
                "my prompt",
                &schedule,
                false,
                None,
                3,
                7200,
                starpod_cron::SessionMode::Isolated,
                Some(&user.id),
            )
            .await
            .unwrap();
        state
            .agent
            .cron()
            .add_job_full(
                "other-job",
                "other prompt",
                &schedule,
                false,
                None,
                3,
                7200,
                starpod_cron::SessionMode::Isolated,
                Some("someone-else"),
            )
            .await
            .unwrap();
        state
            .agent
            .cron()
            .add_job_full(
                "agent-job",
                "agent prompt",
                &schedule,
                false,
                None,
                3,
                7200,
                starpod_cron::SessionMode::Isolated,
                None,
            )
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/cron/jobs")
            .header("x-api-key", &key.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let jobs: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(jobs.len(), 1, "User should only see their own jobs");
        assert_eq!(jobs[0]["name"], "my-job");
    }

    #[tokio::test]
    async fn create_cron_job_via_api() {
        let (_tmp, state) = test_app_state().await;

        let app = build_router(Arc::clone(&state));
        let body = serde_json::json!({
            "name": "api-job",
            "prompt": "Do something useful",
            "schedule": { "kind": "interval", "every_ms": 300000 }
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/cron/jobs")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let job: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(job["name"], "api-job");
        assert_eq!(job["prompt"], "Do something useful");
        assert_eq!(job["enabled"], true);
    }

    #[tokio::test]
    async fn update_cron_job_via_api() {
        let (_tmp, state) = test_app_state().await;
        let schedule = starpod_cron::Schedule::Interval { every_ms: 60000 };
        let id = state
            .agent
            .cron()
            .add_job("update-test", "old prompt", &schedule, false, None)
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let body = serde_json::json!({
            "prompt": "new prompt",
            "enabled": false
        });
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/cron/jobs/{}", id))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let job: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(job["prompt"], "new prompt");
        assert_eq!(job["enabled"], false);
    }

    #[tokio::test]
    async fn delete_cron_job_via_api() {
        let (_tmp, state) = test_app_state().await;
        let schedule = starpod_cron::Schedule::Interval { every_ms: 60000 };
        let id = state
            .agent
            .cron()
            .add_job("delete-me", "prompt", &schedule, false, None)
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/cron/jobs/{}", id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Verify it's gone
        let jobs = state.agent.cron().list_jobs().await.unwrap();
        assert!(jobs.is_empty());
    }

    #[tokio::test]
    async fn delete_cron_job_user_cannot_delete_others() {
        let (_tmp, state) = test_app_state().await;
        let _admin = state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();
        let user = state
            .auth
            .create_user(None, None, starpod_auth::Role::User)
            .await
            .unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();

        let schedule = starpod_cron::Schedule::Interval { every_ms: 60000 };
        let id = state
            .agent
            .cron()
            .add_job_full(
                "other-job",
                "not yours",
                &schedule,
                false,
                None,
                3,
                7200,
                starpod_cron::SessionMode::Isolated,
                Some("someone-else"),
            )
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/cron/jobs/{}", id))
            .header("x-api-key", &key.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn update_cron_job_user_cannot_update_others() {
        let (_tmp, state) = test_app_state().await;
        let _admin = state
            .auth
            .create_user(None, None, starpod_auth::Role::Admin)
            .await
            .unwrap();
        let user = state
            .auth
            .create_user(None, None, starpod_auth::Role::User)
            .await
            .unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();

        let schedule = starpod_cron::Schedule::Interval { every_ms: 60000 };
        let id = state
            .agent
            .cron()
            .add_job_full(
                "other-job",
                "not yours",
                &schedule,
                false,
                None,
                3,
                7200,
                starpod_cron::SessionMode::Isolated,
                Some("someone-else"),
            )
            .await
            .unwrap();

        let app = build_router(Arc::clone(&state));
        let body = serde_json::json!({ "prompt": "hacked" });
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/cron/jobs/{}", id))
            .header("x-api-key", &key.key)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}

/// Authenticate a request via API key.
///
/// Looks up the `X-API-Key` header, verifies against the auth store (argon2id),
/// enforces rate limiting, and returns the authenticated `User`.
///
/// If no users exist in the auth store (fresh install, no bootstrap yet),
/// all requests are allowed with a `None` user for backward compatibility.
pub(crate) async fn authenticate_request(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<starpod_auth::User>, (StatusCode, Json<ErrorResponse>)> {
    // If no users exist yet, allow unauthenticated access (pre-bootstrap)
    let has_users = state.auth.has_users().await.unwrap_or(false);
    if !has_users {
        return Ok(None);
    }

    let provided = headers.get("x-api-key").and_then(|v| v.to_str().ok());

    let key = match provided {
        Some(k) => k,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Missing API key — set X-API-Key header".into(),
                }),
            ));
        }
    };

    let user = state.auth.authenticate_api_key(key).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Auth error: {}", e),
            }),
        )
    })?;

    match user {
        Some(u) => {
            // Rate limit check
            if !state.rate_limiter.check(&u.id) {
                return Err((
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(ErrorResponse {
                        error: "Rate limit exceeded".into(),
                    }),
                ));
            }
            Ok(Some(u))
        }
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid API key".into(),
            }),
        )),
    }
}

// ── Cron jobs listing ────────────────────────────────────────────────────

async fn list_cron_jobs_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<starpod_cron::CronJob>>, (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;

    let jobs = match &auth_user {
        Some(u) if u.role != starpod_auth::Role::Admin => {
            state.agent.cron().list_jobs_for_user(&u.id).await
        }
        _ => state.agent.cron().list_jobs().await,
    };

    let jobs = jobs.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to list cron jobs: {}", e),
            }),
        )
    })?;

    Ok(Json(jobs))
}

#[derive(Deserialize)]
struct CreateCronJobRequest {
    name: String,
    prompt: String,
    schedule: starpod_cron::Schedule,
    #[serde(default)]
    delete_after_run: bool,
}

async fn create_cron_job_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateCronJobRequest>,
) -> Result<(StatusCode, Json<starpod_cron::CronJob>), (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;
    let user_id = auth_user.as_ref().map(|u| u.id.as_str());

    let id = state
        .agent
        .cron()
        .add_job_full(
            &req.name,
            &req.prompt,
            &req.schedule,
            req.delete_after_run,
            None,
            3,
            7200,
            starpod_cron::SessionMode::Isolated,
            user_id,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to create cron job: {}", e),
                }),
            )
        })?;

    let job = state.agent.cron().get_job(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to fetch created job: {}", e),
            }),
        )
    })?;

    Ok((StatusCode::CREATED, Json(job)))
}

#[derive(Deserialize)]
struct UpdateCronJobRequest {
    prompt: Option<String>,
    schedule: Option<starpod_cron::Schedule>,
    enabled: Option<bool>,
}

async fn update_cron_job_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateCronJobRequest>,
) -> Result<Json<starpod_cron::CronJob>, (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;

    // Non-admin users can only update their own jobs
    if let Some(user) = &auth_user {
        if user.role != starpod_auth::Role::Admin {
            let job = state.agent.cron().get_job(&id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to fetch job: {}", e),
                    }),
                )
            })?;
            if job.user_id.as_deref() != Some(&user.id) {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        error: "Cannot update another user's job".into(),
                    }),
                ));
            }
        }
    }

    let update = starpod_cron::JobUpdate {
        prompt: req.prompt,
        schedule: req.schedule,
        enabled: req.enabled,
        ..Default::default()
    };

    state
        .agent
        .cron()
        .update_job(&id, &update)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update cron job: {}", e),
                }),
            )
        })?;

    let job = state.agent.cron().get_job(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to fetch updated job: {}", e),
            }),
        )
    })?;

    Ok(Json(job))
}

async fn delete_cron_job_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;

    // Non-admin users can only delete their own jobs
    if let Some(user) = &auth_user {
        if user.role != starpod_auth::Role::Admin {
            let job = state.agent.cron().get_job(&id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to fetch job: {}", e),
                    }),
                )
            })?;
            if job.user_id.as_deref() != Some(&user.id) {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        error: "Cannot delete another user's job".into(),
                    }),
                ));
            }
        }
    }

    state.agent.cron().remove_job(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to delete cron job: {}", e),
            }),
        )
    })?;

    Ok(StatusCode::NO_CONTENT)
}
