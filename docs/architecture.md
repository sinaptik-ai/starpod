# Architecture

Starpod is a Rust workspace with 11 crates, each responsible for a single concern.

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
        memory    vault   session  skills    cron
            │        │       │       │        │
            └────────┴───────┼───────┴────────┘
                             ▼
                         starpod-core
                             │
                         agent-sdk
```

## Data Flow

### 1. User Sends a Message

Via the web UI (WebSocket), Telegram bot, CLI (`starpod agent chat`), or HTTP API (`POST /api/chat`).

### 2. Channel Routing

The `starpod-agent` maps the incoming message to a **Channel** (`Main` or `Telegram`) and resolves the session:
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

The agent has access to file I/O, web search, memory, vault, skills, and cron tools.

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
| Vault | `Arc<Vault>` | Agent |
| Sessions | `Arc<SessionManager>` | Agent, Gateway |
| Skills | `Arc<SkillStore>` | Agent |
| Cron | `Arc<CronStore>` | Agent, Scheduler |

SQLite connections use `Mutex<Connection>` for safe concurrent access.

## Project Directory

```
.starpod/
├── config.toml          Shared configuration (model, provider, etc.)
├── instance.toml        Instance-specific config (channels, overrides)
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

Starpod walks up from the current directory to find the nearest `.starpod/` folder — just like Git finds `.git/`.
