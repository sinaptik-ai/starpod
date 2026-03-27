# Architecture

Starpod is a Rust workspace with 16 crates, each responsible for a single concern.

```
crates/
├── agent-sdk/            Claude API client + agent loop
├── starpod-hooks/        Lifecycle hook system (events, callbacks, permissions)
├── starpod-core/         Shared types, config, error handling
├── starpod-db/           Unified SQLite (core.db)
├── starpod-memory/       SQLite FTS5 full-text search + markdown files
├── starpod-vault/        AES-256-GCM encrypted credentials
├── starpod-session/      Channel-aware session lifecycle
├── starpod-skills/       Self-extension skill system (markdown-based)
├── starpod-cron/         Cron scheduling (interval, cron expr, one-shot)
├── starpod-agent/        Orchestrator wiring everything together
├── starpod-gateway/      Axum HTTP/WS server + embedded web UI
├── starpod-telegram/     Telegram bot interface (teloxide)
├── starpod-instances/    Remote instance management client
└── starpod/              CLI binary
```

## Agent Layout

An agent is bootstrapped with `starpod init` and lives entirely in a `.starpod/` directory. There is no separate blueprint/instance distinction — the agent IS the instance.

```
my-agent/                           # project root
├── .starpod/
│   ├── config/                     # agent configuration
│   │   ├── agent.toml             # main config (models, server_addr, etc.)
│   │   ├── SOUL.md                # personality
│   │   ├── HEARTBEAT.md           # periodic self-reflection
│   │   ├── BOOT.md                # boot instructions
│   │   ├── BOOTSTRAP.md           # first-run instructions
│   │   └── frontend.toml          # web UI config
│   ├── skills/                     # agent skills
│   ├── db/                         # SQLite databases (runtime)
│   │   ├── core.db                # sessions, cron, auth
│   │   ├── memory.db              # FTS5 + vector memory
│   │   └── vault.db               # encrypted secrets (AES-256-GCM)
│   └── users/<id>/                 # per-user data
│       ├── USER.md
│       ├── MEMORY.md
│       └── memory/                 # daily logs
├── home/                           # agent's sandboxed filesystem
│   ├── desktop/
│   ├── documents/
│   ├── projects/
│   └── downloads/
└── .gitignore                      # excludes .starpod/db/ and home/
```

All secrets live in the vault (`vault.db`), seeded via `starpod init --env KEY=VAL` or the web UI Settings page. At startup, vault contents are injected into process environment variables.

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

The provider is selected at runtime from the `models` field in `agent.toml` (e.g. `"anthropic/claude-haiku-4-5"`):

| Provider | Struct | Default Endpoint |
|----------|--------|-----------------|
| `anthropic` | `AnthropicProvider` | `api.anthropic.com/v1/messages` |
| `bedrock` | `BedrockProvider` | `bedrock-runtime.<region>.amazonaws.com` |
| `vertex` | `VertexProvider` | `<region>-aiplatform.googleapis.com` |
| `openai` | `OpenAiProvider` | `api.openai.com/v1/chat/completions` |
| `gemini` | `GeminiProvider` | `generativelanguage.googleapis.com/v1beta` |
| `groq` | `OpenAiProvider` | `api.groq.com/openai/v1/chat/completions` |
| `deepseek` | `OpenAiProvider` | `api.deepseek.com/v1/chat/completions` |
| `openrouter` | `OpenAiProvider` | `openrouter.ai/api/v1/chat/completions` |
| `ollama` | `OpenAiProvider` | `localhost:11434/v1/chat/completions` |

Bedrock uses AWS SigV4 authentication and the AWS Event Stream binary protocol for streaming. Vertex AI uses Google OAuth2 (Application Default Credentials) and standard SSE streaming. All other providers translate between the canonical Anthropic types (`CreateMessageRequest`, `MessageResponse`, `StreamEvent`) and their own wire format internally.

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

## Mode Detection

Starpod auto-detects the agent by walking up from the current directory looking for `.starpod/config/agent.toml` (or the legacy `.starpod/agent.toml`). This means `starpod dev`, `starpod serve`, `starpod chat`, etc. work from any subdirectory — they walk up to find the nearest `.starpod/`.

If no `.starpod/` is found, commands like `starpod chat` create an ephemeral instance for the message.
