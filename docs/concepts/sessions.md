# Sessions

Orion tracks conversations as **sessions** with per-channel strategies for creation and continuation.

## Channels

| Channel | Source | Strategy | Session Key |
|---------|--------|----------|-------------|
| `Main` | Web UI, CLI, API | Explicit | Client-provided UUID |
| `Telegram` | Telegram bot | Time-gap | Chat ID |

### Main Channel

The client provides a `channel_session_key` (typically a UUID). The same key always maps to the same session. Multiple concurrent sessions are supported.

```json
{
  "type": "message",
  "text": "Hello!",
  "channel_id": "main",
  "channel_session_key": "550e8400-e29b-41d4-a716-446655440000"
}
```

### Telegram Channel

Messages from the same chat ID within **6 hours** continue the same session. After a 6-hour gap, the old session is auto-closed and a new one begins.

No explicit session management needed — just send messages.

## Resolution Flow

```
Message arrives
    │
    ▼
Resolve channel (Main or Telegram)
    │
    ▼
Look up session by (channel, key)
    │
    ├── Found & within time window → Continue
    │
    └── Not found or gap exceeded → Create new session
                                     (auto-close old if Telegram)
```

## Session Data

| Field | Description |
|-------|-------------|
| `id` | Unique session identifier |
| `channel` | `main` or `telegram` |
| `channel_session_key` | Client key or chat ID |
| `title` | Auto-generated after first turn |
| `message_count` | Number of messages |
| `created_at` | Session start time |
| `last_message_at` | Last activity |
| `is_closed` | Whether the session is closed |

## Usage Tracking

Every agent turn records:

- Input and output tokens
- Cache read/write tokens
- Cost in USD
- Model used
- Turn number

## Conversation Compaction

When a conversation approaches the model's context window limit (~160k tokens), Orion automatically compacts older messages:

1. **Detection** — after each tool-use cycle, the agent checks if `input_tokens` exceeds the context budget (160k tokens)
2. **Summarization** — older messages are sent to a summarizer model (configurable via `compaction_model` in config, defaults to the primary model) which produces a structured summary
3. **Splicing** — old messages are replaced with the summary, preserving the system prompt and recent turns (at least 4 messages)
4. **Persistence** — the full transcript is already persisted to disk; compaction only affects the in-memory context sent to the API
5. **Logging** — a `CompactBoundary` event is emitted and recorded in the `compaction_log` table

Tool-use cycles are never split — if a compaction boundary would fall between a tool call and its result, it moves to keep them together.

Configure the summarization model in `.orion/config.toml`:

```toml
compaction_model = "claude-haiku-4-5"
```

If the compaction model fails, it falls back to the primary model.

## Message Persistence

All messages (user, assistant, tool use/results) are saved to the session database. The web UI loads full history when revisiting a session.

## CLI

```bash
orion sessions list --limit 10
```
