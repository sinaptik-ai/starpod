# Orion RS

A local-first personal AI assistant platform built in Rust, powered by Claude. Each project gets its own `.orion/` directory with config, memory, credentials, skills, and scheduled tasks — no global state.

## Architecture

```
crates/
├── agent-sdk/          Claude API client + agent loop
├── orion-core/         Shared types, config, error handling
├── orion-memory/       SQLite FTS5 full-text search + markdown files
├── orion-vault/        AES-256-GCM encrypted credential storage
├── orion-session/      Session lifecycle + time-gap analysis
├── orion-skills/       Self-extension skill system (markdown-based)
├── orion-cron/         Cron scheduling (interval, cron expr, one-shot)
├── orion-agent/        Orchestrator wiring everything together
├── orion-gateway/      Axum HTTP/WS server + embedded web UI
├── orion-telegram/     Telegram bot interface (teloxide)
└── orion/              CLI binary
```

## Quick Start

### Prerequisites

- Rust 1.87+
- An Anthropic API key

### Install

```bash
cargo install --path crates/orion --locked
```

### Initialize a Project

```bash
cd your-project
orion agent init
```

This creates `.orion/config.toml` and `.orion/data/` in the current directory.

### Set Your API Key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Run

```bash
# Start the server (web UI + API + WebSocket + optional Telegram bot)
orion agent serve

# One-shot chat
orion agent chat "What files are in this directory?"

# Interactive REPL
orion agent repl
```

When you run `orion agent serve`, you'll see:

```
  Orion is running

  Frontend http://127.0.0.1:3000
  API      http://127.0.0.1:3000/api
  WS       ws://127.0.0.1:3000/ws
  Telegram not configured
  Model    claude-haiku-4-5
  Project  /path/to/your-project
```

### Configure

Edit `.orion/config.toml`:

```toml
# Claude model to use
model = "claude-haiku-4-5"

# Maximum agentic turns per request
max_turns = 30

# Server bind address
server_addr = "127.0.0.1:3000"

# Anthropic API key (or set ANTHROPIC_API_KEY env var)
# api_key = "sk-ant-..."

# Telegram bot token (or set TELEGRAM_BOT_TOKEN env var)
# telegram_bot_token = "123456:ABC..."
```

Config is per-project. Orion walks up from the current directory to find the nearest `.orion/` folder (like git finds `.git/`).

## CLI Reference

```
orion agent init                        Initialize .orion/ in current directory
orion agent serve                       Start server (web UI + API + WS + Telegram)
orion agent chat "<message>"            Send a one-shot message
orion agent repl                        Interactive REPL session

orion instance create                   Create a remote instance (coming soon)
orion instance list                     List running instances
orion instance kill <id>                Kill an instance
orion instance pause <id>               Pause an instance
orion instance restart <id>             Restart an instance

orion memory search "<query>" [-l 5]    Full-text search over memory
orion memory reindex                    Rebuild the FTS5 index

orion vault get <key>                   Retrieve a stored credential
orion vault set <key> <value>           Encrypt and store a credential
orion vault delete <key>                Delete a credential
orion vault list                        List all stored keys

orion sessions list [-l 10]             List recent sessions

orion skills list                       List all skills
orion skills show <name>                Show a skill's content
orion skills create <name> -c "..."     Create a skill from inline content
orion skills create <name> -f file.md   Create a skill from a file
orion skills delete <name>              Delete a skill

orion cron list                         List all cron jobs
orion cron remove <name>                Remove a cron job
orion cron runs <name> [-l 10]          Show recent runs for a job
```

## Web UI

The embedded web UI is served at `http://localhost:3000/` when running `orion agent serve`.

- Minimal dark theme
- Streaming responses via WebSocket with live text deltas
- Collapsible tool cards — click to expand input JSON and results
- Clickable URLs in responses
- Usage stats after each response (turns, cost, tokens)
- Auto-reconnect with exponential backoff

API key authentication: set `ORION_API_KEY` env var on the server, then `localStorage.setItem('orion_api_key', 'your-key')` in the browser.

## Telegram Bot

Set `telegram_bot_token` in `.orion/config.toml` or the `TELEGRAM_BOT_TOKEN` env var. The bot starts automatically alongside the gateway when you run `orion agent serve`, sharing the same agent instance.

Features:
- Handles `/start` command
- Shows typing indicator while processing
- Splits long messages at line boundaries (Telegram's 4096-char limit)
- Sends as MarkdownV2, falls back to plain text on parse failure

## Gateway API

### Authentication

Set the `ORION_API_KEY` environment variable to require API key auth. Clients include it in the `X-API-Key` header (HTTP) or `?token=` query param (WebSocket).

### HTTP Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Send a chat message |
| `GET` | `/api/sessions?limit=20` | List recent sessions |
| `GET` | `/api/sessions/:id` | Get a specific session |
| `GET` | `/api/memory/search?q=...&limit=10` | Full-text search |
| `POST` | `/api/memory/reindex` | Rebuild FTS index |
| `GET` | `/api/health` | Health check |

### WebSocket Streaming

Connect to `ws://localhost:3000/ws` (or `ws://localhost:3000/ws?token=KEY`).

**Client sends:**
```json
{"type": "message", "text": "Hello!", "channel_id": "web"}
```

**Server streams:**
```json
{"type": "stream_start", "session_id": "..."}
{"type": "text_delta", "text": "Hi "}
{"type": "text_delta", "text": "there!"}
{"type": "tool_use", "name": "Read", "input": {"file_path": "/tmp/foo.txt"}}
{"type": "tool_result", "content": "file contents...", "is_error": false}
{"type": "stream_end", "session_id": "...", "num_turns": 1, "cost_usd": 0.004, "input_tokens": 1200, "output_tokens": 45, "is_error": false, "errors": []}
```

## Agent Tools

The agent has access to built-in tools from the SDK plus 13 custom tools:

| Category | Tools |
|----------|-------|
| Files | `Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep` |
| Web | `WebSearch`, `WebFetch` |
| Memory | `MemorySearch`, `MemoryWrite`, `MemoryAppendDaily` |
| Vault | `VaultGet`, `VaultSet` |
| Skills | `SkillCreate`, `SkillUpdate`, `SkillDelete`, `SkillList` |
| Cron | `CronAdd`, `CronList`, `CronRemove`, `CronRuns` |

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

### orion-memory

Persistent memory: markdown files on disk + SQLite FTS5 index. Text is chunked into ~400-token segments with 80-token overlap at line boundaries. `bootstrap_context()` assembles personality + user facts + knowledge + recent daily logs into the system prompt.

### orion-vault

AES-256-GCM encrypted credential storage in SQLite with audit logging.

### orion-session

Automatic session management: messages within 30 minutes continue the same session, otherwise a new one starts. Tracks token usage and cost per turn.

### orion-skills

Markdown-based skill files at `<data_dir>/skills/<name>/SKILL.md`. Skills are injected into the system prompt on every turn. The agent can create, update, and delete skills at runtime.

### orion-cron

Scheduling system supporting interval (`every_ms`), cron expressions, and one-shot (`at`) schedules. A background scheduler (30s tick) runs jobs through `OrionAgent::chat()` and records run history.

## Development

### Run Tests

```bash
cargo test
```

75 unit tests + 2 doc-tests across all crates, zero warnings.

| Crate | Tests |
|-------|-------|
| agent-sdk | 17 + 2 doc |
| orion-memory | 8 |
| orion-vault | 7 |
| orion-session | 8 |
| orion-skills | 9 |
| orion-cron | 7 |
| orion-agent | 3 |
| orion-telegram | 3 |

## License

MIT
