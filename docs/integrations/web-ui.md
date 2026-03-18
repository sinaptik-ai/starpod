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
- **Cron notifications** — toast alerts when cron jobs or heartbeats complete, with click-to-navigate
- **Unread indicators** — blue dot on sessions with new activity, sorted to the top of the sidebar
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

## Welcome Screen

The welcome screen greeting and suggested prompts are configured via `.starpod/config/frontend.toml`:

```toml
greeting = "Hi! I'm Aster."

prompts = [
    "What can you help me with?",
    "What do you remember about me?",
]
```

Prompt chips appear as monospace terminal-style lines below the greeting. Clicking one sends it as a message immediately. If `frontend.toml` is missing or empty, the welcome screen shows the default `ready_` greeting with no chips.

This file is read on every page load, so you can edit it and refresh the browser to see changes — no server restart needed.

See [Configuration > Frontend](/getting-started/configuration#frontend-web-ui) for the full reference.

## Session Management

Each conversation tab in the web UI uses a unique `channel_session_key` (UUID). The sidebar lists previous sessions with their auto-generated titles.

### Unread Sessions

Sessions are marked as **unread** (shown with a blue dot) when they receive new activity that the user hasn't viewed. This includes:

- Sessions created by cron jobs or heartbeats
- Sessions updated while the user is viewing a different conversation

Unread sessions are sorted to the top of the sidebar. Clicking a session marks it as read and removes the indicator.

Unread state is tracked client-side in `localStorage` (`starpod_read_sessions`). Stale entries are automatically pruned when the session list is refreshed.

## Cron & Heartbeat Notifications

When a cron job or heartbeat completes, a **toast notification** slides in from the top-right corner showing:

- The job name
- A preview of the result (success or failure)
- A success/error indicator

Clicking the toast navigates to the cron job's session, where you can see the full execution transcript (tool calls, results, and agent response). Toasts auto-dismiss after 6 seconds.

The session sidebar also refreshes automatically, so the new cron session appears at the top with an unread indicator.
