mod routes;
mod ws;

use std::sync::Arc;

use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use orion_agent::OrionAgent;
use orion_core::OrionConfig;

/// Shared application state.
pub struct AppState {
    pub agent: Arc<OrionAgent>,
    pub api_key: Option<String>,
}

/// Serve the embedded web UI.
async fn index_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../static/index.html"),
    )
}

/// Build the Axum router with all routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .merge(routes::api_routes())
        .merge(ws::ws_routes())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the gateway server (creates its own agent).
pub async fn serve(config: OrionConfig) -> orion_core::Result<()> {
    let agent = Arc::new(OrionAgent::new(config.clone()).await?);
    serve_with_agent(agent, config).await
}

/// Start the gateway server with a pre-built agent (for sharing with Telegram bot).
pub async fn serve_with_agent(
    agent: Arc<OrionAgent>,
    config: OrionConfig,
) -> orion_core::Result<()> {
    // Start the cron scheduler in the background
    let _scheduler_handle = agent.start_scheduler();
    info!("Cron scheduler started");

    let api_key = std::env::var("ORION_API_KEY").ok();

    let state = Arc::new(AppState {
        agent,
        api_key,
    });

    let app = build_router(state);

    let addr = &config.server_addr;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| orion_core::OrionError::Config(format!("Failed to bind {}: {}", addr, e)))?;

    info!(addr = %addr, "Orion gateway listening");

    axum::serve(listener, app)
        .await
        .map_err(|e| orion_core::OrionError::Config(format!("Server error: {}", e)))?;

    Ok(())
}
