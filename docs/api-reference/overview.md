# API Reference

Starpod exposes a REST API and WebSocket endpoint through the `starpod-gateway` crate.

## Base URL

```
http://localhost:3000/api
```

## Authentication

API key authentication is enabled automatically when the first admin user is bootstrapped on startup. Keys use the `sp_live_` prefix and are verified against argon2id hashes in the database.

**HTTP requests** — include the key in the `X-API-Key` header:

```bash
curl -H "X-API-Key: sp_live_..." http://localhost:3000/api/sessions
```

**WebSocket** — pass the key as a query parameter:

```
ws://localhost:3000/ws?token=sp_live_...
```

**Verify** — check if a key is valid (never returns 401):

```bash
curl -H "X-API-Key: sp_live_..." http://localhost:3000/api/auth/verify
# → { "authenticated": true, "auth_disabled": false, "user": { "id": "...", "role": "admin" } }
```

When no users exist yet (fresh install), all endpoints are accessible without a key.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/auth/verify` | Verify API key validity |
| `POST` | [`/api/chat`](/api-reference/chat) | Send a chat message |
| `GET` | [`/api/sessions`](/api-reference/sessions#list-sessions) | List recent sessions |
| `GET` | [`/api/sessions/:id`](/api-reference/sessions#get-session) | Get session metadata |
| `GET` | [`/api/sessions/:id/messages`](/api-reference/sessions#get-messages) | Get session messages |
| `GET` | [`/api/memory/search`](/api-reference/memory#search) | Full-text memory search |
| `POST` | [`/api/memory/reindex`](/api-reference/memory#reindex) | Rebuild FTS5 index |
| `GET` | [`/api/instances`](/api-reference/instances#list-instances) | List remote instances |
| `POST` | [`/api/instances`](/api-reference/instances#create-instance) | Create a new instance |
| `GET` | [`/api/instances/:id`](/api-reference/instances#get-instance) | Get instance details |
| `DELETE` | [`/api/instances/:id`](/api-reference/instances#delete-instance) | Delete (kill) an instance |
| `POST` | [`/api/instances/:id/pause`](/api-reference/instances#pause-instance) | Pause an instance |
| `POST` | [`/api/instances/:id/restart`](/api-reference/instances#restart-instance) | Restart an instance |
| `GET` | [`/api/instances/:id/health`](/api-reference/instances#instance-health) | Instance health info |
| `GET` | [`/api/health`](/api-reference/health) | Health check |
| `GET` | [`/api/system/version`](/api-reference/system#get-apisystemversion) | Version check (current vs latest) |
| `POST` | [`/api/system/update`](/api-reference/system#post-apisystemupdate) | Trigger self-update |
| `GET/PUT` | `/api/settings/general` | General config (model, provider, limits) |
| `GET` | `/api/settings/models` | Well-known models per provider |
| `GET/PUT` | `/api/settings/memory` | Memory settings |
| `GET/PUT` | `/api/settings/cron` | Cron settings |
| `GET/PUT` | `/api/settings/channels` | Channel settings (Telegram enabled, gap, stream mode) |
| `GET/PUT` | `/api/settings/frontend` | Frontend config (greeting, prompts) |
| `GET/PUT` | `/api/settings/files/:name` | Agent personality files (SOUL.md, etc.) |
| `GET/POST` | `/api/settings/auth/users` | List / create auth users |
| `GET/PUT` | `/api/settings/auth/users/:id` | Get / update auth user |
| `GET/PUT/DELETE` | `/api/settings/auth/users/:id/telegram` | Per-user Telegram linking |
| `GET/POST` | `/api/settings/auth/users/:id/api-keys` | User API key management |
| `POST` | `/api/settings/auth/api-keys/:id/revoke` | Revoke an API key |

## WebSocket

| Path | Description |
|------|-------------|
| [`/ws`](/integrations/websocket) | Streaming chat via WebSocket |

## Error Responses

Errors return JSON with an appropriate HTTP status code:

```json
{
  "error": "Unauthorized"
}
```

| Status | Meaning |
|--------|---------|
| `401` | Missing or invalid API key |
| `403` | Forbidden (e.g. non-admin accessing settings) |
| `404` | Resource not found |
| `429` | Rate limit exceeded |
| `500` | Internal server error |
