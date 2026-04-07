mod files;
mod routes;
mod settings;
mod system;
mod ws;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use include_dir::{include_dir, Dir};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use rust_embed::Embed;
use tower_http::cors::CorsLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};

use agent_sdk::models::ModelRegistry;
use agent_sdk::OllamaDiscovery;
use starpod_agent::StarpodAgent;
use starpod_auth::{AuthStore, RateLimiter};
use starpod_core::{reload_agent_config, ResolvedPaths, StarpodConfig};

/// Event broadcast to connected WebSocket clients via a `tokio::sync::broadcast` channel.
///
/// When a cron job or heartbeat completes, the gateway pushes a `CronComplete` event
/// to all connected WS clients. Each client forwards it as a `ServerMessage::Notification`
/// so the web UI can show a toast and update the session list in real time.
///
/// The broadcast channel is created in [`serve_with_agent`] and stored in [`AppState`].
/// The composed notifier writes to the channel before forwarding to the Telegram notifier.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum GatewayEvent {
    /// A cron job or heartbeat completed (success or failure).
    ///
    /// - `session_id` is empty when the job failed before creating a session.
    /// - `result_preview` is truncated to 500 characters by the executor.
    #[serde(rename = "cron_complete")]
    CronComplete {
        job_name: String,
        session_id: String,
        result_preview: String,
        success: bool,
    },
}

/// Shared application state.
///
/// Config is wrapped in `RwLock` for hot reload support.
pub struct AppState {
    pub agent: Arc<StarpodAgent>,
    pub auth: Arc<AuthStore>,
    pub rate_limiter: Arc<RateLimiter>,
    pub config: RwLock<StarpodConfig>,
    pub paths: ResolvedPaths,
    /// Centralized model catalog (pricing + capabilities + provider metadata).
    pub model_registry: Arc<ModelRegistry>,
    /// Broadcast channel for pushing events to connected WebSocket clients.
    pub events_tx: tokio::sync::broadcast::Sender<GatewayEvent>,
    /// Encrypted credential vault for system keys (API keys, bot tokens).
    pub vault: Option<Arc<starpod_vault::Vault>>,
    /// Handle to the running Telegram bot task (if any).
    pub telegram_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Handle to the running Slack bot task (if any).
    pub slack_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Cached latest-release info for version checks (`GET /api/system/version`).
    /// Populated on first request and refreshed after 1 hour.
    pub update_cache: system::UpdateCache,
    /// Sender to signal graceful shutdown. The self-update handler sends `true`
    /// after spawning the new binary, causing `axum::serve` to drain connections
    /// and exit cleanly.
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl AppState {
    /// (Re)start the Telegram bot, aborting any previously running instance.
    ///
    /// Reads the token from the `TELEGRAM_BOT_TOKEN` env var and the `enabled`
    /// flag from the current config. If both are set, spawns a new bot task.
    ///
    /// Called in three situations:
    /// 1. On gateway startup (initial boot).
    /// 2. When the admin saves channel settings via the Settings UI.
    /// 3. When the config file watcher detects a change to `[channels.telegram]`.
    pub async fn restart_telegram(&self) {
        // Abort existing bot task
        let mut handle = self.telegram_handle.lock().await;
        if let Some(h) = handle.take() {
            h.abort();
            info!("Telegram bot stopped");
        }

        let config = self.config.read().unwrap().clone();
        let enabled = config.channels.telegram.as_ref().is_some_and(|t| t.enabled);
        let token = config.resolved_telegram_token();

        match (enabled, token) {
            (true, Some(token)) => {
                let agent = Arc::clone(&self.agent);
                let auth = Arc::clone(&self.auth);
                let h = tokio::spawn(async move {
                    if let Err(e) =
                        starpod_telegram::run_with_agent_and_auth(agent, auth, token).await
                    {
                        tracing::error!(error = %e, "Telegram bot error");
                    }
                });
                info!("Telegram bot started");
                *handle = Some(h);
            }
            (true, None) => {
                warn!("Telegram channel enabled but TELEGRAM_BOT_TOKEN is not set");
            }
            (false, _) => {
                debug!("Telegram channel disabled, bot not started");
            }
        }
    }

    /// (Re)start the Slack bot, aborting any previously running instance.
    ///
    /// Reads `SLACK_APP_TOKEN` (Socket Mode app-level token) and
    /// `SLACK_BOT_TOKEN` (bot user OAuth token) from env vars, and the
    /// `enabled` flag from `[channels.slack]` in the live config. Both
    /// tokens are required — with only one the bot is inert.
    ///
    /// Called in three situations:
    /// 1. On gateway startup (initial boot).
    /// 2. When the admin saves channel settings via the Settings UI.
    /// 3. When the config file watcher detects a change to `[channels.slack]`.
    pub async fn restart_slack(&self) {
        let mut handle = self.slack_handle.lock().await;
        if let Some(h) = handle.take() {
            h.abort();
            info!("Slack bot stopped");
        }

        let config = self.config.read().unwrap().clone();
        let enabled = config.channels.slack.as_ref().is_some_and(|s| s.enabled);
        let app_token = config.resolved_slack_app_token();
        let bot_token = config.resolved_slack_bot_token();

        match (enabled, app_token, bot_token) {
            (true, Some(app), Some(bot)) => {
                let agent = Arc::clone(&self.agent);
                let auth = Arc::clone(&self.auth);
                let h = tokio::spawn(async move {
                    if let Err(e) =
                        starpod_slack::run_with_agent_and_auth(agent, auth, app, bot).await
                    {
                        tracing::error!(error = %e, "Slack bot error");
                    }
                });
                info!("Slack bot started");
                *handle = Some(h);
            }
            (true, None, _) => {
                warn!("Slack channel enabled but SLACK_APP_TOKEN is not set");
            }
            (true, _, None) => {
                warn!("Slack channel enabled but SLACK_BOT_TOKEN is not set");
            }
            (false, _, _) => {
                debug!("Slack channel disabled, bot not started");
            }
        }
    }
}

/// Embedded web UI assets (built by Vite into static/dist/).
#[derive(Embed)]
#[folder = "static/dist/"]
struct Asset;

/// Embedded documentation site (built by VitePress into docs/.vitepress/dist/).
static DOCS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../docs/.vitepress/dist");

/// Serve embedded static files, falling back to index.html for SPA routing.
/// When serving index.html, injects welcome config as `window.__STARPOD__`.
async fn static_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    uri: Uri,
) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try the exact path first
    if !path.is_empty() {
        if let Some(file) = Asset::get(path) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            return Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(file.data.to_vec()))
                .unwrap();
        }
    }

    // Fallback to index.html (SPA) — inject welcome config
    match Asset::get("index.html") {
        Some(file) => {
            let html = String::from_utf8_lossy(&file.data);
            let starpod_config = state.config.read().unwrap();
            let html = inject_frontend_config(&html, &state.paths.config_dir, &starpod_config);
            Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(html))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(
                "Web UI not found. Run `npm run build` in web/ first.",
            ))
            .unwrap(),
    }
}

/// Read frontend.toml + starpod config and inject as `window.__STARPOD__` into the HTML.
fn inject_frontend_config(
    html: &str,
    config_dir: &std::path::Path,
    starpod_config: &starpod_core::StarpodConfig,
) -> String {
    let frontend = starpod_core::FrontendConfig::load(config_dir);
    let oauth_proxy_url = std::env::var("OAUTH_PROXY_URL")
        .or_else(|_| std::env::var("STARPOD_URL"))
        .unwrap_or_else(|_| "https://console.starpod.sh".to_string());
    let oauth_proxy_url = Some(oauth_proxy_url);
    let merged = serde_json::json!({
        "greeting": frontend.greeting,
        "prompts": frontend.prompts,
        "models": starpod_config.models,
        "agent_name": starpod_config.agent_name,
        "oauth_proxy_url": oauth_proxy_url,
    });
    let json = serde_json::to_string(&merged).unwrap_or_else(|_| "{}".to_string());
    let script = format!("<script>window.__STARPOD__={}</script>", json);
    html.replace("</head>", &format!("{}\n</head>", script))
}

/// Serve embedded docs under /docs, handling VitePress clean URLs.
async fn docs_handler(uri: Uri) -> Response {
    let path = uri
        .path()
        .trim_start_matches("/docs")
        .trim_start_matches('/');

    fn mime_from_path(path: &str) -> &'static str {
        match path.rsplit('.').next() {
            Some("html") => "text/html; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("js") => "application/javascript; charset=utf-8",
            Some("json") => "application/json",
            Some("svg") => "image/svg+xml",
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("woff2") => "font/woff2",
            Some("woff") => "font/woff",
            _ => "application/octet-stream",
        }
    }

    let file_path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = DOCS_DIR.get_file(file_path) {
        let mime = mime_from_path(file_path);
        ([(header::CONTENT_TYPE, mime)], file.contents()).into_response()
    } else if let Some(file) = DOCS_DIR.get_file(format!("{}.html", file_path)) {
        let mime = mime_from_path(&format!("{}.html", file_path));
        ([(header::CONTENT_TYPE, mime)], file.contents()).into_response()
    } else if let Some(file) = DOCS_DIR.get_file(format!("{}/index.html", file_path)) {
        (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            file.contents(),
        )
            .into_response()
    } else if let Some(file) = DOCS_DIR.get_file("404.html") {
        (
            StatusCode::NOT_FOUND,
            Html(String::from_utf8_lossy(file.contents()).to_string()),
        )
            .into_response()
    } else {
        (StatusCode::NOT_FOUND, "Not found").into_response()
    }
}

/// Build the Axum router with all routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    let api = routes::api_routes(Arc::clone(&state)).layer(SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    ));

    Router::new()
        .merge(api)
        .merge(ws::ws_routes())
        .route("/docs", get(docs_handler))
        .route("/docs/", get(docs_handler))
        .route("/docs/{*path}", get(docs_handler))
        .fallback(get(static_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the gateway server with a pre-built agent.
///
/// Takes ownership of the agent, config, and resolved paths. Starts the cron
/// scheduler, runs lifecycle prompts (BOOTSTRAP/BOOT), sets up the config file
/// watcher for hot reload, and binds the Axum HTTP server.
///
/// The `paths` parameter determines which config files are watched for hot reload:
/// - **Workspace**: watches both `starpod.toml` and `agents/<name>/agent.toml`
/// - **SingleAgent**: watches `.starpod/agent.toml`
///
/// Create and bootstrap an `AuthStore` for the given paths.
///
/// This is the canonical way to create an auth store. Both the gateway and the
/// CLI should use this to ensure the same DB path and bootstrap logic.
/// Bootstrap result returned by [`create_auth_store`].
pub struct AuthBootstrap {
    pub store: Arc<AuthStore>,
    /// The admin API key plaintext, if a new admin was just bootstrapped.
    pub admin_key: Option<String>,
}

pub async fn create_auth_store(paths: &ResolvedPaths) -> starpod_core::Result<AuthBootstrap> {
    let core_db = starpod_db::CoreDb::new(&paths.db_dir).await?;
    let auth = Arc::new(AuthStore::from_pool(core_db.pool().clone()));
    let admin_key = bootstrap_auth(&auth).await?;
    Ok(AuthBootstrap {
        store: auth,
        admin_key,
    })
}

/// Bootstrap the admin user and return the API key if newly created.
async fn bootstrap_auth(auth: &Arc<AuthStore>) -> starpod_core::Result<Option<String>> {
    let existing_api_key = std::env::var("STARPOD_API_KEY").ok();
    if let Some((admin, key)) = auth.bootstrap_admin(existing_api_key.as_deref()).await? {
        info!(user_id = %admin.id, "Admin user bootstrapped");
        if existing_api_key.is_none() {
            info!(api_key = %key, "New admin API key generated — save this!");
        }
        Ok(Some(key))
    } else {
        Ok(None)
    }
}

pub async fn serve_with_agent(
    agent: Arc<StarpodAgent>,
    config: StarpodConfig,
    notifier: Option<starpod_cron::NotificationSender>,
    paths: ResolvedPaths,
    existing_auth: Option<Arc<AuthStore>>,
) -> starpod_core::Result<()> {
    // Create broadcast channel for WS event push
    let (events_tx, _) = tokio::sync::broadcast::channel::<GatewayEvent>(64);

    // Compose notifier: broadcast to WS clients + forward to original (Telegram) notifier
    let ws_tx = events_tx.clone();
    let notify_agent = agent.clone();
    let composed_notifier: Option<starpod_cron::NotificationSender> = {
        Some(Arc::new(
            move |job_name: String, session_id: String, result_text: String, success: bool| {
                let notifier = notifier.clone();
                let ws_tx = ws_tx.clone();
                let agent = notify_agent.clone();
                Box::pin(async move {
                    // Mark the cron session as unread
                    if !session_id.is_empty() {
                        let _ = agent.session_mgr().mark_read(&session_id, false).await;
                    }
                    // Broadcast to connected WS clients
                    let _ = ws_tx.send(GatewayEvent::CronComplete {
                        job_name: job_name.clone(),
                        session_id: session_id.clone(),
                        result_preview: result_text.clone(),
                        success,
                    });
                    // Forward to original notifier (Telegram) if present
                    if let Some(ref n) = notifier {
                        (n)(job_name, session_id, result_text, success).await;
                    }
                })
            },
        ))
    };

    // Start the cron scheduler in the background
    let _scheduler_handle = agent.start_scheduler(composed_notifier);
    info!("Cron scheduler started");

    // Run lifecycle prompts (BOOTSTRAP.md on first init, BOOT.md on every start)
    let _lifecycle_handle = agent.run_lifecycle();
    info!("Lifecycle prompts dispatched");

    // Use pre-created auth store or create one from the agent's shared pool
    let auth = match existing_auth {
        Some(a) => a,
        None => {
            let store = Arc::new(AuthStore::from_pool(agent.core_db().pool().clone()));
            let _ = bootstrap_auth(&store).await?;
            store
        }
    };

    // Create rate limiter from config
    let rate_limiter = Arc::new(RateLimiter::new(
        config.auth.rate_limit_requests,
        std::time::Duration::from_secs(config.auth.rate_limit_window_secs),
    ));

    // Load model registry: embedded defaults + optional config override + Ollama discovery.
    let mut model_registry = ModelRegistry::with_defaults();
    let models_path = paths.config_dir.join("models.toml");
    if models_path.exists() {
        match std::fs::read_to_string(&models_path) {
            Ok(contents) => match ModelRegistry::from_toml(&contents) {
                Ok(overrides) => {
                    debug!(path = %models_path.display(), "loaded model registry overrides");
                    model_registry.merge(overrides);
                }
                Err(e) => {
                    warn!(path = %models_path.display(), error = %e, "failed to parse models.toml");
                }
            },
            Err(e) => {
                warn!(path = %models_path.display(), error = %e, "failed to read models.toml");
            }
        }
    }

    // Ollama auto-discovery: populate the registry with locally-available models.
    if let Some(base_url) = config.resolved_provider_base_url("ollama") {
        let discovery = OllamaDiscovery::new(&base_url);
        match discovery.discover_all().await {
            Ok(ollama_registry) => {
                let count = ollama_registry.len();
                model_registry.merge(ollama_registry);
                if count > 0 {
                    debug!(count, "discovered ollama models");
                }
            }
            Err(e) => {
                debug!(error = %e, "ollama discovery unavailable, using static catalog only");
            }
        }
    }

    // Ensure vault is always available — lazily create if the agent didn't have one
    // (e.g. fresh install where .vault_key didn't exist when agent was constructed).
    let vault = match agent.vault().cloned() {
        Some(v) => Some(v),
        None => match starpod_vault::derive_master_key(&paths.db_dir) {
            Ok(master_key) => {
                let vault_db = paths.db_dir.join("vault.db");
                match starpod_vault::Vault::new(&vault_db, &master_key).await {
                    Ok(v) => {
                        debug!("lazily created vault for settings API");
                        Some(Arc::new(v))
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to create vault");
                        None
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to derive vault master key");
                None
            }
        },
    };
    let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
    let state = Arc::new(AppState {
        agent,
        auth,
        rate_limiter,
        config: RwLock::new(config.clone()),
        paths,
        model_registry: Arc::new(model_registry),
        events_tx,
        vault,
        telegram_handle: tokio::sync::Mutex::new(None),
        slack_handle: tokio::sync::Mutex::new(None),
        update_cache: system::new_update_cache(),
        shutdown_tx,
    });

    // Start Telegram bot if configured
    state.restart_telegram().await;

    // Start Slack bot if configured
    state.restart_slack().await;

    // Start config file watcher in background
    let _watcher_handle = start_config_watcher(Arc::clone(&state), &config, &state.paths);

    // Subscribe to shutdown signal for graceful shutdown (used by self-update)
    let mut shutdown_rx = state.shutdown_tx.subscribe();

    let app = build_router(state);

    let addr = &config.server_addr;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        starpod_core::StarpodError::Config(format!("Failed to bind {}: {}", addr, e))
    })?;

    info!(addr = %addr, "Starpod gateway listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.wait_for(|&v| v).await;
            info!("graceful shutdown signal received");
        })
        .await
        .map_err(|e| starpod_core::StarpodError::Config(format!("Server error: {}", e)))?;

    Ok(())
}

/// Start a file watcher that hot-reloads config on change.
///
/// When `paths` is provided (workspace-aware mode), watches the relevant
/// config files. Otherwise falls back to legacy `.starpod/` watching.
///
/// Returns a handle that keeps the watcher alive. The watcher is dropped
/// (and stops) when the handle is dropped.
fn start_config_watcher(
    state: Arc<AppState>,
    _config: &StarpodConfig,
    paths: &ResolvedPaths,
) -> Option<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>> {
    let paths_clone = paths.clone();
    #[allow(clippy::type_complexity)]
    let (watch_dir, watch_files, reload_fn): (
        PathBuf,
        Vec<PathBuf>,
        Box<dyn Fn() -> starpod_core::Result<StarpodConfig> + Send>,
    ) = match &paths.mode {
        starpod_core::Mode::Workspace { root, .. } => {
            let watch_files = vec![root.join("starpod.toml"), paths.agent_toml.clone()];
            let watch = paths.config_dir.clone();
            let p = paths_clone.clone();
            (
                watch,
                watch_files,
                Box::new(move || {
                    let agent_cfg = reload_agent_config(&p)?;
                    Ok(agent_cfg.into_starpod_config(&p))
                }),
            )
        }
        starpod_core::Mode::Instance { .. } => {
            let watch = paths.config_dir.clone();
            let agent_toml = paths.agent_toml.clone();
            let p = paths_clone.clone();
            (
                watch,
                vec![agent_toml],
                Box::new(move || {
                    let agent_cfg = reload_agent_config(&p)?;
                    Ok(agent_cfg.into_starpod_config(&p))
                }),
            )
        }
        starpod_core::Mode::SingleAgent { .. } => {
            let watch = paths.config_dir.clone();
            let agent_toml = paths.agent_toml.clone();
            let p = paths_clone.clone();
            (
                watch,
                vec![agent_toml],
                Box::new(move || {
                    let agent_cfg = reload_agent_config(&p)?;
                    Ok(agent_cfg.into_starpod_config(&p))
                }),
            )
        }
    };

    if !watch_dir.exists() {
        debug!(dir = %watch_dir.display(), "Watch directory not found, skipping config watcher");
        return None;
    }

    let (tx, rx) = std::sync::mpsc::channel();

    let mut debouncer = match new_debouncer(Duration::from_secs(2), tx) {
        Ok(d) => d,
        Err(e) => {
            warn!(error = %e, "Failed to create config file watcher");
            return None;
        }
    };

    // Watch the directory for changes
    if let Err(e) = debouncer
        .watcher()
        .watch(&watch_dir, notify::RecursiveMode::NonRecursive)
    {
        warn!(error = %e, dir = %watch_dir.display(), "Failed to watch directory");
        return None;
    }

    // If workspace or instance mode, also watch the workspace root for starpod.toml changes
    match &paths.mode {
        starpod_core::Mode::Workspace { root, .. } => {
            let _ = debouncer
                .watcher()
                .watch(root, notify::RecursiveMode::NonRecursive);
        }
        starpod_core::Mode::Instance { instance_root, .. } => {
            // Watch workspace root (grandparent of instance_root)
            if let Some(workspace_root) = instance_root.parent().and_then(|p| p.parent()) {
                let _ = debouncer
                    .watcher()
                    .watch(workspace_root, notify::RecursiveMode::NonRecursive);
            }
        }
        _ => {}
    }

    info!(dir = %watch_dir.display(), "Config hot reload enabled");

    // Capture the tokio runtime handle for async operations from the watcher thread
    let rt_handle = tokio::runtime::Handle::current();

    // Spawn a background thread (not async — notify uses std channels)
    std::thread::spawn(move || {
        for events in rx {
            match events {
                Ok(events) => {
                    let config_changed = events.iter().any(|e| {
                        e.kind == DebouncedEventKind::Any
                            && watch_files.iter().any(|f| &e.path == f)
                    });

                    if config_changed {
                        info!("Config file changed, reloading...");
                        match reload_fn() {
                            Ok(new_config) => {
                                let old_config = state.config.read().unwrap().clone();

                                // Log what changed
                                if old_config.models != new_config.models {
                                    info!(old = ?old_config.models, new = ?new_config.models, "Models changed");
                                }
                                if old_config.agent_name != new_config.agent_name {
                                    info!(old = %old_config.agent_name, new = %new_config.agent_name, "Agent name changed");
                                }
                                if old_config.server_addr != new_config.server_addr {
                                    warn!(
                                        old = %old_config.server_addr,
                                        new = %new_config.server_addr,
                                        "server_addr changed — restart required to take effect"
                                    );
                                }

                                // Check if Telegram config changed
                                let tg_changed =
                                    old_config.channels.telegram != new_config.channels.telegram;
                                // Check if Slack config changed
                                let slack_changed =
                                    old_config.channels.slack != new_config.channels.slack;

                                // Update the agent's config (affects next chat request)
                                state.agent.reload_config(new_config.clone());

                                // Update the gateway's config
                                *state.config.write().unwrap() = new_config;

                                // Restart Telegram bot if its config changed
                                if tg_changed {
                                    info!("Telegram config changed, restarting bot...");
                                    let state = Arc::clone(&state);
                                    let rt = rt_handle.clone();
                                    rt.spawn(async move {
                                        state.restart_telegram().await;
                                    });
                                }

                                // Restart Slack bot if its config changed
                                if slack_changed {
                                    info!("Slack config changed, restarting bot...");
                                    let state = Arc::clone(&state);
                                    let rt = rt_handle.clone();
                                    rt.spawn(async move {
                                        state.restart_slack().await;
                                    });
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to reload config (keeping previous config)");
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = ?e, "Config watcher error");
                }
            }
        }
    });

    Some(debouncer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_starpod_config() -> starpod_core::StarpodConfig {
        starpod_core::StarpodConfig {
            models: vec!["anthropic/claude-sonnet-4-6".into()],
            ..starpod_core::StarpodConfig::default()
        }
    }

    #[test]
    fn inject_frontend_config_with_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("frontend.toml"),
            "greeting = \"Hi!\"\nprompts = [\"help me\"]\n",
        )
        .unwrap();

        let html = "<html><head><title>Test</title></head><body></body></html>";
        let cfg = test_starpod_config();
        let result = inject_frontend_config(html, dir.path(), &cfg);

        assert!(result.contains("window.__STARPOD__="));
        assert!(result.contains("\"greeting\":\"Hi!\""));
        assert!(result.contains("\"prompts\":[\"help me\"]"));
        assert!(result.contains("\"models\":[\"anthropic/claude-sonnet-4-6\"]"));
        assert!(result.contains("\"agent_name\":\"Nova\""));
        assert!(
            result.contains("</head>"),
            "closing head tag should be preserved"
        );
    }

    #[test]
    fn inject_frontend_config_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let html = "<html><head></head><body></body></html>";
        let cfg = test_starpod_config();
        let result = inject_frontend_config(html, dir.path(), &cfg);

        assert!(result.contains("window.__STARPOD__="));
        assert!(result.contains("\"greeting\":null"));
        assert!(result.contains("\"prompts\":[]"));
        assert!(result.contains("\"models\":[\"anthropic/claude-sonnet-4-6\"]"));
    }

    #[test]
    fn inject_frontend_config_preserves_html_structure() {
        let dir = tempfile::tempdir().unwrap();
        let html = "<html><head><meta charset=\"UTF-8\"></head><body>content</body></html>";
        let cfg = test_starpod_config();
        let result = inject_frontend_config(html, dir.path(), &cfg);

        assert!(result.contains("<meta charset=\"UTF-8\">"));
        assert!(result.contains("<body>content</body>"));
        // Script tag should be injected before </head>
        let head_pos = result.find("</head>").unwrap();
        let script_pos = result.find("<script>window.__STARPOD__=").unwrap();
        assert!(script_pos < head_pos);
    }

    #[test]
    fn gateway_event_cron_complete_serializes_correctly() {
        let event = GatewayEvent::CronComplete {
            job_name: "daily-digest".into(),
            session_id: "sess-xyz".into(),
            result_preview: "Digest sent".into(),
            success: true,
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();

        assert_eq!(json["type"], "cron_complete");
        assert_eq!(json["job_name"], "daily-digest");
        assert_eq!(json["session_id"], "sess-xyz");
        assert_eq!(json["result_preview"], "Digest sent");
        assert_eq!(json["success"], true);
    }

    #[test]
    fn gateway_event_is_clone_and_send() {
        let event = GatewayEvent::CronComplete {
            job_name: "test".into(),
            session_id: "s".into(),
            result_preview: "ok".into(),
            success: true,
        };
        // Clone works (required by broadcast channel)
        let _cloned = event.clone();
        // Send is required for broadcast — this compiles = it's Send
        fn assert_send<T: Send>(_t: &T) {}
        assert_send(&event);
    }

    #[test]
    fn broadcast_channel_creation() {
        // Verify broadcast channel can be created with GatewayEvent
        let (tx, _rx) = tokio::sync::broadcast::channel::<GatewayEvent>(64);
        let event = GatewayEvent::CronComplete {
            job_name: "test".into(),
            session_id: "s1".into(),
            result_preview: "result".into(),
            success: true,
        };
        // Should not panic — no subscribers is fine
        let _ = tx.send(event);
    }

    fn test_paths(dir: &std::path::Path) -> starpod_core::ResolvedPaths {
        let agent_home = dir.join("starpod");
        let config_dir = agent_home.join("config");
        let db_dir = agent_home.join("db");
        let skills_dir = agent_home.join("skills");
        let connectors_dir = agent_home.join("connectors");
        let users_dir = agent_home.join("users");
        let home_dir = dir.join("home");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&connectors_dir).unwrap();
        std::fs::create_dir_all(&users_dir).unwrap();
        std::fs::create_dir_all(&home_dir).unwrap();
        starpod_core::ResolvedPaths {
            mode: starpod_core::Mode::SingleAgent {
                starpod_dir: agent_home.clone(),
            },
            agent_toml: config_dir.join("agent.toml"),
            agent_home,
            config_dir,
            db_dir,
            skills_dir,
            connectors_dir,
            project_root: dir.to_path_buf(),
            instance_root: dir.to_path_buf(),
            home_dir,
            users_dir,
            env_file: None,
        }
    }

    #[tokio::test]
    async fn create_auth_store_uses_env_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());

        // Set env var before creating auth store
        std::env::set_var("STARPOD_API_KEY", "test-secret-key-123");

        let bootstrap = create_auth_store(&paths).await.unwrap();

        // First run: should return the configured key
        assert_eq!(bootstrap.admin_key.as_deref(), Some("test-secret-key-123"));

        // Verify the key actually works for auth
        let user = bootstrap
            .store
            .authenticate_api_key("test-secret-key-123")
            .await
            .unwrap();
        assert!(
            user.is_some(),
            "imported key should authenticate successfully"
        );

        // Second run: admin exists, returns None (this is why dev needs the fallback)
        let bootstrap2 = create_auth_store(&paths).await.unwrap();
        assert!(
            bootstrap2.admin_key.is_none(),
            "should return None when admin already exists"
        );

        std::env::remove_var("STARPOD_API_KEY");
    }

    #[tokio::test]
    async fn create_auth_store_generates_key_without_env() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());

        // Ensure STARPOD_API_KEY is NOT set
        std::env::remove_var("STARPOD_API_KEY");

        let bootstrap = create_auth_store(&paths).await.unwrap();

        // Should auto-generate a key (may be env-provided if test runs parallel
        // with create_auth_store_uses_env_api_key, but must always return Some)
        assert!(
            bootstrap.admin_key.is_some(),
            "should generate a key on first run"
        );
    }

    #[tokio::test]
    async fn create_auth_store_returns_none_on_subsequent_run() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());

        std::env::remove_var("STARPOD_API_KEY");

        // First run bootstraps the admin
        let bootstrap1 = create_auth_store(&paths).await.unwrap();
        assert!(bootstrap1.admin_key.is_some());

        // Second run: admin already exists → returns None
        let bootstrap2 = create_auth_store(&paths).await.unwrap();
        assert!(
            bootstrap2.admin_key.is_none(),
            "should return None when admin already exists"
        );
    }
}
