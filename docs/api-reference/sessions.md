# Sessions

## List Sessions {#list-sessions}

### GET /api/sessions

```bash
curl http://localhost:3000/api/sessions?limit=20 \
  -H "X-API-Key: your-key"
```

#### Query Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `limit` | integer | `20` | Maximum number of sessions to return |

#### Response

```json
[
  {
    "id": "abc123",
    "channel": "main",
    "channel_session_key": "550e8400-...",
    "title": "File listing and project overview",
    "message_count": 4,
    "created_at": "2026-03-14T10:00:00Z",
    "last_message_at": "2026-03-14T10:05:00Z",
    "is_closed": false,
    "summary": null,
    "user_id": "user123"
  }
]
```

## Get Session {#get-session}

### GET /api/sessions/:id

```bash
curl http://localhost:3000/api/sessions/abc123 \
  -H "X-API-Key: your-key"
```

#### Response

Returns a single session object (same shape as the list items above), or `404` if not found.

## Get Session Messages {#get-messages}

### GET /api/sessions/:id/messages

```bash
curl http://localhost:3000/api/sessions/abc123/messages \
  -H "X-API-Key: your-key"
```

#### Response

```json
[
  {
    "id": 1,
    "session_id": "abc123",
    "role": "user",
    "content": "What files are in this directory?",
    "timestamp": "2026-03-14T10:00:00Z"
  },
  {
    "id": 2,
    "session_id": "abc123",
    "role": "assistant",
    "content": "The directory contains:\n- Cargo.toml\n- README.md",
    "timestamp": "2026-03-14T10:00:05Z"
  }
]
```

#### Message Roles

| Role | Description |
|------|-------------|
| `user` | User message |
| `assistant` | Agent response |
| `tool_use` | Tool call (JSON-encoded name + input) |
| `tool_result` | Tool result (JSON-encoded output) |
