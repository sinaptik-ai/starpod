# starpod-gateway

Axum HTTP/WebSocket server with an embedded web UI.

## API

```rust
// Start with config (creates agent internally)
starpod_gateway::serve(config).await?;

// Start with a shared agent (for Telegram co-hosting)
starpod_gateway::serve_with_agent(agent, config, notifier).await?;

// Build just the router (for testing or embedding)
let router = starpod_gateway::build_router(state);
```

## Routes

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Chat (non-streaming) |
| `GET` | `/api/sessions` | List sessions |
| `GET` | `/api/sessions/:id` | Get session metadata |
| `GET` | `/api/sessions/:id/messages` | Get session messages |
| `GET` | `/api/memory/search` | Full-text search |
| `POST` | `/api/memory/reindex` | Rebuild FTS index |
| `GET` | `/api/health` | Health check |
| `GET` | `/ws` | WebSocket streaming |
| `GET` | `/` | Embedded web UI |

## Authentication

Optional API key auth via `STARPOD_API_KEY` environment variable:

- HTTP: `X-API-Key` header
- WebSocket: `?token=` query parameter

## AppState

```rust
pub struct AppState {
    pub agent: Arc<StarpodAgent>,
    pub api_key: Option<String>,
}
```

Shared across all routes via Axum's state extraction.

## WebSocket Protocol

See the [WebSocket documentation](/integrations/websocket) for the full protocol specification.
