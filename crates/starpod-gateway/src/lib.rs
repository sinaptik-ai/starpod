mod routes;
mod ws;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use include_dir::{include_dir, Dir};
use rust_embed::Embed;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use starpod_agent::StarpodAgent;
use starpod_core::StarpodConfig;

/// Shared application state.
pub struct AppState {
    pub agent: Arc<StarpodAgent>,
    pub api_key: Option<String>,
    pub config: StarpodConfig,
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

/// Start the gateway server (creates its own agent).
pub async fn serve(config: StarpodConfig) -> starpod_core::Result<()> {
    let agent = Arc::new(StarpodAgent::new(config.clone()).await?);
    serve_with_agent(agent, config, None).await
}

/// Start the gateway server with a pre-built agent (for sharing with Telegram bot).
pub async fn serve_with_agent(
    agent: Arc<StarpodAgent>,
    config: StarpodConfig,
    notifier: Option<starpod_cron::NotificationSender>,
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
        config: config.clone(),
    });

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
