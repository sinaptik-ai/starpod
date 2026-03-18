# starpod-gateway

Axum HTTP/WebSocket server with an embedded web UI.

## API

```rust
// Start with a shared agent, config, optional notifier, and resolved paths
starpod_gateway::serve_with_agent(agent, config, notifier, paths).await?;

// Build just the router (for testing or embedding)
let router = starpod_gateway::build_router(state);
```

## Routes

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Chat (non-streaming) |
| `GET` | `/api/frame-check` | Check if a URL is frameable (X-Frame-Options / CSP) |
| `GET` | `/api/sessions` | List sessions |
| `GET` | `/api/sessions/:id` | Get session metadata |
| `GET` | `/api/sessions/:id/messages` | Get session messages |
| `GET` | `/api/memory/search` | Full-text search |
| `POST` | `/api/memory/reindex` | Rebuild FTS index |
| `GET` | `/api/instances` | List remote instances |
| `POST` | `/api/instances` | Create a remote instance |
| `GET` | `/api/instances/:id` | Get instance details |
| `DELETE` | `/api/instances/:id` | Kill (terminate) an instance |
| `POST` | `/api/instances/:id/pause` | Pause an instance |
| `POST` | `/api/instances/:id/restart` | Restart an instance |
| `GET` | `/api/instances/:id/health` | Instance health info |
| `GET` | `/api/health` | Health check |
| `GET` | `/ws` | WebSocket streaming |
| `GET` | `/docs`, `/docs/*` | Embedded documentation site |
| `GET` | `/` | Embedded web UI (SPA fallback) |

## Authentication

Optional API key auth via `STARPOD_API_KEY` environment variable:

- HTTP: `X-API-Key` header
- WebSocket: `?token=` query parameter

## AppState

```rust
pub struct AppState {
    pub agent: Arc<StarpodAgent>,
    pub api_key: Option<String>,
    pub config: RwLock<StarpodConfig>,
    pub paths: ResolvedPaths,
    pub events_tx: tokio::sync::broadcast::Sender<GatewayEvent>,
}
```

Shared across all routes via Axum's state extraction. Config is wrapped in `RwLock` for hot reload support. The `events_tx` broadcast channel pushes cron/heartbeat notifications to all connected WebSocket clients.

## Event Broadcasting

When a cron job or heartbeat completes, the gateway broadcasts a `GatewayEvent` to all connected WebSocket clients:

```rust
pub enum GatewayEvent {
    CronComplete {
        job_name: String,      // Job that completed
        session_id: String,    // Session created (empty on failure)
        result_preview: String, // Truncated result (500 chars)
        success: bool,         // Success or failure
    },
}
```

The gateway composes the cron `NotificationSender` to both:
1. Broadcast to the WS event channel (for web UI toasts + session list updates)
2. Forward to the original Telegram notifier (if configured)

This composition happens transparently in `serve_with_agent` — callers pass their Telegram notifier and the gateway wraps it.

## Config Hot Reload

The gateway watches `agent.toml` for changes. When the file is modified, the config is reloaded and applied to both the agent and gateway state. See [Configuration — Hot Reload](/getting-started/configuration#hot-reload) for details.

## WebSocket Protocol

See the [WebSocket documentation](/integrations/websocket) for the full protocol specification.
