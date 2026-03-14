# WebSocket

The WebSocket endpoint provides real-time streaming for chat interactions.

## Connection

```
ws://localhost:3000/ws
```

With authentication:

```
ws://localhost:3000/ws?token=your-api-key
```

## Protocol

### Client → Server

Send a JSON message to start a conversation:

```json
{
  "type": "message",
  "text": "What files are in this directory?",
  "user_id": "user123",
  "channel_id": "main",
  "channel_session_key": "550e8400-e29b-41d4-a716-446655440000",
  "attachments": [
    {
      "file_name": "photo.png",
      "mime_type": "image/png",
      "data": "<base64-encoded-data>"
    }
  ]
}
```

| Field | Type | Required | Description |
|-------|------|:---:|-------------|
| `type` | string | Yes | Always `"message"` |
| `text` | string | Yes | The user's message |
| `user_id` | string | No | User identifier |
| `channel_id` | string | No | Channel (`"main"` default) |
| `channel_session_key` | string | No | Session key (UUID recommended) |
| `attachments` | array | No | File attachments (base64-encoded, max 20 MB each) |

Each attachment object has:

| Field | Type | Description |
|-------|------|-------------|
| `file_name` | string | Original filename |
| `mime_type` | string | MIME type (e.g. `"image/png"`, `"application/pdf"`) |
| `data` | string | Base64-encoded file content |

Images (`image/png`, `image/jpeg`, `image/gif`, `image/webp`) are sent to Claude for vision analysis. All files are saved to `{data_dir}/downloads/{session_id}/`.

### Server → Client

The server streams a sequence of events:

#### `stream_start`

Emitted when the agent begins processing:

```json
{
  "type": "stream_start",
  "session_id": "abc123"
}
```

#### `text_delta`

Emitted for each chunk of the agent's response:

```json
{
  "type": "text_delta",
  "text": "The directory contains "
}
```

#### `tool_use`

Emitted when the agent calls a tool:

```json
{
  "type": "tool_use",
  "name": "Glob",
  "input": { "pattern": "*" }
}
```

#### `tool_result`

Emitted when a tool returns a result:

```json
{
  "type": "tool_result",
  "content": "Cargo.toml\nREADME.md\nsrc/",
  "is_error": false
}
```

#### `stream_end`

Emitted when the agent finishes:

```json
{
  "type": "stream_end",
  "session_id": "abc123",
  "num_turns": 2,
  "cost_usd": 0.004,
  "input_tokens": 1200,
  "output_tokens": 150,
  "is_error": false,
  "errors": []
}
```

#### `error`

Emitted on errors:

```json
{
  "type": "error",
  "message": "API key not configured"
}
```

## Event Sequence

A typical exchange looks like:

```
Client:  {"type": "message", "text": "List files"}
Server:  {"type": "stream_start", "session_id": "..."}
Server:  {"type": "tool_use", "name": "Glob", "input": {"pattern": "*"}}
Server:  {"type": "tool_result", "content": "...", "is_error": false}
Server:  {"type": "text_delta", "text": "Here are the files:\n\n"}
Server:  {"type": "text_delta", "text": "- Cargo.toml\n"}
Server:  {"type": "text_delta", "text": "- README.md\n"}
Server:  {"type": "stream_end", ...}
```

## Reconnection

The web UI implements auto-reconnect with exponential backoff. If you're building a custom client, handle WebSocket disconnection and reconnect with increasing delays.

## Session Continuity

Pass the same `channel_session_key` across reconnections to continue the same conversation. The session manager persists all messages to the database, so history is preserved.
