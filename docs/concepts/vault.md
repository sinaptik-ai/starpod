# Vault

The vault provides **AES-256-GCM encrypted credential storage** with audit logging. Store API keys, tokens, and secrets that the agent can access at runtime.

## How It Works

- Credentials are encrypted with AES-256-GCM before being stored in SQLite
- A master key (derived from your API key) encrypts/decrypts values
- All access is audit-logged

## Agent Tools

The `VaultGet` and `VaultSet` tools have been removed from the agent's tool set. Environment variables are now accessed via the `EnvGet` tool instead (see [Tools](/concepts/tools)).

The Vault crate still exists for **programmatic use** from Rust code (e.g., storing credentials during onboarding or via the CLI), but the agent no longer has direct tool access to the vault during conversations.

## Programmatic Use

The vault is available as a Rust library (`starpod_vault::Vault`) but does not have CLI commands. Secrets should be stored in `.env` files and accessed by the agent via the `EnvGet` tool.

## Use Cases

- Store API keys for external services the agent calls via `WebFetch`
- Store database credentials
- Store tokens for Slack, GitHub, or other integrations
- Any secret the agent needs across conversations

::: warning
The vault encrypts values at rest, but they are decrypted in memory when accessed. Treat the `.starpod/db/vault.db` file as sensitive.
:::
