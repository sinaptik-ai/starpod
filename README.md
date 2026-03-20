# Starpod RS

A local-first personal AI assistant platform built in Rust, powered by Claude. Each project gets its own `.starpod/` directory with config, memory, credentials, skills, and scheduled tasks — no global state.

## Architecture

```
crates/
├── agent-sdk/            Claude API client + agent loop
├── starpod-hooks/        Lifecycle hook system (events, callbacks, permissions)
├── starpod-core/         Shared types, config, error handling
├── starpod-memory/       SQLite FTS5 full-text search + per-user memory
├── starpod-session/      Channel-aware session lifecycle (per-user)
├── starpod-skills/       Self-extension skill system (markdown-based)
├── starpod-cron/         Cron scheduling (interval, cron expr, one-shot)
├── starpod-agent/        Orchestrator wiring everything together
├── starpod-gateway/      Axum HTTP/WS server + embedded web UI
├── starpod-telegram/     Telegram bot interface (teloxide)
├── starpod-instances/    Remote instance management client
└── starpod/              CLI binary
```

## Quick Start

### Prerequisites

- Rust 1.87+
- An Anthropic API key

### Install

```bash
cargo install --path crates/starpod --locked
```

### Initialize a Workspace

```bash
cd your-project
starpod init
```

The interactive wizard walks you through provider selection (Anthropic, OpenAI, Gemini, Groq, DeepSeek, OpenRouter, Ollama), model, API key (saved to `.env`), and optionally creating your first agent.

To skip the wizard and use defaults (Anthropic / `claude-haiku-4-5`):

```bash
starpod init --default
```

### Create an Agent

If you didn't create one during init:

```bash
starpod agent new my-agent
```

### Run

```bash
# Start in dev mode (applies blueprint, creates .instances/, serves)
starpod dev my-agent

# One-shot chat
starpod chat -a my-agent "What files are in this directory?"

# Interactive REPL
starpod repl -a my-agent
```

When you run `starpod dev`, you'll see:

```
  Starpod is running

  Frontend http://127.0.0.1:3000
  API      http://127.0.0.1:3000/api
  WS       ws://127.0.0.1:3000/ws
  Telegram not configured
  Model    claude-haiku-4-5
  Project  /path/to/your-project
```

### Configure

**Workspace config** (`starpod.toml`) — shared defaults for all agents:

```toml
provider = "anthropic"
model = "claude-haiku-4-5"
max_turns = 30
```

**Agent config** (`agents/<name>/agent.toml`) — per-agent overrides:

```toml
agent_name = "Aster"
model = "claude-haiku-4-6"
# reasoning_effort = "medium"
# timezone = "Europe/Rome"

[providers.anthropic]
# base_url = "https://custom.example.com"  # Optional override

[channels.telegram]
# gap_minutes = 360
# allowed_users = [123456789, "alice"]
```

**Secrets** go in `.env` files (never in config):
- `agents/<name>/.env` — production secrets
- `agents/<name>/.env.dev` — dev overrides (used by `starpod dev`)
- `ANTHROPIC_API_KEY`, `TELEGRAM_BOT_TOKEN`, etc.

**Personality** is defined in `agents/<name>/SOUL.md`, not in config.

## CLI Reference

```
starpod init                              Initialize workspace (interactive wizard)
starpod init --default                    Initialize with defaults (no wizard)
starpod agent new <name>                  Create a new agent blueprint
starpod agent list                        List agents in workspace
starpod dev <agent> [--port N]            Apply blueprint + start dev server
starpod serve                             Start production server (single-agent mode)
starpod chat -a <agent> "<message>"       Send a one-shot message
starpod repl -a <agent>                   Interactive REPL session

starpod memory search "<query>" [-l 5]    Full-text search over memory
starpod memory reindex                    Rebuild the FTS5 index
starpod sessions list [-l 10]             List recent sessions

starpod skill list                        List all skills
starpod skill show <name>                 Show a skill's content
starpod skill new <name> -d "..." -b "..."  Create a skill
starpod skill delete <name>               Delete a skill

starpod cron list                         List all cron jobs
starpod cron remove <name>                Remove a cron job
starpod cron runs <name> [-l 10]          Show recent runs for a job
starpod cron run <name>                   Trigger a job immediately
starpod cron edit <name> [--prompt ...] [--schedule ...] [--enabled true/false] [--max-retries N] [--timeout-secs N] [--session-mode ...]
```

## Web UI

The embedded web UI is served at `http://localhost:3000/` when running `starpod dev`.

- Minimal dark theme
- Streaming responses via WebSocket with live text deltas
- Collapsible tool cards — click to expand input JSON and results
- Clickable URLs in responses
- Usage stats after each response (turns, cost, tokens)
- Auto-reconnect with exponential backoff

API key authentication: set `STARPOD_API_KEY` env var on the server, then `localStorage.setItem('starpod_api_key', 'your-key')` in the browser.

## Telegram Bot

### Setup (step by step)

1. **Create a bot with BotFather**
   - Open Telegram and search for `@BotFather`
   - Send `/newbot`
   - Choose a name (e.g. "My Starpod Assistant")
   - Choose a username (must end in `bot`, e.g. `my_starpod_bot`)
   - BotFather will reply with a token like `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`

2. **Add the token to your `.env`**

   Add to `agents/<name>/.env.dev` (for dev) or `agents/<name>/.env` (for prod):
   ```
   TELEGRAM_BOT_TOKEN=123456789:ABCdefGHIjklMNOpqrsTUVwxyz
   ```

3. **Restrict access (recommended)**

   Send `/start` to your bot — it will reply with your user ID and username. Add to `agents/<name>/agent.toml`:
   ```toml
   [channels.telegram]
   allowed_users = [123456789]
   ```
   You can mix user IDs and usernames (without `@`): `allowed_users = [123456789, "alice", 987654321]`

   The bot won't respond to anyone until you add at least one entry. `/start` is the only command that works without being whitelisted (so you can discover your ID and username).

4. **Start the server**
   ```bash
   starpod dev <agent-name>
   ```
   You should see `Telegram  connected` in the startup banner. The bot is now running.

5. **Chat with your bot**
   - Open Telegram and search for your bot's username (e.g. `@my_starpod_bot`)
   - Send `/start` to begin
   - Send any message — the bot uses the same agent as the web UI and API

### Optional: customize your bot in BotFather

- `/setdescription` — set what users see before starting a chat
- `/setabouttext` — set the bio shown on the bot's profile
- `/setuserpic` — set the bot's profile picture
- `/setcommands` — register `/start` as a command with a description

### Features

- Shares the same `StarpodAgent` instance with the web UI and API
- Shows typing indicator while the agent is thinking
- Splits long responses at line boundaries (Telegram's 4096-char limit)
- Sends as HTML (`ParseMode::Html`), falls back to plain text on parse failure

## Gateway API

### Authentication

Set the `STARPOD_API_KEY` environment variable to require API key auth. Clients include it in the `X-API-Key` header (HTTP) or `?token=` query param (WebSocket).

### HTTP Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Send a chat message |
| `GET` | `/api/frame-check?url=...` | Check if a URL allows framing |
| `GET` | `/api/sessions?limit=20` | List recent sessions |
| `GET` | `/api/sessions/:id` | Get a specific session |
| `GET` | `/api/sessions/:id/messages` | Get session messages |
| `GET` | `/api/memory/search?q=...&limit=10` | Full-text search |
| `POST` | `/api/memory/reindex` | Rebuild FTS index |
| `GET` | `/api/instances` | List remote instances |
| `POST` | `/api/instances` | Create a remote instance |
| `GET` | `/api/instances/:id` | Get instance details |
| `DELETE` | `/api/instances/:id` | Delete an instance |
| `POST` | `/api/instances/:id/pause` | Pause an instance |
| `POST` | `/api/instances/:id/restart` | Restart an instance |
| `GET` | `/api/instances/:id/health` | Instance health check |
| `GET` | `/api/health` | Health check |

### WebSocket Streaming

Connect to `ws://localhost:3000/ws` (or `ws://localhost:3000/ws?token=KEY`).

**Client sends:**
```json
{"type": "message", "text": "Hello!", "channel_id": "main", "channel_session_key": "conv-uuid-here"}
```

**Server streams:**
```json
{"type": "stream_start", "session_id": "..."}
{"type": "text_delta", "text": "Hi "}
{"type": "text_delta", "text": "there!"}
{"type": "tool_use", "id": "toolu_abc123", "name": "Read", "input": {"file_path": "/tmp/foo.txt"}}
{"type": "tool_result", "tool_use_id": "toolu_abc123", "content": "file contents...", "is_error": false}
{"type": "stream_end", "session_id": "...", "num_turns": 1, "cost_usd": 0.004, "input_tokens": 1200, "output_tokens": 45, "is_error": false, "errors": []}
```

## Agent Tools

The agent has access to built-in tools from the SDK plus 20 custom tools:

| Category | Tools |
|----------|-------|
| Built-in | `Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep` |
| Memory | `MemorySearch`, `MemoryWrite`, `MemoryAppendDaily` |
| Environment | `EnvGet` |
| File Sandbox | `FileRead`, `FileWrite`, `FileList`, `FileDelete` |
| Skills | `SkillActivate`, `SkillCreate`, `SkillUpdate`, `SkillDelete`, `SkillList` |
| Cron | `CronAdd`, `CronList`, `CronRemove`, `CronRuns`, `CronRun`, `CronUpdate`, `HeartbeatWake` |

The Bash tool supports `run_in_background: true` for long-running processes (servers, etc.) that should not block the agent.

## Crate Details

### agent-sdk

Rust port of the Claude Agent SDK. Provides `query()` which drives the agentic loop: prompt → Claude API → tool execution → feed results → repeat.

```rust
use agent_sdk::{query, Options, Message};
use tokio_stream::StreamExt;

let mut stream = query(
    "What files are in this directory?",
    Options::builder()
        .allowed_tools(vec!["Bash".into(), "Glob".into()])
        .build(),
);

while let Some(msg) = stream.next().await {
    let msg = msg?;
    if let Message::Result(result) = &msg {
        println!("{}", result.result.as_deref().unwrap_or(""));
    }
}
```

### starpod-memory

Persistent memory: markdown files on disk + SQLite FTS5 index. Text is chunked into ~400-token segments with 80-token overlap at line boundaries. `bootstrap_context()` returns SOUL.md (agent-level), while `UserMemoryView` assembles per-user context: USER.md, MEMORY.md, and recent daily logs. Each user gets their own memory space under `.starpod/users/<id>/`.

### starpod-session

Channel-aware session management with per-channel strategies and per-user scoping:

- **`main`** (web, REPL, CLI): Explicit sessions — the client provides a `channel_session_key` (e.g. a conversation UUID) and the session continues until closed. Multiple concurrent sessions are supported.
- **`telegram`**: Time-gap sessions — messages within 6 hours continue the same session (keyed by chat ID), otherwise a new session starts and the old one is auto-closed.

Tracks token usage and cost per turn. The scheduler creates standalone `main` sessions (one per cron run) and delivers results via the configured notification channel.

### starpod-skills

Markdown-based skill files at `.starpod/skills/<name>/SKILL.md`. Skills are injected into the system prompt on every turn. The agent can create, update, and delete skills at runtime.

### starpod-cron

Scheduling system supporting interval (`every_ms`), cron expressions, and one-shot (`at`) schedules. Cron expressions are evaluated in the user's local timezone when `timezone` is configured in agent.toml. A background scheduler (30s tick) runs jobs through `StarpodAgent::chat()` and records run history.

## Development

### Run Tests

```bash
cargo test
```

407 tests across all crates, zero warnings.

```bash
cargo test --workspace
```

## License

MIT
