# API Reference

Starpod exposes a REST API and WebSocket endpoint through the `starpod-gateway` crate.

## Base URL

```
http://localhost:3000/api
```

## Authentication

Set the `STARPOD_API_KEY` environment variable to enable API key authentication.

**HTTP requests** — include the key in the `X-API-Key` header:

```bash
curl -H "X-API-Key: your-key" http://localhost:3000/api/health
```

**WebSocket** — pass the key as a query parameter:

```
ws://localhost:3000/ws?token=your-key
```

If `STARPOD_API_KEY` is not set, the API is open (no auth required).

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
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
| `404` | Resource not found |
| `500` | Internal server error |
