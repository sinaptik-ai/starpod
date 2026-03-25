# Vault

The vault provides **AES-256-GCM encrypted credential storage** with audit logging. Store API keys, tokens, and secrets that the agent can access at runtime.

## How It Works

- Credentials are encrypted with AES-256-GCM before being stored in SQLite
- A master key (derived from your API key) encrypts/decrypts values
- All access is audit-logged (with optional `user_id` tracking)

## Environment Variable Flow

Secrets flow through a three-stage pipeline:

1. **Build time** — `.env` values are validated against `deploy.toml` declarations, then encrypted into the vault via `populate_vault()`
2. **Serve time** — `inject_env_from_vault()` decrypts declared secrets and calls `std::env::set_var()` to load them into the process environment
3. **Runtime** — The agent accesses them two ways:
   - **`EnvGet` tool** — reads `std::env::var()`, blocks system keys, audit-logs each read
   - **Bash/SSH commands** — child processes inherit the process environment automatically

The system prompt dynamically lists which non-system env vars are available, so the agent knows what credentials it can use.

## System Keys

System-managed secrets (LLM provider keys, service tokens, platform secrets) are protected at two layers:

- **`EnvGet` tool** — `is_system_key()` blocks reads and returns an error
- **Bash tool** — system keys are stripped from child process environments via `env_remove()`, preventing `echo $ANTHROPIC_API_KEY` or `env | grep API` from leaking them

See the [starpod-vault crate docs](/crates/starpod-vault) for the full list of system keys.

## Programmatic Use

The vault is available as a Rust library (`starpod_vault::Vault`). System keys (API keys, bot tokens) are stored in the vault and managed via the Settings UI. For local development, secrets can be placed in `.env` — they are populated into the vault at startup via `populate_vault()`. The agent accesses secrets via the `EnvGet` tool.

## Use Cases

- Store API keys for external services the agent calls via `WebFetch`
- Store database credentials
- Store tokens for Slack, GitHub, or other integrations
- Any secret the agent needs across conversations

::: warning
The vault encrypts values at rest, but they are decrypted in memory when accessed. Treat the `.starpod/db/vault.db` file as sensitive.
:::
