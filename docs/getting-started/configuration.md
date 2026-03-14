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

# Followup message handling during active agent loops
# followup_mode = "inject"        # "inject" or "queue"

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
| `followup_mode` | string | `"inject"` | How followup messages are handled during an active agent loop: `"inject"` or `"queue"` |

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

For each provider, keys are resolved in priority order (first match wins):

1. `providers.<name>.api_key` in `config.toml`
2. Provider-specific environment variable (see table below)

Ollama requires no API key by default.

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
| `OPENAI_API_KEY` | `providers.openai.api_key` |
| `GEMINI_API_KEY` | `providers.gemini.api_key` |
| `GROQ_API_KEY` | `providers.groq.api_key` |
| `DEEPSEEK_API_KEY` | `providers.deepseek.api_key` |
| `OPENROUTER_API_KEY` | `providers.openrouter.api_key` |
| `TELEGRAM_BOT_TOKEN` | `telegram.bot_token` |
| `ORION_API_KEY` | API key auth for the HTTP/WS gateway |
| `ORION_INSTANCE_BACKEND_URL` | `instance_backend_url` |
