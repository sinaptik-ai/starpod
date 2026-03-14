# Configuration

All configuration lives in `.orion/config.toml` in your project root.

## Full Reference

```toml
# ─── General ────────────────────────────────────────────
provider = "anthropic"            # Active LLM provider
model = "claude-haiku-4-5"        # Model name
max_turns = 30                    # Max agentic turns per chat
server_addr = "127.0.0.1:3000"   # HTTP/WS server bind address

# Extended thinking (optional)
# reasoning_effort = "medium"     # "low", "medium", or "high"

# Conversation compaction (optional)
# compaction_model = "claude-haiku-4-5"  # Model for summarizing old messages

# ─── Agent Identity ────────────────────────────────────
[identity]
# name = "Orion"                  # Agent's display name
# emoji = "🤖"                    # Agent's avatar emoji
# soul = ""                       # Personality injected into system prompt

# ─── User Profile ──────────────────────────────────────
[user]
# name = "Your Name"
# timezone = "America/New_York"   # IANA timezone

# ─── Providers ─────────────────────────────────────────
[providers.anthropic]
# api_key = "sk-ant-..."          # Or set ANTHROPIC_API_KEY env var

[providers.openai]
# api_key = "sk-..."              # Not yet implemented

# ─── Instances ─────────────────────────────────────────
# instance_backend_url = "https://api.orion.example.com"  # Or set ORION_INSTANCE_BACKEND_URL env var

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
| `server_addr` | string | `"127.0.0.1:3000"` | Server bind address |
| `reasoning_effort` | string | — | Extended thinking: `"low"`, `"medium"`, `"high"` |
| `compaction_model` | string | primary model | Model for conversation compaction summaries |

## Identity

The `[identity]` section controls how the agent presents itself.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `"Orion"` | Display name in system prompt |
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

Priority order (first match wins):

1. `providers.anthropic.api_key` in `config.toml`
2. `ANTHROPIC_API_KEY` environment variable

::: warning
Never commit API keys to version control. Use environment variables or add `.orion/config.toml` to `.gitignore`.
:::

## Telegram Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `bot_token` | string | — | From BotFather (or `TELEGRAM_BOT_TOKEN` env) |
| `allowed_users` | array | `[]` | User ID allowlist |
| `stream_mode` | string | `"final_only"` | `"final_only"` or `"all_messages"` |
| `edit_throttle_ms` | integer | `300` | Edit-in-place throttle |

## Environment Variables

| Variable | Maps To |
|----------|---------|
| `ANTHROPIC_API_KEY` | `providers.anthropic.api_key` |
| `TELEGRAM_BOT_TOKEN` | `telegram.bot_token` |
| `ORION_API_KEY` | API key auth for the HTTP/WS gateway |
| `ORION_INSTANCE_BACKEND_URL` | `instance_backend_url` |
