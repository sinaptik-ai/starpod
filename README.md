# Orion RS

A modular personal AI assistant platform built in Rust, powered by Claude. Orion provides persistent memory, encrypted credential storage, automatic session management, self-extending skills, scheduled tasks, and a gateway server supporting HTTP and WebSocket frontends.

## Architecture

```
crates/
├── agent-sdk/          Claude API client + agent loop
├── orion-core/         Shared types, config, error handling, migration runner
├── orion-memory/       SQLite FTS5 full-text search + markdown files
├── orion-vault/        AES-256-GCM encrypted credential storage
├── orion-session/      Session lifecycle + time-gap analysis
├── orion-skills/       Self-extension skill system (markdown-based)
├── orion-cron/         Cron scheduling (interval, cron expr, one-shot)
├── orion-agent/        Orchestrator wiring everything together
├── orion-gateway/      Axum HTTP/WS server
└── orion/              CLI binary
```

### Dependency Graph

```
agent-sdk              (independent)
orion-core             (independent)
orion-memory           → orion-core
orion-vault            → orion-core
orion-session          → orion-core
orion-skills           → orion-core
orion-cron             → orion-core
orion-agent            → orion-core, orion-memory, orion-session, orion-vault, orion-skills, orion-cron, agent-sdk
orion-gateway          → orion-core, orion-agent, orion-session
orion (bin)            → orion-core, orion-agent, orion-gateway, orion-skills, orion-cron
```

## Quick Start

### Prerequisites

- Rust 1.87+
- An Anthropic API key

### Build

```bash
cargo build --release
```

### Configure

Orion loads configuration from `~/.orion/config.toml`. All fields are optional with sensible defaults:

```toml
# Data directory for memory files and databases
data_dir = "~/.orion/orion_data"

# SQLite database path (default: <data_dir>/memory.db)
# db_path = "~/.orion/orion_data/memory.db"

# Gateway server bind address
server_addr = "127.0.0.1:3000"

# Claude model
model = "claude-haiku-4-5"

# Maximum agentic turns per request
max_turns = 30

# API key (recommended: use ANTHROPIC_API_KEY env var instead)
# api_key = "sk-ant-..."
```

If no config file exists, defaults are used.

### Set Your API Key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Run

```bash
# One-shot chat
orion chat "What's the capital of France?"

# Interactive REPL
orion repl

# Start the HTTP/WS gateway (also starts the cron scheduler)
orion serve
```

## CLI Reference

```
orion serve                              Start the gateway server + cron scheduler
orion chat "<message>"                   Send a one-shot message
orion repl                               Interactive REPL session

orion memory search "<query>" [-l 5]     Full-text search over memory
orion memory reindex                     Rebuild the FTS5 index

orion vault get <key>                    Retrieve a stored credential
orion vault set <key> <value>            Encrypt and store a credential
orion vault delete <key>                 Delete a credential
orion vault list                         List all stored keys

orion sessions list [-l 10]              List recent sessions

orion skills list                        List all skills
orion skills show <name>                 Show a skill's content
orion skills create <name> -c "..."      Create a skill from inline content
orion skills create <name> -f file.md    Create a skill from a file
orion skills delete <name>               Delete a skill

orion cron list                          List all cron jobs
orion cron remove <name>                 Remove a cron job
orion cron runs <name> [-l 10]           Show recent runs for a job
```

## Gateway API

The gateway server (`orion serve`) exposes both HTTP and WebSocket endpoints.

### Authentication

Set the `ORION_API_KEY` environment variable to require API key authentication. Clients must include the key in the `X-API-Key` header. If unset, all requests are allowed.

### HTTP Endpoints

#### `POST /api/chat`

Send a chat message and receive a response.

```bash
curl -X POST http://localhost:3000/api/chat \
  -H "Content-Type: application/json" \
  -H "X-API-Key: your-key" \
  -d '{"text": "Hello!", "user_id": "alice", "channel_id": "web"}'
```

**Request:**

```json
{
  "text": "Hello!",
  "user_id": "alice",
  "channel_id": "web"
}
```

**Response:**

```json
{
  "text": "Hi there! How can I help you today?",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "usage": {
    "input_tokens": 1200,
    "output_tokens": 45,
    "cache_read_tokens": 800,
    "cache_write_tokens": 0,
    "cost_usd": 0.0042
  }
}
```

#### `GET /api/sessions?limit=20`

List recent sessions.

```json
[
  {
    "id": "550e8400-...",
    "created_at": "2026-03-13T10:00:00+00:00",
    "last_message_at": "2026-03-13T10:15:00+00:00",
    "is_closed": false,
    "summary": null,
    "message_count": 5
  }
]
```

#### `GET /api/sessions/:id`

Get a specific session by ID.

#### `GET /api/memory/search?q=rust+programming&limit=10`

Full-text search over memory files.

```json
[
  {
    "source": "knowledge/rust.md",
    "text": "Rust is a systems programming language...",
    "line_start": 1,
    "line_end": 5
  }
]
```

#### `POST /api/memory/reindex`

Rebuild the full-text search index from all markdown files.

#### `GET /api/health`

Health check.

```json
{"status": "ok", "version": "0.1.0"}
```

### WebSocket

Connect to `ws://localhost:3000/ws` for bidirectional messaging.

**Client sends:**

```json
{"type": "message", "text": "Hello!", "user_id": "alice", "channel_id": "web"}
```

**Server responds:**

```json
{"type": "response", "text": "Hi there!", "session_id": "550e8400-..."}
```

Or on error:

```json
{"type": "error", "message": "Chat error: ..."}
```

## Crate Details

### agent-sdk

Rust port of the Claude Agent SDK. Provides the core `query()` function that drives the agentic loop: prompt -> Claude API call -> tool execution -> feed results back -> repeat.

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

**Built-in tools:** Read, Write, Edit, Bash, Glob, Grep

**Custom tools:** Register via `Options::builder().external_tool_handler(handler).custom_tools(defs).build()`. The external handler is called before the built-in executor; return `Some(ToolResult)` to handle the call, or `None` to fall through to built-ins.

**Hooks:** 18 lifecycle events (PreToolUse, PostToolUse, SessionStart, etc.) with async callbacks, regex matchers, and configurable timeouts.

### orion-core

Shared foundation: `OrionConfig` (loaded from TOML), `OrionError` (unified error enum), request/response types (`ChatMessage`, `ChatResponse`, `ChatUsage`), and the **migration runner**.

#### Migration System

Each crate owns its schema via versioned migrations. The `run_migrations(conn, namespace, migrations)` function:
- Creates a `_migrations` table for tracking applied versions
- Runs each pending migration in its own transaction
- Scopes versions by namespace so multiple crates can share a database

```rust
use orion_core::{Migration, run_migrations};

let migrations = &[
    Migration { version: 1, name: "create_items", sql: "CREATE TABLE ..." },
    Migration { version: 2, name: "add_column", sql: "ALTER TABLE ..." },
];

run_migrations(&conn, "my_crate", migrations)?;
```

### orion-memory

Persistent memory system combining markdown files on disk with SQLite FTS5 full-text search.

**Data directory layout** (`~/.orion/orion_data/`):

```
SOUL.md              Personality definition (seeded on first run)
USER.md              Learned user facts
MEMORY.md            Long-term knowledge
memory/YYYY-MM-DD.md Daily conversation logs
knowledge/*.md       Knowledge base files
memory.db            SQLite database (FTS5 index)
```

On initialization, default `SOUL.md`, `USER.md`, and `MEMORY.md` files are created and all markdown files are indexed into the FTS5 table.

**Chunking:** Text is split into ~400-token chunks with 80-token overlap at line boundaries for accurate search results.

**Bootstrap context:** `bootstrap_context()` assembles SOUL + USER + MEMORY + the last 3 daily logs (capped at 20K chars each) into a single string used as the system prompt prefix.

### orion-vault

Encrypted credential storage using AES-256-GCM. Each value is encrypted with a random 96-bit nonce and stored alongside it in SQLite. All operations (get, set, delete) are recorded in an audit log.

The vault master key is derived from the Anthropic API key if available, otherwise a default key is used. For production use, integrate a proper KDF (Argon2, HKDF).

### orion-session

Automatic session management with time-gap heuristics:

| Gap since last message | Action |
|---|---|
| < 30 minutes | Continue existing session |
| >= 30 minutes | Start new session |

Each session tracks creation time, last message time, message count, closure status, summary, and per-turn token usage with cost.

### orion-skills

Self-extension system. Skills are markdown files stored at `<data_dir>/skills/<name>/SKILL.md` that augment the agent's capabilities at runtime.

**How it works:**
1. The agent (or user via CLI) creates a skill with `SkillCreate`
2. On each chat turn, all skills are loaded and injected into the system prompt
3. Skills can be updated or deleted at any time

**Agent tools:** `SkillCreate`, `SkillUpdate`, `SkillDelete`, `SkillList`

Skills are pure filesystem — no database required. The directory-per-skill layout (`skills/<name>/SKILL.md`) allows future expansion (config files, examples per skill).

### orion-cron

Built-in scheduling system for recurring and one-shot tasks.

**Schedule types:**

| Type | Config | Example |
|---|---|---|
| Interval | `every_ms` | Every 5 minutes (300000) |
| Cron | `expr` | `"0 0 9 * * *"` (daily at 9am) |
| One-shot | `at` | ISO 8601 timestamp |

**How it works:**
1. The agent creates a job via `CronAdd` with a name, prompt, and schedule
2. A background scheduler (30-second tick) polls for due jobs
3. When a job fires, its prompt is sent through `OrionAgent::chat()`
4. Run history is recorded with status and result summary
5. One-shot jobs can auto-delete with `delete_after_run: true`

**Agent tools:** `CronAdd`, `CronList`, `CronRemove`, `CronRuns`

The scheduler starts automatically when the gateway server runs (`orion serve`).

### orion-agent

The orchestrator. `OrionAgent::chat()` executes the full pipeline:

1. **Resolve session** — time-gap analysis to continue or create
2. **Build system prompt** — bootstrap context + active skills + date/time + tool instructions
3. **Configure tools** — built-in (Read, Bash, Glob, Grep) + 13 custom tools
4. **Run agent loop** — `agent_sdk::query()` with `BypassPermissions` mode
5. **Record usage** — persist token counts and cost to session
6. **Append daily log** — summarize the exchange in today's log file

**Custom tools (13):**

| Category | Tools |
|---|---|
| Memory | `MemorySearch`, `MemoryWrite`, `MemoryAppendDaily` |
| Vault | `VaultGet`, `VaultSet` |
| Skills | `SkillCreate`, `SkillUpdate`, `SkillDelete`, `SkillList` |
| Cron | `CronAdd`, `CronList`, `CronRemove`, `CronRuns` |

All tool handlers go through a shared `ToolContext` struct that holds `Arc` references to each subsystem.

### orion-gateway

Axum-based HTTP and WebSocket server. Wraps `OrionAgent` in shared state (`Arc<AppState>`) and exposes it through REST endpoints and a WebSocket connection. Starts the cron scheduler as a background task on boot.

### orion (CLI)

Command-line interface with clap. The `repl` command uses rustyline for readline support (history, Ctrl+C/Ctrl+D handling). Rich colored output with per-tool icons for streaming display.

## Development

### Run Tests

```bash
cargo test
```

67 unit tests + 2 doc-tests across all crates.

### Test Breakdown

| Crate | Tests |
|---|---|
| agent-sdk | 17 unit + 2 doc |
| orion-core | 4 (migration runner) |
| orion-memory | 8 |
| orion-vault | 7 |
| orion-session | 8 |
| orion-skills | 9 |
| orion-cron | 7 |
| orion-agent | 3 |

### Project Structure

```
crates/
├── agent-sdk/
│   ├── src/
│   │   ├── lib.rs           Public API re-exports
│   │   ├── client.rs        Anthropic Messages API client
│   │   ├── query.rs         Agent loop + Query stream
│   │   ├── options.rs       Options builder + types
│   │   ├── permissions.rs   Permission evaluation
│   │   ├── error.rs         AgentError enum
│   │   ├── hooks/           Hook system (18 event types)
│   │   ├── mcp/             MCP server configs
│   │   ├── session/         JSONL session persistence
│   │   ├── tools/           Executor + JSON schema definitions
│   │   └── types/           Messages, tools, agent, permissions
│   └── examples/            5 examples
├── orion-core/
│   └── src/
│       ├── config.rs        OrionConfig from TOML
│       ├── error.rs         OrionError enum
│       ├── migrate.rs       Migration runner (namespace-scoped, versioned)
│       └── types.rs         ChatMessage, ChatResponse
├── orion-memory/
│   └── src/
│       ├── store.rs         MemoryStore (search, write, reindex)
│       ├── indexer.rs       Text chunking + FTS5 indexing
│       ├── schema.rs        Versioned migrations
│       └── defaults.rs      Default SOUL/USER/MEMORY content
├── orion-vault/
│   └── src/
│       ├── lib.rs           Vault (AES-256-GCM encrypt/decrypt)
│       └── schema.rs        Versioned migrations
├── orion-session/
│   └── src/
│       ├── lib.rs           SessionManager (time-gap, usage)
│       └── schema.rs        Versioned migrations
├── orion-skills/
│   └── src/
│       └── lib.rs           SkillStore (create, update, delete, bootstrap)
├── orion-cron/
│   └── src/
│       ├── lib.rs           Re-exports
│       ├── store.rs         CronStore (CRUD, due jobs, run history)
│       ├── scheduler.rs     Background scheduler loop
│       ├── schema.rs        Versioned migrations
│       └── types.rs         CronJob, CronRun, Schedule, RunStatus
├── orion-agent/
│   └── src/
│       ├── lib.rs           OrionAgent orchestrator
│       └── tools.rs         13 custom tool definitions + ToolContext handler
├── orion-gateway/
│   └── src/
│       ├── lib.rs           Server setup + AppState + scheduler start
│       ├── routes.rs        HTTP API endpoints
│       └── ws.rs            WebSocket handler
└── orion/
    └── src/
        └── main.rs          CLI with clap + REPL + skills/cron subcommands
```

## License

MIT
