<p align="center">
  <img src="docs/public/logo.svg" alt="Starpod" width="80" height="80">
</p>

<h1 align="center">Starpod</h1>

<p align="center">
  <strong>Kubernetes for AI agents. Minus the pain.</strong>
</p>

<p align="center">
  <a href="https://starpod.sh">Website</a> · <a href="https://docs.starpod.sh">Docs</a> · <a href="https://console.starpod.sh">Console</a> · <a href="https://discord.com/invite/KYKj9F2FRH">Discord</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-C0C0C0.svg?style=flat-square" alt="MIT License"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.87+-C0C0C0.svg?style=flat-square&logo=rust&logoColor=white" alt="Rust 1.87+"></a>
  <img src="https://img.shields.io/badge/tests-1316-C0C0C0.svg?style=flat-square" alt="1316 tests">
  <img src="https://img.shields.io/badge/crates-16-C0C0C0.svg?style=flat-square" alt="16 crates">
  <a href="https://discord.com/invite/KYKj9F2FRH"><img src="https://img.shields.io/discord/1102146545580785686?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square" alt="Discord"></a>
</p>

---

Starpod is an open-source AI agent runtime built in Rust. Define an agent once — skills, config, tools — then deploy isolated instances for every user, team, or client. Each instance gets its own memory, vault, filesystem, and sessions. No cross-contamination. Scale from 1 to 10,000.

```
          Starpod UI · Telegram · Slack · Email · API
                          │
                          ▼
               ┌─────────────────────┐
               │       AGENT         │  Define once
               │  skills · config    │
               │  tools · personality│
               └────────┬────────────┘
                        │  starpod deploy
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
    ┌───────────┐ ┌───────────┐ ┌───────────┐
    │ Instance A│ │ Instance B│ │ Instance C│  Isolated per tenant
    │ memory    │ │ memory    │ │ memory    │
    │ vault     │ │ vault     │ │ vault     │
    │ files     │ │ files     │ │ files     │
    └───────────┘ └───────────┘ └───────────┘
```

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
# Initialize a workspace
starpod init

# Create an agent
starpod agent new my-agent

# Run locally
starpod dev my-agent
```

```
  Starpod is running

  Frontend  http://127.0.0.1:3000
  API       http://127.0.0.1:3000/api
  WS        ws://127.0.0.1:3000/ws
  Telegram  connected
  Model     claude-haiku-4-5
```

Or deploy to [Starpod Console](https://console.starpod.sh) — one-click deploy, real-time logs, secrets vault, usage analytics. Managed cloud or your own (AWS, GCP, Azure).

## Highlights

- **One agent, infinite instances** — define skills and config once, deploy isolated instances per user, team, or client. Each gets its own memory, vault, and filesystem.
- **Multi-channel** — Starpod UI, Telegram, Slack, Email, HTTP API, WebSocket streaming. Same agent, every surface.
- **Persistent memory** — markdown files + SQLite FTS5. Per-user memory spaces. The agent remembers across sessions.
- **Self-extending skills** — the agent creates, edits, and deletes its own skill files at runtime. `/skill` and it executes.
- **Encrypted vault** — AES-256-GCM credential storage per instance. API keys never touch disk in plaintext.
- **Cron scheduling** — interval, cron expressions, one-shot. Runs through the full agent loop, records history.
- **Channel-aware sessions** — explicit sessions for web/API, time-gap sessions for Telegram. Per-user scoping.
- **Streaming** — real-time text deltas and tool-use events over WebSocket.
- **Built in Rust** — 16 crates, 1,316 tests, zero warnings.

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

## Instance layout

Each instance gets its own isolated `.starpod/` directory:

```
.starpod/
├── .env                    Secrets (never overwritten by deploy)
├── config/
│   ├── agent.toml          Agent configuration
│   ├── SOUL.md             Personality
│   └── BOOT.md             Bootstrap prompt
├── skills/                 Skill files (merged on build)
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

**Workspace** (`starpod.toml`) — shared defaults for all agents:

```toml
provider = "anthropic"
model = "claude-haiku-4-5"
max_turns = 30
```

**Agent** (`agents/<name>/agent.toml`) — per-agent overrides:

```toml
agent_name = "Aster"
model = "claude-haiku-4-5"

[channels.telegram]
allowed_users = [123456789]
```

**Secrets** in `.env` — never in config:

```
ANTHROPIC_API_KEY=sk-ant-...
TELEGRAM_BOT_TOKEN=123456789:ABC...
```

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
2. Add `TELEGRAM_BOT_TOKEN=...` to `.env`
3. Add `allowed_users` to `agent.toml`
4. `starpod dev <agent>` — look for `Telegram connected`

The bot shares the same agent as web and API. Typing indicators, smart message splitting, HTML rendering with plain-text fallback.

## CLI

```
starpod init                          Initialize workspace
starpod agent new <name>              Create agent
starpod agent list                    List agents
starpod dev <agent>                   Dev server (blueprint + serve)
starpod serve                         Production server
starpod chat -a <agent> "..."         One-shot message
starpod repl -a <agent>               Interactive REPL

starpod memory search "..." [-l 5]    Search memory
starpod memory reindex                Rebuild FTS5 index
starpod sessions list [-l 10]         List sessions

starpod skill list                    List skills
starpod skill new <name> -d "..."     Create skill
starpod skill delete <name>           Delete skill

starpod cron list                     List cron jobs
starpod cron remove <name>            Remove cron job
starpod cron runs <name>              Show run history
starpod cron run <name>               Trigger immediately
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
