# Vault

The vault provides **AES-256-GCM encrypted credential storage** with audit logging. Store API keys, tokens, and secrets that the agent can access at runtime.

## How It Works

- Credentials are encrypted with AES-256-GCM before being stored in SQLite
- A master key (derived from your API key) encrypts/decrypts values
- All access is audit-logged

## Agent Tools

The agent can read and write credentials during conversations:

### VaultGet

```json
{ "key": "github_token" }
```

Returns the decrypted value, or null if the key doesn't exist.

### VaultSet

```json
{ "key": "github_token", "value": "ghp_xxxxxxxxxxxx" }
```

Encrypts and stores the value. Overwrites if the key already exists.

## CLI

```bash
# Store a credential
starpod agent vault set github_token "ghp_xxxxxxxxxxxx"

# Retrieve it
starpod agent vault get github_token

# List all stored keys (values are not shown)
starpod agent vault list

# Delete a credential
starpod agent vault delete github_token
```

## Use Cases

- Store API keys for external services the agent calls via `WebFetch`
- Store database credentials
- Store tokens for Slack, GitHub, or other integrations
- Any secret the agent needs across conversations

::: warning
The vault encrypts values at rest, but they are decrypted in memory when accessed. Treat the `.starpod/data/memory.db` file as sensitive.
:::
