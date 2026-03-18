# Configuration

Starpod uses a two-file configuration model:

- **`starpod.toml`** — workspace-level defaults shared across all agents (git-tracked).
- **`agent.toml`** — per-agent overrides (lives in `agents/<name>/agent.toml` in workspace mode, or `.starpod/config/agent.toml` in single-agent mode). Can contain any setting from `starpod.toml` as an override, plus **channels** (which can _only_ be configured here).

Agent personality lives in `.starpod/config/SOUL.md`. User profile lives in `.starpod/users/<id>/USER.md`. These are not part of the config files.

## Full Reference

### starpod.toml (workspace defaults)

```toml
# ─── General ────────────────────────────────────────────
provider = "anthropic"            # Active LLM provider
model = "claude-haiku-4-5"        # Model name
max_turns = 30                    # Max agentic turns per chat
max_tokens = 16384                # Max tokens for LLM API responses
server_addr = "127.0.0.1:3000"   # HTTP/WS server bind address
agent_name = "Aster"              # Agent display name (personality in SOUL.md)
# timezone = "America/New_York"   # IANA timezone for cron scheduling

# Extended thinking (optional)
# reasoning_effort = "medium"     # "low", "medium", or "high"

# Conversation compaction (optional)
# compaction_model = "claude-haiku-4-5"  # Model for summarizing old messages

# Followup message handling during active agent loops
# followup_mode = "inject"        # "inject" or "queue"

# ─── Providers ─────────────────────────────────────────
# API keys must be set via environment variables or .env files, not here.
# e.g. ANTHROPIC_API_KEY=sk-ant-... in .env

[providers.anthropic]
# enabled = true
# base_url = "https://api.anthropic.com/v1/messages"

[providers.openai]
# base_url = "https://api.openai.com/v1/chat/completions"

[providers.gemini]
# base_url = "https://generativelanguage.googleapis.com/v1beta"

[providers.groq]
# base_url = "https://api.groq.com/openai/v1/chat/completions"

[providers.deepseek]
# base_url = "https://api.deepseek.com/v1/chat/completions"

[providers.openrouter]
# base_url = "https://openrouter.ai/api/v1/chat/completions"

[providers.ollama]
# base_url = "http://localhost:11434/v1/chat/completions"  # No API key needed

# ─── Memory ───────────────────────────────────────────
[memory]
# half_life_days = 30.0           # Temporal decay half-life for daily logs
# mmr_lambda = 0.7                # 0.0 = max diversity, 1.0 = pure relevance
# vector_search = true            # Enable vector search (requires embeddings feature)
# chunk_size = 1600               # Chunk size in chars for indexing (~400 tokens)
# chunk_overlap = 320             # Overlap in chars between chunks (~80 tokens)
# bootstrap_file_cap = 20000      # Max chars per file in bootstrap context
# export_sessions = true          # Export closed session transcripts to memory for long-term recall

# ─── Compaction ───────────────────────────────────────
[compaction]
# context_budget = 160000         # Token budget triggering compaction
# summary_max_tokens = 4096       # Max tokens for the compaction summary
# min_keep_messages = 4           # Minimum messages to keep (never compact below this)

# ─── Cron ─────────────────────────────────────────────
[cron]
# default_max_retries = 3         # Default max retries for failed jobs
# default_timeout_secs = 7200     # Default job timeout (2h)
# max_concurrent_runs = 1         # Maximum concurrent job runs

# ─── Attachments ──────────────────────────────────────
[attachments]
# enabled = true                   # Set to false to disable attachments
# allowed_extensions = []          # e.g. ["jpg", "png", "pdf"]; empty = all
# max_file_size = 20971520         # Max file size in bytes (default: 20 MB)

```

### agent.toml (per-agent)

```toml
# Per-agent overrides — can contain any starpod.toml key,
# plus channels which are ONLY valid here.

# Override model for this agent:
# model = "claude-sonnet-4-6"

# ─── Channels ─────────────────────────────────────────
[channels.telegram]
# enabled = true                  # Enable/disable the Telegram channel
# gap_minutes = 360               # Inactivity gap (minutes) before new session (6h)
# allowed_users = [123456789]     # Empty = no one can chat; set TELEGRAM_BOT_TOKEN in .env
# stream_mode = "final_only"      # "final_only" or "all_messages"
```

## General Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `provider` | string | `"anthropic"` | Active LLM provider |
| `model` | string | `"claude-haiku-4-5"` | Model identifier |
| `max_turns` | integer | `30` | Max agentic loop iterations per chat |
| `max_tokens` | integer | `16384` | Maximum tokens for LLM API responses |
| `server_addr` | string | `"127.0.0.1:3000"` | Server bind address |
| `agent_name` | string | `"Aster"` | Agent display name (personality lives in `SOUL.md`) |
| `timezone` | string | — | IANA timezone for cron scheduling (user profile lives in `USER.md`) |
| `reasoning_effort` | string | — | Extended thinking: `"low"`, `"medium"`, `"high"` |
| `compaction_model` | string | primary model | Model for conversation compaction summaries |
| `followup_mode` | string | `"inject"` | How followup messages are handled during an active agent loop: `"inject"` or `"queue"` |

## Agent Personality & User Profile

Personality and user profile are **not** config settings — they live in markdown files:

| File | Purpose |
|------|---------|
| `.starpod/config/SOUL.md` | Agent personality, tone, and instructions. Loaded into every system prompt via bootstrap context. |
| `.starpod/users/<id>/USER.md` | User name, timezone, preferences. Loaded into every system prompt via bootstrap context. |

Edit these files directly to customize agent behavior or update user info. The agent can also update them itself through memory tools.

## API Key Resolution

API keys are resolved exclusively from environment variables (or `.env` files). There is no `api_key` field in config files — any `api_key` found in a config file is ignored and triggers a warning.

Each provider uses its conventional env var (e.g. `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). Ollama requires no API key by default.

::: warning
Never commit API keys to version control. Store them in `.env` files (which are gitignored) or set them as environment variables.
:::

## Telegram Settings

Telegram settings live in `agent.toml` under `[channels.telegram]`:

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the Telegram channel |
| `gap_minutes` | integer | `360` | Inactivity gap (minutes) before new session |
| `allowed_users` | array | `[]` | User IDs and/or usernames allowed to chat |
| `stream_mode` | string | `"final_only"` | `"final_only"` or `"all_messages"` |

The bot token must be set via the `TELEGRAM_BOT_TOKEN` environment variable (or `.env` file), not in config.

## Attachments

The `[attachments]` section controls file upload handling across all channels (WebSocket, Telegram, API).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Set to `false` to reject all file uploads |
| `allowed_extensions` | array | `[]` | Allowed extensions (e.g. `["jpg", "png", "pdf"]`). Empty = all allowed |
| `max_file_size` | integer | `20971520` (20 MB) | Maximum file size in bytes |

Extension matching is case-insensitive (`"jpg"` matches `photo.JPG`).

## Memory

The `[memory]` section tunes search and indexing behavior.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `half_life_days` | float | `30.0` | Temporal decay half-life (days) for daily logs |
| `mmr_lambda` | float | `0.7` | MMR diversity trade-off: `0.0` = max diversity, `1.0` = pure relevance |
| `vector_search` | bool | `true` | Enable vector search (requires `embeddings` feature) |
| `chunk_size` | integer | `1600` | Chunk size in characters for indexing (~400 tokens) |
| `chunk_overlap` | integer | `320` | Overlap in characters between chunks (~80 tokens) |
| `bootstrap_file_cap` | integer | `20000` | Max characters per file included in bootstrap context |
| `export_sessions` | bool | `true` | Export closed session transcripts to memory for long-term recall |

## Compaction

The `[compaction]` section controls conversation compaction (summarizing older messages to stay within the model's context window).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `context_budget` | integer | `160000` | Token budget that triggers compaction (~80% of model context) |
| `summary_max_tokens` | integer | `4096` | Max tokens for the compaction summary response |
| `min_keep_messages` | integer | `4` | Minimum recent messages to keep (never compacted) |

## Cron

The `[cron]` section sets defaults for the job scheduling system.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_max_retries` | integer | `3` | Default maximum retries for failed jobs |
| `default_timeout_secs` | integer | `7200` | Default job timeout in seconds (2h) |
| `max_concurrent_runs` | integer | `1` | Maximum concurrent job runs |

## Hot Reload

Starpod watches `starpod.toml` and `agent.toml` for changes while the server is running. When a file is modified, the new config is loaded and applied automatically — no restart needed.

### What reloads instantly

- `model` and `provider` — switch models on the fly
- `agent_name` — update the agent's display name
- `max_turns`, `max_tokens` — adjust limits
- `reasoning_effort` — change thinking budget
- `compaction` settings — adjust context budget
- `memory.export_sessions` — toggle session export
- `followup_mode` — switch between inject and queue

### What requires a restart

- `server_addr` — the TCP listener is already bound
- `TELEGRAM_BOT_TOKEN` — the Telegram bot is already running

When a restart-required setting changes, Starpod logs a warning but continues running with the new values for everything else.

### How it works

A file watcher (debounced at 2 seconds) monitors the `.starpod/config/` directory. On change, it reloads both config files with the same layering logic as startup, then atomically swaps the config in both the agent and gateway. The next chat request uses the new settings.

## Config Layering

When Starpod loads config, it:

1. Reads `starpod.toml` as the workspace-level base (in workspace mode)
2. Strips any `[channels]` section from `starpod.toml` (with a warning — channels belong in `agent.toml`)
3. Deep-merges the agent's `agent.toml` on top (agent values win on conflicts)

This means you can share the same `starpod.toml` across all agents and only vary `agent.toml` per agent.

**Examples:**

Disable attachments entirely:

```toml
[attachments]
enabled = false
```

Allow only images and PDFs, max 5 MB:

```toml
[attachments]
allowed_extensions = ["jpg", "jpeg", "png", "gif", "webp", "pdf"]
max_file_size = 5242880
```

## Frontend (Web UI)

The welcome screen of the web UI is configured via `frontend.toml` (in the agent blueprint, or `.starpod/config/frontend.toml` at runtime).

```toml
# Custom greeting shown below the logo (default: "ready_")
greeting = "Hi! I'm Aster."

# Suggested prompts shown as clickable chips on the welcome screen
prompts = [
    "What can you help me with?",
    "What do you remember about me?",
    "Summarize my recent notes",
]
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `greeting` | string | — | Custom greeting text below the logo. If absent, shows `ready_` |
| `prompts` | array | `[]` | Suggested prompt chips. Clicking one sends it as a message |

This file is read on every page load — changes take effect on the next browser refresh without restarting the server.

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `ANTHROPIC_API_KEY` | Anthropic API key |
| `OPENAI_API_KEY` | OpenAI API key |
| `GEMINI_API_KEY` | Gemini API key |
| `GROQ_API_KEY` | Groq API key |
| `DEEPSEEK_API_KEY` | DeepSeek API key |
| `OPENROUTER_API_KEY` | OpenRouter API key |
| `TELEGRAM_BOT_TOKEN` | Telegram bot token |
| `STARPOD_API_KEY` | API key auth for the HTTP/WS gateway |
| `STARPOD_INSTANCE_BACKEND_URL` | Remote instance backend URL |
