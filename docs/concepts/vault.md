# Vault

The vault provides **AES-256-GCM encrypted credential storage** with audit logging. Store API keys, tokens, and secrets that the agent can access at runtime.

## How It Works

- Credentials are encrypted with AES-256-GCM before being stored in SQLite
- A master key (derived deterministically) encrypts/decrypts values
- All access is audit-logged (with optional `user_id` tracking)

## Secrets Flow

Secrets enter the vault through two paths:

1. **At init time** — `starpod init --env KEY=VAL` seeds secrets directly into the vault
2. **Via the web UI** — the Settings page provides a UI for managing vault entries

At startup (`starpod dev`, `starpod serve`, `starpod repl`, `starpod chat`), all vault secrets are decrypted and injected into the process environment via `std::env::set_var()`. The agent accesses them two ways:
- **`EnvGet` tool** — reads `std::env::var()`, blocks system keys, audit-logs each read
- **Bash/SSH commands** — child processes inherit the process environment automatically

The system prompt dynamically lists which non-system env vars are available, so the agent knows what credentials it can use.

## System Keys

System-managed secrets (LLM provider keys, service tokens, platform secrets) are protected at two layers:

- **`EnvGet` tool** — `is_system_key()` blocks reads and returns an error
- **Bash tool** — system keys are stripped from child process environments via `env_remove()`, preventing `echo $ANTHROPIC_API_KEY` or `env | grep API` from leaking them

See the [starpod-vault crate docs](/crates/starpod-vault) for the full list of system keys.

## Programmatic Use

The vault is available as a Rust library (`starpod_vault::Vault`). System keys (API keys, bot tokens) are stored in the vault and managed via the Settings UI or `starpod init --env`. The agent accesses secrets via the `EnvGet` tool.

## Use Cases

- Store API keys for external services the agent calls via `WebFetch`
- Store database credentials
- Store tokens for Slack, GitHub, or other integrations
- Any secret the agent needs across conversations

::: warning
The vault encrypts values at rest, but they are decrypted in memory when accessed. Treat the `.starpod/db/vault.db` file as sensitive.
:::
