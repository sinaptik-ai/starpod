# Architecture

Starpod is a Rust workspace with 11 crates, each responsible for a single concern.

```
crates/
├── agent-sdk/          Claude API client + agent loop
├── starpod-core/         Shared types, config, error handling, instance management
├── starpod-memory/       SQLite FTS5 full-text search + markdown files
├── starpod-session/      Channel-aware session lifecycle (per-user)
├── starpod-skills/       Self-extension skill system (markdown-based)
├── starpod-cron/         Cron scheduling (interval, cron expr, one-shot)
├── starpod-agent/        Orchestrator wiring everything together
├── starpod-gateway/      Axum HTTP/WS server + embedded web UI
├── starpod-telegram/     Telegram bot interface (teloxide)
└── starpod/              CLI binary
```

## Blueprint / Instance Separation

Starpod separates **blueprints** (git-tracked agent definitions) from **instances** (runtime state):

- **Blueprint** (`agents/<name>/`) — config, personality, secrets templates. Committed to git.
- **Instance** (`.instances/<name>/`) — databases, memory, user data, agent-created files. Gitignored.

`starpod dev <agent>` copies the blueprint into an instance via `apply_blueprint()`, then serves it.

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
            ┌────────┬───────┼───────┬────────┐
            ▼        ▼       ▼       ▼        ▼
        memory    env     session  skills    cron
            │        │       │       │        │
            └────────┴───────┼───────┴────────┘
                             ▼
                         starpod-core
                             │
                         agent-sdk
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

SQLite connections use connection pools for safe concurrent access.

## Directory Layouts

### Workspace (development)

```
workspace/
├── starpod.toml                    # workspace defaults (git-tracked)
├── skills/                         # shared skills (git-tracked)
├── agents/                         # BLUEPRINTS (git-tracked)
│   └── aster/
│       ├── agent.toml              # config + default permissions
│       ├── SOUL.md                 # personality
│       ├── .env                    # prod secrets
│       ├── .env.dev                # dev secrets override
│       ├── users/                  # per-user permission templates
│       └── files/                  # template filesystem
└── .instances/                     # RUNTIME (gitignored)
    └── aster/                      # agent's filesystem root
        ├── .starpod/               # internal (like .git/)
        │   ├── agent.toml          # copied from blueprint
        │   ├── SOUL.md             # copied from blueprint
        │   ├── .env                # ONE file (from .env.dev or .env)
        │   ├── users/
        │   │   └── admin/          # auto-created
        │   │       ├── USER.md
        │   │       ├── MEMORY.md
        │   │       └── memory/
        │   └── data/               # SQLite DBs
        ├── reports/                # agent creates freely
        └── ...                     # full filesystem sandbox
```

### Single-agent (production)

```
/srv/aster/                         # agent's filesystem root
├── .starpod/
│   ├── agent.toml
│   ├── SOUL.md
│   ├── .env
│   ├── users/admin/
│   └── data/
├── reports/                        # agent-produced files
└── ...
```

Starpod auto-detects the mode by walking up from the current directory:
- `.starpod/agent.toml` found → **SingleAgent** mode
- Inside `.instances/<name>/` with `starpod.toml` sibling → **Instance** mode
- `starpod.toml` found → **Workspace** mode
