# Starpod RS

A local-first personal AI assistant platform built in Rust, powered by Claude. Each project gets its own `.starpod/` directory with config, memory, credentials, skills, and scheduled tasks — no global state.

## Architecture

```
crates/
├── agent-sdk/          Claude API client + agent loop
├── starpod-core/         Shared types, config, error handling
├── starpod-memory/       SQLite FTS5 full-text search + markdown files
├── starpod-vault/        AES-256-GCM encrypted credential storage
├── starpod-session/      Channel-aware session lifecycle
├── starpod-skills/       Self-extension skill system (markdown-based)
├── starpod-cron/         Cron scheduling (interval, cron expr, one-shot)
├── starpod-agent/        Orchestrator wiring everything together
├── starpod-gateway/      Axum HTTP/WS server + embedded web UI
├── starpod-telegram/     Telegram bot interface (teloxide)
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

To skip the wizard and use defaults (Anthropic / `claude-sonnet-4-6`):

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
# Start the server (web UI + API + WebSocket + optional Telegram bot)
starpod serve -a my-agent

# One-shot chat
starpod chat -a my-agent "What files are in this directory?"

# Interactive REPL
starpod repl -a my-agent
```

When you run `starpod agent serve`, you'll see:

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

Edit `.starpod/config.toml`:

```toml
# Active LLM provider
provider = "anthropic"
model = "claude-haiku-4-5"
max_turns = 30
server_addr = "127.0.0.1:3000"

# Reasoning effort for extended thinking: "low", "medium", "high"
# reasoning_effort = "medium"

[identity]
# name = "Aster"                  # Agent's display name
# emoji = "🤖"                    # Agent's emoji/avatar
# soul = ""                       # Freeform personality injected into system prompt

[user]
# name = "Your Name"
# timezone = "America/New_York"

[providers.anthropic]
# api_key = "sk-ant-..."          # Or set ANTHROPIC_API_KEY env var

[providers.openai]
# api_key = "sk-..."              # Not yet implemented
# models = ["gpt-5-4"]

[telegram]
# bot_token = "123456:ABC..."     # Or set TELEGRAM_BOT_TOKEN env var
# allowed_users = [123456789, "alice"]  # User IDs or usernames (without @)
# stream_mode = "off"             # "edit_in_place" or "off"
# edit_throttle_ms = 300
```

Config is per-project. Starpod walks up from the current directory to find the nearest `.starpod/` folder (like git finds `.git/`).

## CLI Reference

```
starpod init                              Initialize workspace (interactive wizard)
starpod init --default                    Initialize with defaults (no wizard)
starpod agent new <name>                  Create a new agent
starpod agent list                        List agents in workspace
starpod serve -a <agent>                  Start server (web UI + API + WS + Telegram)
starpod chat -a <agent> "<message>"       Send a one-shot message
starpod repl -a <agent>                   Interactive REPL session

starpod instance create                   Create a remote instance (coming soon)
starpod instance list                     List running instances
starpod instance kill <id>                Kill an instance
starpod instance pause <id>               Pause an instance
starpod instance restart <id>             Restart an instance

starpod agent memory search "<query>" [-l 5]    Full-text search over memory
starpod agent memory reindex                    Rebuild the FTS5 index

starpod agent vault get <key>                   Retrieve a stored credential
starpod agent vault set <key> <value>           Encrypt and store a credential
starpod agent vault delete <key>                Delete a credential
starpod agent vault list                        List all stored keys

starpod agent sessions list [-l 10]             List recent sessions

starpod agent skills list                       List all skills
starpod agent skills show <name>                Show a skill's content
starpod agent skills create <name> -c "..."     Create a skill from inline content
starpod agent skills create <name> -f file.md   Create a skill from a file
starpod agent skills delete <name>              Delete a skill

starpod agent cron list                         List all cron jobs
starpod agent cron remove <name>                Remove a cron job
starpod agent cron runs <name> [-l 10]          Show recent runs for a job
```

## Web UI

The embedded web UI is served at `http://localhost:3000/` when running `starpod agent serve`.

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

2. **Add the token to your project**

   Either add it to `.starpod/config.toml`:
   ```toml
   [telegram]
   bot_token = "123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
   ```

   Or set it as an environment variable:
   ```bash
   export TELEGRAM_BOT_TOKEN="123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
   ```

   **Tip:** `starpod agent init` can set this up for you during the interactive wizard.

3. **Restrict access (recommended)**

   Send `/start` to your bot — it will reply with your user ID and username. Add either to `.starpod/config.toml`:
   ```toml
   [telegram]
   allowed_users = [123456789]
   ```
   You can mix user IDs and usernames (without `@`): `allowed_users = [123456789, "alice", 987654321]`

   The bot won't respond to anyone until you add at least one entry. `/start` is the only command that works without being whitelisted (so you can discover your ID and username).

4. **Start the server**
   ```bash
   starpod agent serve
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
- Sends as MarkdownV2, falls back to plain text on parse failure

## Gateway API

### Authentication

Set the `STARPOD_API_KEY` environment variable to require API key auth. Clients include it in the `X-API-Key` header (HTTP) or `?token=` query param (WebSocket).

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
{"type": "message", "text": "Hello!", "channel_id": "main", "channel_session_key": "conv-uuid-here"}
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

### starpod-memory

Persistent memory: markdown files on disk + SQLite FTS5 index. Text is chunked into ~400-token segments with 80-token overlap at line boundaries. `bootstrap_context()` assembles personality + user facts + knowledge + recent daily logs into the system prompt.

### starpod-vault

AES-256-GCM encrypted credential storage in SQLite with audit logging.

### starpod-session

Channel-aware session management with per-channel strategies:

- **`main`** (web, REPL, CLI): Explicit sessions — the client provides a `channel_session_key` (e.g. a conversation UUID) and the session continues until closed. Multiple concurrent sessions are supported.
- **`telegram`**: Time-gap sessions — messages within 6 hours continue the same session (keyed by chat ID), otherwise a new session starts and the old one is auto-closed.

Tracks token usage and cost per turn. The scheduler creates standalone `main` sessions (one per cron run) and delivers results via the configured notification channel.

### starpod-skills

Markdown-based skill files at `<data_dir>/skills/<name>/SKILL.md`. Skills are injected into the system prompt on every turn. The agent can create, update, and delete skills at runtime.

### starpod-cron

Scheduling system supporting interval (`every_ms`), cron expressions, and one-shot (`at`) schedules. Cron expressions are evaluated in the user's local timezone when `[user] timezone` is configured (auto-detected during `starpod agent init`). A background scheduler (30s tick) runs jobs through `StarpodAgent::chat()` and records run history.

## Development

### Run Tests

```bash
cargo test
```

70 unit tests + 2 doc-tests across all crates, zero warnings.

| Crate | Tests |
|-------|-------|
| agent-sdk | 17 + 2 doc |
| starpod-memory | 8 |
| starpod-vault | 7 |
| starpod-session | 11 |
| starpod-skills | 9 |
| starpod-cron | 11 |
| starpod-agent | 3 |
| starpod-telegram | 4 |

## License

MIT
