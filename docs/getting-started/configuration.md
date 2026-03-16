# Configuration

Starpod uses a two-file configuration model:

- **`config.toml`** — shared settings (model, provider, memory, compaction, cron, etc.). Deploy the same file across all instances.
- **`instance.toml`** — instance-specific overrides. Can contain any setting from `config.toml` as an override, plus **channels** (which can _only_ be configured here).

Agent personality lives in `.starpod/data/SOUL.md`. User profile lives in `.starpod/data/USER.md`. These are not part of the config files.

## Full Reference

### config.toml (shared)

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
[providers.anthropic]
# api_key = "sk-ant-..."          # Or set ANTHROPIC_API_KEY env var
# base_url = "https://api.anthropic.com/v1/messages"

[providers.openai]
# api_key = "sk-..."              # Or set OPENAI_API_KEY env var
# base_url = "https://api.openai.com/v1/chat/completions"

[providers.gemini]
# api_key = "..."                 # Or set GEMINI_API_KEY env var
# base_url = "https://generativelanguage.googleapis.com/v1beta"

[providers.groq]
# api_key = "gsk_..."             # Or set GROQ_API_KEY env var

[providers.deepseek]
# api_key = "..."                 # Or set DEEPSEEK_API_KEY env var

[providers.openrouter]
# api_key = "..."                 # Or set OPENROUTER_API_KEY env var

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

# ─── Instances ─────────────────────────────────────────
# instance_backend_url = "https://api.starpod.example.com"  # Or set STARPOD_INSTANCE_BACKEND_URL env var

[instances]
# health_check_interval_secs = 30  # Health check polling interval
# heartbeat_timeout_secs = 90      # Instance unhealthy after this
# http_timeout_secs = 30           # HTTP request timeout for instance API calls
```

### instance.toml (per-instance)

```toml
# Instance-specific overrides — can contain any config.toml key,
# plus channels which are ONLY valid here.

# Override model for this instance:
# model = "claude-sonnet-4-6"

# ─── Channels ─────────────────────────────────────────
[channels.telegram]
# enabled = true                  # Enable/disable the Telegram channel
# gap_minutes = 360               # Inactivity gap (minutes) before new session (6h)
# bot_token = "123456:ABC..."     # Or set TELEGRAM_BOT_TOKEN env var
# allowed_users = [123456789]     # Empty = no one can chat
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
| `.starpod/data/SOUL.md` | Agent personality, tone, and instructions. Loaded into every system prompt via bootstrap context. |
| `.starpod/data/USER.md` | User name, timezone, preferences. Loaded into every system prompt via bootstrap context. |

Edit these files directly to customize agent behavior or update user info. The agent can also update them itself through memory tools.

## API Key Resolution

For each provider, keys are resolved in priority order (first match wins):

1. `providers.<name>.api_key` in `config.toml`
2. Provider-specific environment variable (see table below)

Ollama requires no API key by default.

::: warning
Never commit API keys to version control. Use environment variables or add `.starpod/config.toml` to `.gitignore`.
:::

## Telegram Settings

Telegram settings live in `instance.toml` under `[channels.telegram]`:

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the Telegram channel |
| `gap_minutes` | integer | `360` | Inactivity gap (minutes) before new session |
| `bot_token` | string | — | From BotFather (or `TELEGRAM_BOT_TOKEN` env) |
| `allowed_users` | array | `[]` | User ID allowlist |
| `stream_mode` | string | `"final_only"` | `"final_only"` or `"all_messages"` |

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

## Instances

The `[instances]` section configures remote instance management.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `health_check_interval_secs` | integer | `30` | Health check polling interval in seconds |
| `heartbeat_timeout_secs` | integer | `90` | Seconds before an instance is considered unhealthy |
| `http_timeout_secs` | integer | `30` | HTTP request timeout for instance API calls |

## Config Layering

When Starpod loads config, it:

1. Reads `.starpod/config.toml` as the base
2. Strips any `[channels]` section from `config.toml` (with a warning — channels belong in `instance.toml`)
3. If `.starpod/instance.toml` exists, deep-merges it on top (instance values win on conflicts)

This means you can deploy the same `config.toml` to every VM and only vary `instance.toml` per machine.

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

## Environment Variables

| Variable | Maps To |
|----------|---------|
| `ANTHROPIC_API_KEY` | `providers.anthropic.api_key` |
| `OPENAI_API_KEY` | `providers.openai.api_key` |
| `GEMINI_API_KEY` | `providers.gemini.api_key` |
| `GROQ_API_KEY` | `providers.groq.api_key` |
| `DEEPSEEK_API_KEY` | `providers.deepseek.api_key` |
| `OPENROUTER_API_KEY` | `providers.openrouter.api_key` |
| `TELEGRAM_BOT_TOKEN` | `channels.telegram.bot_token` |
| `STARPOD_API_KEY` | API key auth for the HTTP/WS gateway |
| `STARPOD_INSTANCE_BACKEND_URL` | `instance_backend_url` |
