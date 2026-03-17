mod routes;
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
use tower_http::trace::TraceLayer;
use tracing::{info, warn, debug};

use starpod_agent::StarpodAgent;
use starpod_core::{StarpodConfig, ResolvedPaths, reload_agent_config};

/// Shared application state.
///
/// Config is wrapped in `RwLock` for hot reload support.
pub struct AppState {
    pub agent: Arc<StarpodAgent>,
    pub api_key: Option<String>,
    pub config: RwLock<StarpodConfig>,
    pub paths: ResolvedPaths,
}

/// Embedded web UI assets (built by Vite into static/dist/).
#[derive(Embed)]
#[folder = "static/dist/"]
struct Asset;

/// Embedded documentation site (built by VitePress into docs/.vitepress/dist/).
static DOCS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../docs/.vitepress/dist");

/// Serve embedded static files, falling back to index.html for SPA routing.
async fn static_handler(uri: Uri) -> Response {
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

    // Fallback to index.html (SPA)
    match Asset::get("index.html") {
        Some(file) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(file.data.to_vec()))
            .unwrap(),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Web UI not found. Run `npm run build` in web/ first."))
            .unwrap(),
    }
}

/// Serve embedded docs under /docs, handling VitePress clean URLs.
async fn docs_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches("/docs").trim_start_matches('/');

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
    } else if let Some(file) = DOCS_DIR.get_file(&format!("{}.html", file_path)) {
        let mime = mime_from_path(&format!("{}.html", file_path));
        ([(header::CONTENT_TYPE, mime)], file.contents()).into_response()
    } else if let Some(file) = DOCS_DIR.get_file(&format!("{}/index.html", file_path)) {
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
    Router::new()
        .merge(routes::api_routes())
        .merge(ws::ws_routes())
        .route("/docs", get(docs_handler))
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
pub async fn serve_with_agent(
    agent: Arc<StarpodAgent>,
    config: StarpodConfig,
    notifier: Option<starpod_cron::NotificationSender>,
    paths: ResolvedPaths,
) -> starpod_core::Result<()> {
    // Start the cron scheduler in the background
    let _scheduler_handle = agent.start_scheduler(notifier);
    info!("Cron scheduler started");

    // Run lifecycle prompts (BOOTSTRAP.md on first init, BOOT.md on every start)
    let _lifecycle_handle = agent.run_lifecycle();
    info!("Lifecycle prompts dispatched");

    let api_key = std::env::var("STARPOD_API_KEY").ok();

    let state = Arc::new(AppState {
        agent,
        api_key,
        config: RwLock::new(config.clone()),
        paths: paths,
    });

    // Start config file watcher in background
    let _watcher_handle = start_config_watcher(Arc::clone(&state), &config, &state.paths);

    let app = build_router(state);

    let addr = &config.server_addr;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| starpod_core::StarpodError::Config(format!("Failed to bind {}: {}", addr, e)))?;

    info!(addr = %addr, "Starpod gateway listening");

    axum::serve(listener, app)
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
    let (watch_dir, watch_files, reload_fn): (PathBuf, Vec<PathBuf>, Box<dyn Fn() -> starpod_core::Result<StarpodConfig> + Send>) =
        match &paths.mode {
            starpod_core::Mode::Workspace { root, .. } => {
                let watch_files = vec![
                    root.join("starpod.toml"),
                    paths.agent_toml.clone(),
                ];
                let watch = paths.agent_home.clone();
                let p = paths_clone.clone();
                (watch, watch_files, Box::new(move || {
                    let agent_cfg = reload_agent_config(&p)?;
                    Ok(agent_cfg.into_starpod_config(&p))
                }))
            }
            starpod_core::Mode::Instance { .. } => {
                let watch = paths.agent_home.clone();
                let agent_toml = paths.agent_toml.clone();
                let p = paths_clone.clone();
                (watch, vec![agent_toml], Box::new(move || {
                    let agent_cfg = reload_agent_config(&p)?;
                    Ok(agent_cfg.into_starpod_config(&p))
                }))
            }
            starpod_core::Mode::SingleAgent { starpod_dir } => {
                let watch = starpod_dir.clone();
                let agent_toml = paths.agent_toml.clone();
                let p = paths_clone.clone();
                (watch, vec![agent_toml], Box::new(move || {
                    let agent_cfg = reload_agent_config(&p)?;
                    Ok(agent_cfg.into_starpod_config(&p))
                }))
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
    if let Err(e) = debouncer.watcher().watch(
        &watch_dir,
        notify::RecursiveMode::NonRecursive,
    ) {
        warn!(error = %e, dir = %watch_dir.display(), "Failed to watch directory");
        return None;
    }

    // If workspace or instance mode, also watch the workspace root for starpod.toml changes
    match &paths.mode {
        starpod_core::Mode::Workspace { root, .. } => {
            let _ = debouncer.watcher().watch(
                root,
                notify::RecursiveMode::NonRecursive,
            );
        }
        starpod_core::Mode::Instance { instance_root, .. } => {
            // Watch workspace root (grandparent of instance_root)
            if let Some(workspace_root) = instance_root.parent().and_then(|p| p.parent()) {
                let _ = debouncer.watcher().watch(
                    workspace_root,
                    notify::RecursiveMode::NonRecursive,
                );
            }
        }
        _ => {}
    }

    info!(dir = %watch_dir.display(), "Config hot reload enabled");

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
                                if old_config.model != new_config.model {
                                    info!(old = %old_config.model, new = %new_config.model, "Model changed");
                                }
                                if old_config.provider != new_config.provider {
                                    info!(old = %old_config.provider, new = %new_config.provider, "Provider changed");
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

                                // Update the agent's config (affects next chat request)
                                state.agent.reload_config(new_config.clone());

                                // Update the gateway's config
                                *state.config.write().unwrap() = new_config;
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
