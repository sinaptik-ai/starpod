<p align="center">
  <img src="docs/public/logo.svg" alt="Starpod" width="80" height="80">
</p>

<h1 align="center">Starpod</h1>

<p align="center">
  <strong>Personal AI agents. Built in Rust.</strong>
</p>

<p align="center">
  <a href="https://starpod.sh">Website</a> · <a href="https://docs.starpod.sh">Docs</a> · <a href="https://console.starpod.sh">Console</a> · <a href="https://discord.com/invite/KYKj9F2FRH">Discord</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-C0C0C0.svg?style=flat-square" alt="MIT License"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.87+-C0C0C0.svg?style=flat-square&logo=rust&logoColor=white" alt="Rust 1.87+"></a>
  <img src="https://img.shields.io/badge/tests-1362-C0C0C0.svg?style=flat-square" alt="1362 tests">
  <img src="https://img.shields.io/badge/crates-16-C0C0C0.svg?style=flat-square" alt="16 crates">
  <a href="https://discord.com/invite/KYKj9F2FRH"><img src="https://img.shields.io/discord/1102146545580785686?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square" alt="Discord"></a>
</p>

---

Starpod is an open-source AI agent runtime built in Rust. Bootstrap an agent in any directory with `starpod init`, then run it locally or deploy to the cloud. Each agent gets its own memory, vault, filesystem, and sessions — all self-contained in a `.starpod/` directory.

## Install

```bash
cargo install starpod
```

Or from source:

```bash
git clone https://github.com/sinaptik-ai/starpod.git
cd starpod
cargo install --path crates/starpod --locked
```

## Quick start

```bash
# Initialize an agent
starpod init --name "Jarvis" --model anthropic/claude-haiku-4-5

# Seed your API key into the vault
starpod init --env ANTHROPIC_API_KEY=sk-ant-...

# Start the dev server (opens browser)
starpod dev
```

```
  ╭──────────────────────────────────────────╮
  │      Jarvis  ·  AI Assistant             │
  ╰──────────────────────────────────────────╯

  Server 127.0.0.1:3000
  API Key sp-abc123...
```

Or use the terminal:

```bash
starpod chat "What files are in this directory?"
starpod repl
```

## Highlights

- **Simple setup** — `starpod init` bootstraps everything. No workspace files, no blueprints, no separate instance management.
- **Multi-channel** — Web UI, Telegram, CLI, HTTP API, WebSocket streaming. Same agent, every surface.
- **Persistent memory** — markdown files + SQLite FTS5. Per-user memory spaces. The agent remembers across sessions.
- **Self-extending skills** — the agent creates, edits, and deletes its own skill files at runtime. `/skill` and it executes.
- **Encrypted vault** — AES-256-GCM credential storage. API keys never touch disk in plaintext. No `.env` files — vault only.
- **Cron scheduling** — interval, cron expressions, one-shot. Runs through the full agent loop, records history.
- **Channel-aware sessions** — explicit sessions for web/API, time-gap sessions for Telegram. Per-user scoping.
- **Streaming** — real-time text deltas and tool-use events over WebSocket.
- **Built in Rust** — 16 crates, 1,362 tests, zero warnings.

## Architecture

```
crates/
├── agent-sdk/            Claude API client + agent loop
├── starpod-hooks/        Lifecycle hooks, events, permissions
├── starpod-core/         Config, shared types, errors
├── starpod-db/           Unified SQLite (core.db)
├── starpod-memory/       FTS5 full-text search + per-user memory
├── starpod-vault/        AES-256-GCM encrypted credentials
├── starpod-session/      Channel-aware session lifecycle
├── starpod-skills/       Markdown-based self-extension
├── starpod-cron/         Scheduling (interval, cron, one-shot)
├── starpod-agent/        Orchestrator wiring everything together
├── starpod-gateway/      Axum HTTP/WS server + embedded web UI
├── starpod-telegram/     Telegram bot (teloxide)
├── starpod-instances/    Remote instance management
└── starpod/              CLI binary
```

## Agent layout

Each agent is self-contained in a `.starpod/` directory:

```
.starpod/
├── config/
│   ├── agent.toml          Agent configuration
│   ├── SOUL.md             Personality
│   ├── HEARTBEAT.md        Periodic self-reflection
│   ├── BOOT.md             Boot instructions
│   ├── BOOTSTRAP.md        First-run instructions
│   └── frontend.toml       Web UI config
├── skills/                 Skill files
├── db/
│   ├── core.db             Sessions + cron + auth
│   ├── memory.db           FTS5 + vectors
│   └── vault.db            Encrypted credentials
└── users/<id>/
    ├── USER.md             User profile
    ├── MEMORY.md           Memory index
    └── memory/             Daily logs
```

## Configuration

All configuration lives in a single `agent.toml`:

```toml
agent_name = "Aster"
models = ["anthropic/claude-haiku-4-5"]
max_turns = 30
server_addr = "127.0.0.1:3000"

[channels.telegram]
allowed_users = [123456789]
```

**Secrets** live in the vault (`vault.db`), seeded via `starpod init --env KEY=VAL` or the web UI Settings page. No `.env` files.

**Personality** in `SOUL.md` — not in config.

## Agent tools

| Category | Tools |
|----------|-------|
| Built-in | `Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep` |
| Memory | `MemorySearch`, `MemoryWrite`, `MemoryAppendDaily` |
| Environment | `EnvGet` |
| File sandbox | `FileRead`, `FileWrite`, `FileList`, `FileDelete` |
| Skills | `SkillActivate`, `SkillCreate`, `SkillUpdate`, `SkillDelete`, `SkillList` |
| Cron | `CronAdd`, `CronList`, `CronRemove`, `CronRuns`, `CronRun`, `CronUpdate`, `HeartbeatWake` |

## API

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/chat` | Send a message |
| `GET` | `/api/sessions` | List sessions |
| `GET` | `/api/sessions/:id` | Get session |
| `GET` | `/api/sessions/:id/messages` | Session messages |
| `GET` | `/api/memory/search?q=...` | Full-text search |
| `POST` | `/api/memory/reindex` | Rebuild FTS index |
| `GET` | `/api/health` | Health check |

WebSocket at `ws://localhost:3000/ws`. Auth via `X-API-Key` header or `?token=` param.

```json
← {"type": "message", "text": "Hello"}
→ {"type": "stream_start", "session_id": "..."}
→ {"type": "text_delta", "text": "Hi "}
→ {"type": "text_delta", "text": "there!"}
→ {"type": "tool_use", "name": "Read", "input": {...}}
→ {"type": "tool_result", "content": "..."}
→ {"type": "stream_end", "cost_usd": 0.004}
```

## Telegram

1. Create a bot with [@BotFather](https://t.me/BotFather)
2. Store the token in the vault (via `starpod init --env TELEGRAM_BOT_TOKEN=...` or Settings UI)
3. Add `allowed_users` to `agent.toml`
4. `starpod dev` — look for `Telegram connected`

## CLI

```
starpod init [--name N] [--model M] [--env K=V]   Initialize agent
starpod dev [--port P]                             Dev server (opens browser)
starpod serve                                      Production server
starpod deploy                                     Deploy to remote (coming soon)
starpod chat "message"                             One-shot message
starpod repl                                       Interactive REPL
starpod auth login|logout|status                   Platform authentication
```

## Development

```bash
cargo test --workspace
```

## Community

- [Discord](https://discord.com/invite/KYKj9F2FRH) — questions, feedback, show & tell
- [GitHub Issues](https://github.com/sinaptik-ai/starpod/issues) — bug reports and feature requests
- [Docs](https://docs.starpod.sh) — full documentation

## License

MIT
