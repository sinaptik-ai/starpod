# starpod-gateway

Axum HTTP/WebSocket server with an embedded web UI.

## API

```rust
// Start with a shared agent, config, optional notifier, resolved paths,
// and an optional pre-created auth store (None to create one internally)
starpod_gateway::serve_with_agent(agent, config, notifier, paths, None).await?;

// Build just the router (for testing or embedding)
let router = starpod_gateway::build_router(state);
```

## Routes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/auth/verify` | Verify API key (never 401 — returns auth status as JSON) |
| `POST` | `/api/chat` | Chat (non-streaming) |
| `GET` | `/api/frame-check` | Check if a URL is frameable (X-Frame-Options / CSP) |
| `GET` | `/api/sessions` | List sessions |
| `GET` | `/api/sessions/:id` | Get session metadata |
| `GET` | `/api/sessions/:id/messages` | Get session messages |
| `POST` | `/api/sessions/:id/read` | Mark session read/unread |
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
| `GET` | `/api/system/version` | Version check (current vs latest release) |
| `POST` | `/api/system/update` | Trigger self-update + restart |
| `GET/PUT` | `/api/settings/general` | General config (model, provider, etc.) |
| `GET` | `/api/settings/models` | Well-known models per provider |
| `GET/PUT` | `/api/settings/memory` | Memory settings |
| `GET/PUT` | `/api/settings/cron` | Cron settings |
| `GET/PUT` | `/api/settings/channels` | Channel settings (Telegram) |
| `GET` | `/api/settings/costs?period=30d` | Cost overview (by user, by model) |
| `GET/PUT` | `/api/settings/frontend` | Frontend config (greeting, prompts) |
| `GET/PUT` | `/api/settings/files/:name` | Agent personality files (SOUL.md, etc.) |
| `GET/POST` | `/api/settings/auth/users` | Auth user CRUD |
| `GET/PUT` | `/api/settings/auth/users/:id` | Auth user detail |
| `GET/PUT/DELETE` | `/api/settings/auth/users/:id/telegram` | Per-user Telegram linking |
| `GET` | `/api/settings/vault` | List vault entries with metadata (is_secret, allowed_hosts, proxy_enabled) |
| `PUT/DELETE` | `/api/settings/vault/{key}` | Set or delete a vault entry (supports is_secret, allowed_hosts in body) |
| `PUT` | `/api/settings/vault/{key}/meta` | Update vault entry metadata only (no value re-entry needed) |
| `GET/POST` | `/api/settings/auth/users/:id/api-keys` | User API key management |
| `GET` | `/ws` | WebSocket streaming |
| `GET` | `/docs`, `/docs/*` | Embedded documentation site |
| `GET` | `/` | Embedded web UI (SPA fallback) |

## Authentication

Database-backed API key auth via `starpod-auth`. On first startup, an admin user and API key are bootstrapped automatically. If `STARPOD_API_KEY` is set, it is imported as the admin key for backward compatibility.

- **HTTP:** `X-API-Key` header
- **WebSocket:** `?token=` query parameter
- **Verify:** `GET /api/auth/verify` — returns `{ authenticated, auth_disabled, user }` (always 200, never 401)

When no users exist yet (fresh install), all requests are allowed without a key.

## AppState

```rust
pub struct AppState {
    pub agent: Arc<StarpodAgent>,
    pub auth: Arc<AuthStore>,
    pub rate_limiter: Arc<RateLimiter>,
    pub config: RwLock<StarpodConfig>,
    pub paths: ResolvedPaths,
    pub events_tx: tokio::sync::broadcast::Sender<GatewayEvent>,
    pub update_cache: system::UpdateCache,   // cached latest-release info
    pub shutdown_tx: watch::Sender<bool>,    // graceful shutdown (self-update)
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

## Self-Update

The `system` module (`src/system.rs`) provides version checking and self-update:

- **Version check** (`GET /api/system/version`) — queries GitHub Releases API, caches for 1 hour, compares via semver.
- **Self-update** (`POST /api/system/update`) — downloads the platform tarball, verifies SHA-256, backs up binary + DBs + config to `.starpod/backups/`, replaces the binary, spawns the new process, and gracefully shuts down via `shutdown_tx`.

The old process monitors the new binary for 30 seconds. If it crashes (non-zero exit), the old binary restores itself from the `.bak` file.

## Config Hot Reload

The gateway watches `agent.toml` for changes. When the file is modified, the config is reloaded and applied to both the agent and gateway state. See [Configuration — Hot Reload](/getting-started/configuration#hot-reload) for details.

## WebSocket Protocol

See the [WebSocket documentation](/integrations/websocket) for the full protocol specification.
