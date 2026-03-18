# Web UI

Starpod ships with an embedded web UI served at `http://localhost:3000/` when running `starpod dev <agent>`.

## Features

- **Dark theme** — minimal, clean interface
- **Streaming responses** — live text deltas via WebSocket
- **File uploads** — attach files via paperclip button or drag & drop (max 20 MB per file). Images are sent to Claude for vision analysis; other files are saved to `{data_dir}/downloads/`
- **Tool cards** — collapsible cards showing tool calls with input JSON and results
- **Clickable URLs** — links in responses are clickable
- **Usage stats** — turns, cost, and token counts after each response
- **Session history** — sidebar with previous conversations, full message persistence
- **Auto-reconnect** — exponential backoff on WebSocket disconnection

## Authentication

To protect the web UI, set the `STARPOD_API_KEY` environment variable on the server:

```bash
STARPOD_API_KEY="your-secret-key" starpod dev <agent>
```

Then set the key in your browser's console:

```js
localStorage.setItem('starpod_api_key', 'your-secret-key')
```

The key is sent as an `X-API-Key` header on HTTP requests and as a `?token=` query parameter on WebSocket connections.

## How It Works

The web UI is a single HTML file embedded in the `starpod-gateway` binary. It connects via WebSocket to `/ws` and:

1. Sends `{"type": "message", "text": "...", "channel_id": "main", "attachments": [...]}` messages
2. Receives streaming events: `stream_start`, `text_delta`, `tool_use`, `tool_result`, `stream_end`
3. Renders text deltas in real-time with markdown formatting
4. Shows tool calls as expandable cards

## Session Management

Each conversation tab in the web UI uses a unique `channel_session_key` (UUID). The sidebar lists previous sessions with their auto-generated titles.
