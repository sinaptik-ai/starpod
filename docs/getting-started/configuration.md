# Configuration

Starpod uses a single configuration file: **`agent.toml`** in `.starpod/config/`. It contains all settings for the agent — model, server address, channels, memory, cron, and more.

Agent personality lives in `.starpod/config/SOUL.md`. User profile lives in `.starpod/users/<id>/USER.md`. These are not part of the config file.

## Full Reference

### agent.toml

```toml
# ─── General ────────────────────────────────────────────
agent_name = "Nova"                  # Agent display name
models = ["anthropic/claude-haiku-4-5"]  # Models in provider/model format (first = default)
max_turns = 30                        # Max agentic turns per chat
server_addr = "127.0.0.1:3000"        # HTTP/WS server bind address

# max_tokens = 16384                  # Max tokens for LLM API responses
# reasoning_effort = "medium"         # "low", "medium", or "high"
# compaction_model = "anthropic/claude-haiku-4-5"  # Model for summarizing old messages
# timezone = "Europe/Rome"            # IANA timezone for cron scheduling
# followup_mode = "inject"            # "inject" or "queue"
# self_improve = false                # Beta: auto-create skills from complex tasks

# ─── Providers ─────────────────────────────────────────
# API keys are stored in the encrypted vault.
# Seed via `starpod init --env KEY=VAL` or manage in the web UI Settings page.

# [providers.anthropic]
# base_url = "https://api.anthropic.com/v1/messages"

[providers.bedrock]
# enabled = true
# [providers.bedrock.options]
# region = "us-east-1"             # AWS region for Bedrock API calls

[providers.vertex]
# enabled = true
# [providers.vertex.options]
# project_id = "my-gcp-project"   # Google Cloud project ID (or set GOOGLE_CLOUD_PROJECT)
# region = "us-central1"          # Vertex AI region (or "global" for auto-routing)

[providers.openai]
# base_url = "https://api.openai.com/v1/chat/completions"

# [providers.ollama]
# base_url = "http://localhost:11434/v1/chat/completions"
# [providers.ollama.options]
# keep_alive = "5m"
# num_ctx = 32768

# ─── Memory ───────────────────────────────────────────
# [memory]
# half_life_days = 30.0
# mmr_lambda = 0.7
# vector_search = true
# chunk_size = 1600
# chunk_overlap = 320
# bootstrap_file_cap = 20000
# export_sessions = true

# ─── Compaction ───────────────────────────────────────
# [compaction]
# context_budget = 160000
# summary_max_tokens = 4096
# min_keep_messages = 4

# ─── Cron ─────────────────────────────────────────────
# [cron]
# default_max_retries = 3
# default_timeout_secs = 7200
# max_concurrent_runs = 1

# ─── Attachments ──────────────────────────────────────
# [attachments]
# enabled = true
# allowed_extensions = []
# max_file_size = 20971520

# ─── Internet ─────────────────────────────────────────
# [internet]
# enabled = true
# timeout_secs = 15
# max_fetch_bytes = 524288

# ─── Channels ─────────────────────────────────────────
# [channels.telegram]
# enabled = true
# gap_minutes = 360
# allowed_users = [123456789]
# stream_mode = "final_only"
```

## General Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `agent_name` | string | `"Nova"` | Agent display name (personality lives in `SOUL.md`) |
| `models` | array | `["anthropic/claude-haiku-4-5"]` | Models in `provider/model` format (first is default) |
| `max_turns` | integer | `30` | Max agentic loop iterations per chat |
| `max_tokens` | integer | `16384` | Maximum tokens for LLM API responses |
| `server_addr` | string | `"127.0.0.1:3000"` | Server bind address |
| `timezone` | string | — | IANA timezone for cron scheduling |
| `reasoning_effort` | string | — | Extended thinking: `"low"`, `"medium"`, `"high"` |
| `compaction_model` | string | primary model | Model for conversation compaction summaries |
| `followup_mode` | string | `"inject"` | How followup messages are handled: `"inject"` or `"queue"` |
| `self_improve` | bool | `false` | Beta: agent proactively creates skills from complex tasks |

## Agent Personality & User Profile

Personality and user profile are **not** config settings — they live in markdown files:

| File | Purpose |
|------|---------|
| `.starpod/config/SOUL.md` | Agent personality, tone, and instructions. Loaded into every system prompt. |
| `.starpod/users/<id>/USER.md` | User name, timezone, preferences. Loaded into every system prompt. |

Edit these files directly to customize agent behavior or update user info. The agent can also update them itself through memory tools.

## API Key Resolution

API keys are stored in the **encrypted vault** and managed via the web UI Settings page or seeded at init time with `starpod init --env KEY=VAL`. There is no `api_key` field in config files.

Each provider uses its conventional env var name (e.g. `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). AWS Bedrock uses `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` instead of a single API key, plus an optional `AWS_SESSION_TOKEN` for temporary credentials. Google Vertex AI uses Application Default Credentials (ADC) — set `GOOGLE_APPLICATION_CREDENTIALS` to point to a service account JSON file, or run `gcloud auth application-default login` for local development. For local development, keys can be placed in `.env` — they are populated into the vault at startup. Ollama requires no API key by default.

::: warning
Never commit API keys to version control. Use the vault for all secrets.
:::

## Provider Options

Each provider supports an optional `[providers.<name>.options]` table for provider-specific fields that are merged into every API request body. This is most useful for Ollama, which accepts extra parameters to control model loading and context size.

| Key | Provider | Default | Description |
|-----|----------|---------|-------------|
| `region` | Bedrock | `AWS_REGION` env | AWS region for Bedrock API calls (e.g. `eu-west-1`, `us-east-1`). Falls back to `AWS_REGION` env var. |
| `project_id` | Vertex | `GOOGLE_CLOUD_PROJECT` env | Google Cloud project ID. Falls back to `GOOGLE_CLOUD_PROJECT` or `GCP_PROJECT_ID` env var. **Required.** |
| `region` | Vertex | `us-central1` | Vertex AI region (e.g. `us-east1`, `europe-west1`, `global`). Falls back to `GOOGLE_CLOUD_LOCATION` or `GCP_REGION` env var. |
| `keep_alive` | Ollama | `"5m"` | How long to keep the model loaded after a request. Ensures KV cache reuse between agentic loop turns. Set to `"-1"` for indefinite. |
| `num_ctx` | Ollama | model default | Context window size override. Larger values use more VRAM but allow longer conversations. |

Starpod automatically sets `keep_alive = "5m"` for Ollama if not explicitly configured.

```toml
[providers.ollama.options]
keep_alive = "30m"
num_ctx = 32768
```

Options are passed through as-is — any field accepted by the provider's API can be set here.

## Telegram Settings

Telegram settings live in `agent.toml` under `[channels.telegram]` and can also be managed from the **Settings > Channels** tab in the web UI.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the Telegram channel |
| `gap_minutes` | integer | `360` | Inactivity gap (minutes) before new session |
| `stream_mode` | string | `"final_only"` | `"final_only"` or `"all_messages"` |

The bot token is stored in the encrypted vault. Set it via **Settings > Channels** in the web UI, or seed it with `starpod init --env TELEGRAM_BOT_TOKEN=...`.

### Telegram User Linking

Telegram access is controlled via user-level linking in the **Settings > Users** tab. Expand a user and use the **Telegram** section to link their Telegram ID to their Starpod user account.

API endpoints for Telegram linking:

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/settings/auth/users/:id/telegram` | Get user's Telegram link |
| `PUT` | `/api/settings/auth/users/:id/telegram` | Link Telegram ID to user |
| `DELETE` | `/api/settings/auth/users/:id/telegram` | Unlink Telegram from user |

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
| `nudge_interval` | integer | `10` | Background memory review every N user messages (`0` = disabled) |
| `nudge_model` | string | — | Model for background reviews (falls back to flush → compaction → primary model) |

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

Starpod watches `agent.toml` for changes while the server is running. When the file is modified, the new config is loaded and applied automatically — no restart needed.

### What reloads instantly

- `models` — switch models on the fly
- `agent_name` — update the agent's display name
- `max_turns`, `max_tokens` — adjust limits
- `reasoning_effort` — change thinking budget
- `compaction` settings — adjust context budget
- `memory.export_sessions` — toggle session export
- `memory.nudge_interval` — adjust review frequency
- `followup_mode` — switch between inject and queue
- `self_improve` — toggle self-improve mode

### What requires a restart

- `server_addr` — the TCP listener is already bound
- `TELEGRAM_BOT_TOKEN` — the Telegram bot is already running

When a restart-required setting changes, Starpod logs a warning but continues running with the new values for everything else.

### How it works

A file watcher (debounced at 2 seconds) monitors the `.starpod/config/` directory. On change, it reloads the config, then atomically swaps it in both the agent and gateway. The next chat request uses the new settings.

## Frontend (Web UI)

The welcome screen of the web UI is configured via `frontend.toml` in `.starpod/config/`.

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
| `AWS_ACCESS_KEY_ID` | AWS access key for Bedrock |
| `AWS_SECRET_ACCESS_KEY` | AWS secret key for Bedrock |
| `AWS_SESSION_TOKEN` | AWS session token for Bedrock (optional, for temporary credentials) |
| `AWS_REGION` | AWS region for Bedrock (default: `us-east-1`) |
| `GOOGLE_CLOUD_PROJECT` | Google Cloud project ID for Vertex AI (or `GCP_PROJECT_ID`) |
| `GOOGLE_CLOUD_LOCATION` | Vertex AI region (or `GCP_REGION`, default: `us-central1`) |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to GCP service account JSON for Vertex AI (optional if using ADC) |
| `OPENAI_API_KEY` | OpenAI API key |
| `GEMINI_API_KEY` | Gemini API key |
| `GROQ_API_KEY` | Groq API key |
| `DEEPSEEK_API_KEY` | DeepSeek API key |
| `OPENROUTER_API_KEY` | OpenRouter API key |
| `TELEGRAM_BOT_TOKEN` | Telegram bot token |
| `STARPOD_API_KEY` | API key auth for the HTTP/WS gateway |

All of these should be stored in the vault, not in environment variables directly. They are listed here for reference — the vault injects them into the process environment at startup.
