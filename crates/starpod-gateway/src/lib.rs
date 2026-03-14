mod routes;
mod ws;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
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

/// Build the Axum router with all routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(routes::api_routes())
        .merge(ws::ws_routes())
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
