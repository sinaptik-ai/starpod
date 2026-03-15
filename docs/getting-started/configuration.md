# Configuration

All configuration lives in `.starpod/config.toml` in your project root.

## Full Reference

```toml
# ─── General ────────────────────────────────────────────
provider = "anthropic"            # Active LLM provider
model = "claude-haiku-4-5"        # Model name
max_turns = 30                    # Max agentic turns per chat
max_tokens = 16384                # Max tokens for LLM API responses
server_addr = "127.0.0.1:3000"   # HTTP/WS server bind address

# Extended thinking (optional)
# reasoning_effort = "medium"     # "low", "medium", or "high"

# Conversation compaction (optional)
# compaction_model = "claude-haiku-4-5"  # Model for summarizing old messages

# Followup message handling during active agent loops
# followup_mode = "inject"        # "inject" or "queue"

# ─── Agent Identity ────────────────────────────────────
[identity]
# name = "Aster"                  # Agent's display name
# emoji = "🤖"                    # Agent's avatar emoji
# soul = ""                       # Personality injected into system prompt

# ─── User Profile ──────────────────────────────────────
[user]
# name = "Your Name"
# timezone = "America/New_York"   # IANA timezone

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

# ─── Session ──────────────────────────────────────────
[session]
# telegram_gap_minutes = 360      # Inactivity gap before auto-closing Telegram sessions (6h)

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

# ─── Telegram ──────────────────────────────────────────
[telegram]
# bot_token = "123456:ABC..."     # Or set TELEGRAM_BOT_TOKEN env var
# allowed_users = [123456789]     # Empty = no one can chat
# stream_mode = "final_only"      # "final_only" or "all_messages"
# edit_throttle_ms = 300
```

## General Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `provider` | string | `"anthropic"` | Active LLM provider |
| `model` | string | `"claude-haiku-4-5"` | Model identifier |
| `max_turns` | integer | `30` | Max agentic loop iterations per chat |
| `max_tokens` | integer | `16384` | Maximum tokens for LLM API responses |
| `server_addr` | string | `"127.0.0.1:3000"` | Server bind address |
| `reasoning_effort` | string | — | Extended thinking: `"low"`, `"medium"`, `"high"` |
| `compaction_model` | string | primary model | Model for conversation compaction summaries |
| `followup_mode` | string | `"inject"` | How followup messages are handled during an active agent loop: `"inject"` or `"queue"` |

## Identity

The `[identity]` section controls how the agent presents itself.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `"Aster"` | Display name in system prompt |
| `emoji` | string | — | Avatar emoji for the web UI |
| `soul` | string | — | Personality/instructions for every turn |

The `soul` field is injected directly into the system prompt:

```toml
[identity]
soul = """
You are a senior Rust developer. Always prefer idiomatic Rust.
When reviewing code, focus on safety and performance.
Never suggest using unwrap() in production code.
"""
```

## User Profile

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `user.name` | string | — | Your display name |
| `user.timezone` | string | — | IANA timezone for cron and timestamps |

## API Key Resolution

For each provider, keys are resolved in priority order (first match wins):

1. `providers.<name>.api_key` in `config.toml`
2. Provider-specific environment variable (see table below)

Ollama requires no API key by default.

::: warning
Never commit API keys to version control. Use environment variables or add `.starpod/config.toml` to `.gitignore`.
:::

## Telegram Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `bot_token` | string | — | From BotFather (or `TELEGRAM_BOT_TOKEN` env) |
| `allowed_users` | array | `[]` | User ID allowlist |
| `stream_mode` | string | `"final_only"` | `"final_only"` or `"all_messages"` |
| `edit_throttle_ms` | integer | `300` | Edit-in-place throttle |

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

## Session

The `[session]` section controls session lifecycle.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `telegram_gap_minutes` | integer | `360` | Inactivity gap (minutes) before auto-closing a Telegram session (default = 6h) |

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
| `TELEGRAM_BOT_TOKEN` | `telegram.bot_token` |
| `STARPOD_API_KEY` | API key auth for the HTTP/WS gateway |
| `STARPOD_INSTANCE_BACKEND_URL` | `instance_backend_url` |
