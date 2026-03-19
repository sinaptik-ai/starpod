# Architecture

Starpod is a Rust workspace with 12 crates, each responsible for a single concern.

```
crates/
├── agent-sdk/            Claude API client + agent loop
├── starpod-hooks/        Lifecycle hook system (events, callbacks, permissions)
├── starpod-core/         Shared types, config, error handling
├── starpod-memory/       SQLite FTS5 full-text search + markdown files
├── starpod-session/      Channel-aware session lifecycle (per-user)
├── starpod-skills/       Self-extension skill system (markdown-based)
├── starpod-cron/         Cron scheduling (interval, cron expr, one-shot)
├── starpod-agent/        Orchestrator wiring everything together
├── starpod-gateway/      Axum HTTP/WS server + embedded web UI
├── starpod-telegram/     Telegram bot interface (teloxide)
├── starpod-instances/    Remote instance management client
└── starpod/              CLI binary
```

## Blueprints vs Instances

An **agent** in Starpod has two halves: a **blueprint** (what you design) and an **instance** (what actually runs).

### Blueprint — the source of truth

A blueprint is a folder under `agents/<name>/` in your workspace. It's git-tracked and contains everything that defines *what* the agent is:

```
agents/aster/
├── agent.toml       # Config: model, provider, max_turns, channels
├── SOUL.md          # Personality and instructions
├── BOOT.md          # Startup prompt (optional)
├── HEARTBEAT.md     # Background prompt (optional)
├── BOOTSTRAP.md     # First-run prompt (optional)
└── files/           # Template files synced to the instance root
```

Think of it like a Dockerfile — it describes the agent, but nothing runs here. No databases, no memory, no user data.

### Instance — the running agent

An instance is the runtime environment created from a blueprint. It lives in `.instances/<name>/` (gitignored) and contains everything the agent accumulates while running:

```
.instances/aster/                   # Agent's filesystem sandbox
├── .starpod/                       # Internal state
│   ├── .env                        # Secrets (environment-specific)
│   ├── config/                     # ← Copied from blueprint (overwritten on rebuild)
│   │   ├── agent.toml
│   │   ├── SOUL.md
│   │   ├── BOOT.md
│   │   ├── HEARTBEAT.md
│   │   └── BOOTSTRAP.md
│   ├── skills/                     # ← Merged from blueprint (user additions preserved)
│   ├── db/                         # SQLite databases (created on first serve)
│   │   ├── memory.db
│   │   ├── session.db
│   │   └── cron.db
│   └── users/
│       ├── admin/                  # Per-user data (auto-created)
│       │   ├── USER.md
│       │   ├── MEMORY.md
│       │   └── memory/             # Daily logs
│       └── user/                   # Default non-admin user (auto-created)
│           ├── USER.md
│           ├── MEMORY.md
│           └── memory/
├── reports/                        # Agent-created files
└── ...                             # Anything the agent writes
```

### What goes where

| | Blueprint (`agents/`) | Instance (`.instances/`) |
|---|---|---|
| **Tracked in git** | Yes | No (gitignored) |
| **Contains** | Config, personality, templates | Databases, memory, user data, files |
| **Editable by** | Developer | Agent + users at runtime |
| **On `starpod dev`** | Read-only source | Created/updated from blueprint |
| **On rebuild** | Source of truth | `config/` overwritten, `db/` + `users/` preserved |

### The three ownership zones inside `.starpod/`

| Directory | Owned by | On rebuild |
|---|---|---|
| `config/` | Blueprint | **Always overwritten** — you're shipping a new version of the agent |
| `skills/` | Both | **Merged** — blueprint skills overwrite by filename, agent-created skills preserved |
| `.env` | Environment | **Not overwritten** — secrets are deployment-specific |
| `db/`, `users/` | Runtime | **Never touched** — this is the agent's accumulated state |

### How they connect

`starpod dev <agent>` copies the blueprint into an instance via `apply_blueprint()`, then serves it. Re-running `starpod dev` refreshes the config but preserves runtime data — so you can iterate on the agent's personality without losing its memory.

For standalone deployments without a workspace, `starpod build --agent <path>` creates a self-contained `.starpod/` via `build_standalone()`, ready for `starpod serve`. If a `.starpod/` already exists, the command will error — use `--force` to overwrite the blueprint files while preserving runtime data.

## Dependency Graph

```
                    ┌─────────────┐
                    │  starpod   │  CLI binary
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
       ┌────────────┐ ┌────────────┐ ┌──────────────┐
       │  gateway   │ │  telegram  │ │  starpod-agent  │
       │  (HTTP/WS) │ │   (bot)   │ │ (orchestrator)│
       └─────┬──────┘ └─────┬──────┘ └──────┬───────┘
             └───────────────┼───────────────┘
                             │
            ┌────────┬───────┼───────┬────────┬──────────┐
            ▼        ▼       ▼       ▼        ▼          ▼
        memory    vault  session  skills    cron     instances
            │        │       │       │        │          │
            └────────┴───────┼───────┴────────┴──────────┘
                             ▼
                         starpod-core
                             │
                      ┌──────┴──────┐
                  agent-sdk    starpod-hooks
```

## Data Flow

### 1. User Sends a Message

Via the web UI (WebSocket), Telegram bot, CLI (`starpod chat`), or HTTP API (`POST /api/chat`).

### 2. Channel Routing

The `starpod-agent` maps the incoming message to a **Channel** (`Main` or `Telegram`) and resolves the session:
- **Main** — explicit sessions, client provides a UUID
- **Telegram** — time-gap sessions, 6-hour inactivity timeout

Sessions are scoped per-user — each user_id gets isolated session history.

### 3. Context Assembly

The memory system bootstraps context:
- `SOUL.md` — agent personality (shared)
- `USER.md` — user info (per-user)
- `MEMORY.md` — long-term knowledge (per-user)
- Last 3 daily logs (per-user)
- All active skills

### 4. Agent Loop

The `agent-sdk` drives the agentic loop through the `LlmProvider` trait:

```
prompt → drain followups → LLM provider → tool calls → execute → feed results → repeat
```

At each iteration boundary (before calling the API), any followup messages that arrived via the `followup_rx` channel are drained and appended as user messages. This allows the agent to incorporate rapid user messages without interrupting the current loop. The behavior is configurable via `followup_mode` (`"inject"` or `"queue"`).

The provider is selected at runtime from `config.provider`:

| Provider | Struct | Default Endpoint |
|----------|--------|-----------------|
| `anthropic` | `AnthropicProvider` | `api.anthropic.com/v1/messages` |
| `openai` | `OpenAiProvider` | `api.openai.com/v1/chat/completions` |
| `gemini` | `GeminiProvider` | `generativelanguage.googleapis.com/v1beta` |
| `groq` | `OpenAiProvider` | `api.groq.com/openai/v1/chat/completions` |
| `deepseek` | `OpenAiProvider` | `api.deepseek.com/v1/chat/completions` |
| `openrouter` | `OpenAiProvider` | `openrouter.ai/api/v1/chat/completions` |
| `ollama` | `OpenAiProvider` | `localhost:11434/v1/chat/completions` |

Each provider translates between the canonical Anthropic types (`CreateMessageRequest`, `MessageResponse`, `StreamEvent`) and its own wire format internally.

The agent has access to file I/O, web search, memory, environment, file sandbox, skills, and cron tools.

**Conversation compaction**: when `input_tokens` exceeds the context budget (160k tokens), older messages are automatically summarized via a separate API call and replaced with a compact summary. The full transcript is preserved on disk. Tool-use cycles are never split.

### 5. Finalization

- Usage is recorded in the session database
- The conversation is appended to the daily log
- The response streams back to the client

## Shared State

All subsystems are wrapped in `Arc` for thread-safe sharing across async tasks:

| Component | Type | Shared By |
|-----------|------|-----------|
| Memory | `Arc<MemoryStore>` | Agent, Gateway |
| Sessions | `Arc<SessionManager>` | Agent, Gateway |
| Skills | `Arc<SkillStore>` | Agent |
| Cron | `Arc<CronStore>` | Agent, Scheduler |

SQLite connections are wrapped in `Mutex<Connection>` for safe concurrent access.

## Directory Layouts

### Workspace (development)

```
workspace/
├── starpod.toml                    # workspace defaults (git-tracked)
├── .env                            # production secrets (gitignored)
├── .env.dev                        # development overrides (gitignored)
├── skills/                         # shared skills (git-tracked)
├── agents/                         # BLUEPRINTS (git-tracked)
│   └── aster/
│       ├── agent.toml              # config + default permissions
│       ├── SOUL.md                 # personality
│       └── files/                  # template filesystem
└── .instances/                     # RUNTIME (gitignored)
    └── aster/                      # agent's filesystem root
        ├── .starpod/               # internal (like .git/)
        │   ├── .env                # secrets (from workspace .env.dev or .env)
        │   ├── config/             # blueprint-managed (overwritten on build)
        │   │   ├── agent.toml
        │   │   ├── SOUL.md
        │   │   ├── HEARTBEAT.md
        │   │   ├── BOOT.md
        │   │   └── BOOTSTRAP.md
        │   ├── skills/             # merged on build
        │   ├── db/                 # SQLite DBs (runtime)
        │   └── users/
        │       ├── admin/          # auto-created (runtime)
        │       │   ├── USER.md
        │       │   ├── MEMORY.md
        │       │   └── memory/
        │       └── user/           # auto-created (runtime)
        │           ├── USER.md
        │           ├── MEMORY.md
        │           └── memory/
        ├── reports/                # agent creates freely
        └── ...                     # full filesystem sandbox
```

### Single-agent (production)

```
/srv/aster/                         # agent's filesystem root
├── .starpod/
│   ├── .env                        # secrets
│   ├── config/                     # blueprint-managed
│   │   ├── agent.toml
│   │   ├── SOUL.md
│   │   ├── HEARTBEAT.md
│   │   ├── BOOT.md
│   │   └── BOOTSTRAP.md
│   ├── skills/                     # merged on build
│   ├── users/{admin,user}/          # runtime (auto-created)
│   └── db/                         # runtime
├── reports/                        # agent-produced files
└── ...
```

Starpod auto-detects the mode by walking up from the current directory:
- `.starpod/config/agent.toml` found (in CWD or any parent) → **SingleAgent** mode
- Inside `.instances/<name>/` with `starpod.toml` sibling → **Instance** mode
- `starpod.toml` found → **Workspace** mode

This means `starpod serve`, `starpod chat`, etc. work from any subdirectory — they walk up to find the nearest `.starpod/`.
