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
| `attachments` | array | No | File attachments (base64-encoded, see [attachment settings](/getting-started/configuration#attachments)) |

Each attachment object has:

| Field | Type | Description |
|-------|------|-------------|
| `file_name` | string | Original filename |
| `mime_type` | string | MIME type (e.g. `"image/png"`, `"application/pdf"`) |
| `data` | string | Base64-encoded file content |

Images (`image/png`, `image/jpeg`, `image/gif`, `image/webp`) are sent to Claude for vision analysis. All files are saved to `{project_root}/downloads/`.

Attachments are validated against the `[attachments]` settings in `agent.toml` — you can disable uploads, restrict file extensions, and set a max file size. See [Attachment Settings](/getting-started/configuration#attachments).

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
  "id": "toolu_abc123",
  "name": "Glob",
  "input": { "pattern": "*" }
}
```

#### `tool_result`

Emitted when a tool returns a result:

```json
{
  "type": "tool_result",
  "tool_use_id": "toolu_abc123",
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

#### `attachment`

Emitted after `stream_end` when the agent attached files for the user (via the `Attach` tool). One message per file.

```json
{
  "type": "attachment",
  "file_name": "report.csv",
  "mime_type": "text/csv",
  "data": "<base64-encoded-file-content>"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `file_name` | string | Original filename |
| `mime_type` | string | MIME type (inferred from extension) |
| `data` | string | Base64-encoded file content (max 20 MB decoded) |

The web UI renders images inline and shows other file types as download links. If you're building a custom client, decode the base64 data and present it appropriately for the file type.

#### `error`

Emitted on errors:

```json
{
  "type": "error",
  "message": "API key not configured"
}
```

#### `notification`

Pushed to **all connected clients** when a cron job or heartbeat completes. This is not part of a chat stream — it arrives independently at any time.

```json
{
  "type": "notification",
  "job_name": "daily-summary",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "result_preview": "No critical errors found today.",
  "success": true
}
```

| Field | Type | Description |
|-------|------|-------------|
| `job_name` | string | Name of the cron job that completed |
| `session_id` | string | Session created by the job (empty on failure) |
| `result_preview` | string | Result summary (truncated to 500 chars) |
| `success` | boolean | Whether the job succeeded |

The web UI uses this to show a toast notification and refresh the session sidebar. If you're building a custom client, you can use this to trigger alerts or update a dashboard.

::: tip
Notifications can arrive during an active chat stream. Handle them independently of the `stream_start` → `stream_end` flow.
:::

## Event Sequence

A typical exchange looks like:

```
Client:  {"type": "message", "text": "List files"}
Server:  {"type": "stream_start", "session_id": "..."}
Server:  {"type": "tool_use", "id": "...", "name": "Glob", "input": {"pattern": "*"}}
Server:  {"type": "tool_result", "tool_use_id": "...", "content": "...", "is_error": false}
Server:  {"type": "text_delta", "text": "Here are the files:\n\n"}
Server:  {"type": "text_delta", "text": "- Cargo.toml\n"}
Server:  {"type": "text_delta", "text": "- README.md\n"}
Server:  {"type": "stream_end", ...}
```

When the agent attaches files, they arrive after `stream_end`:

```
Client:  {"type": "message", "text": "Generate a CSV report"}
Server:  {"type": "stream_start", ...}
Server:  {"type": "tool_use", "name": "FileWrite", ...}
Server:  {"type": "tool_result", ...}
Server:  {"type": "tool_use", "name": "Attach", "input": {"path": "report.csv"}}
Server:  {"type": "tool_result", ...}
Server:  {"type": "text_delta", "text": "Here's your report."}
Server:  {"type": "stream_end", ...}
Server:  {"type": "attachment", "file_name": "report.csv", ...}
```

## Followup Messages

You can send additional messages while a stream is active. The behavior depends on the `followup_mode` setting in `agent.toml`:

### Inject Mode (default)

Messages are integrated into the running agent loop at the next iteration boundary. The agent sees them as additional context before its next API call.

```
Client:  {"type": "message", "text": "List files"}
Server:  {"type": "stream_start", "session_id": "..."}
Server:  {"type": "tool_use", "name": "Glob", ...}
Client:  {"type": "message", "text": "also check hidden files"}   ← sent during stream
Server:  {"type": "tool_result", ...}
Server:  {"type": "text_delta", "text": "..."}                    ← response includes both requests
Server:  {"type": "stream_end", ...}
```

### Queue Mode

Messages are buffered and processed as a new agent loop after the current stream finishes.

```
Client:  {"type": "message", "text": "List files"}
Server:  {"type": "stream_start", ...}
Client:  {"type": "message", "text": "also check hidden files"}   ← queued
Server:  {"type": "stream_end", ...}
Server:  {"type": "stream_start", ...}                            ← new stream for queued message
Server:  {"type": "stream_end", ...}
```

## Reconnection

The web UI implements auto-reconnect with exponential backoff. If you're building a custom client, handle WebSocket disconnection and reconnect with increasing delays.

## Session Continuity

Pass the same `channel_session_key` across reconnections to continue the same conversation. The session manager persists all messages to the database, so history is preserved.
