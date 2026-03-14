# Architecture

Orion is a Rust workspace with 11 crates, each responsible for a single concern.

```
crates/
├── agent-sdk/          Claude API client + agent loop
├── orion-core/         Shared types, config, error handling
├── orion-memory/       SQLite FTS5 full-text search + markdown files
├── orion-vault/        AES-256-GCM encrypted credential storage
├── orion-session/      Channel-aware session lifecycle
├── orion-skills/       Self-extension skill system (markdown-based)
├── orion-cron/         Cron scheduling (interval, cron expr, one-shot)
├── orion-agent/        Orchestrator wiring everything together
├── orion-gateway/      Axum HTTP/WS server + embedded web UI
├── orion-telegram/     Telegram bot interface (teloxide)
└── orion/              CLI binary
```

## Dependency Graph

```
                    ┌─────────────┐
                    │    orion    │  CLI binary
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
       ┌────────────┐ ┌────────────┐ ┌──────────────┐
       │  gateway   │ │  telegram  │ │  orion-agent  │
       │  (HTTP/WS) │ │   (bot)   │ │ (orchestrator)│
       └─────┬──────┘ └─────┬──────┘ └──────┬───────┘
             └───────────────┼───────────────┘
                             │
            ┌────────┬───────┼───────┬────────┐
            ▼        ▼       ▼       ▼        ▼
        memory    vault   session  skills    cron
            │        │       │       │        │
            └────────┴───────┼───────┴────────┘
                             ▼
                         orion-core
                             │
                         agent-sdk
```

## Data Flow

### 1. User Sends a Message

Via the web UI (WebSocket), Telegram bot, CLI (`orion agent chat`), or HTTP API (`POST /api/chat`).

### 2. Channel Routing

The `orion-agent` maps the incoming message to a **Channel** (`Main` or `Telegram`) and resolves the session:
- **Main** — explicit sessions, client provides a UUID
- **Telegram** — time-gap sessions, 6-hour inactivity timeout

### 3. Context Assembly

The memory system bootstraps context:
- `SOUL.md` — agent personality
- `USER.md` — user info
- `MEMORY.md` — long-term knowledge
- Last 3 daily logs
- All active skills

### 4. Agent Loop

The `agent-sdk` drives the agentic loop:

```
prompt → Claude API → tool calls → execute → feed results → repeat
```

The agent has access to file I/O, web search, memory, vault, skills, and cron tools.

### 5. Finalization

- Usage is recorded in the session database
- The conversation is appended to the daily log
- The response streams back to the client

## Shared State

All subsystems are wrapped in `Arc` for thread-safe sharing across async tasks:

| Component | Type | Shared By |
|-----------|------|-----------|
| Memory | `Arc<MemoryStore>` | Agent, Gateway |
| Vault | `Arc<Vault>` | Agent |
| Sessions | `Arc<SessionManager>` | Agent, Gateway |
| Skills | `Arc<SkillStore>` | Agent |
| Cron | `Arc<CronStore>` | Agent, Scheduler |

SQLite connections use `Mutex<Connection>` for safe concurrent access.

## Project Directory

```
.orion/
├── config.toml          Project configuration
└── data/
    ├── SOUL.md          Agent personality
    ├── USER.md          User information
    ├── MEMORY.md        General knowledge
    ├── memory/          Daily logs (YYYY-MM-DD.md)
    ├── knowledge/       Knowledge base documents
    ├── skills/          Skill definitions
    │   └── <name>/
    │       └── SKILL.md
    ├── downloads/       Uploaded file attachments
    │   └── <session_id>/
    └── memory.db        SQLite (FTS5 + sessions + vault + cron)
```

Orion walks up from the current directory to find the nearest `.orion/` folder — just like Git finds `.git/`.
