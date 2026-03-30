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
- **`VaultGet` tool** — retrieves secrets directly from the vault (returns opaque tokens when proxy is enabled)
- **Bash/SSH commands** — child processes inherit the process environment automatically

The system prompt dynamically lists which non-system env vars are available, so the agent knows what credentials it can use.

## Secret Classification

Each vault entry has two metadata fields:

- **`is_secret`** (default: `true`) — Whether the value should be opaque-ified when the secret proxy is enabled. Set to `false` for non-sensitive config like `SENTRY_DSN`.
- **`allowed_hosts`** — Hostnames where the secret may be sent (e.g. `["api.openai.com"]`). `null` means unrestricted. Well-known keys (see below) get auto-suggested hosts.

### Default Hosts for Known Keys

When storing a well-known key without specifying hosts, the vault auto-suggests appropriate host bindings:

| Key | Default Hosts |
|-----|---------------|
| `ANTHROPIC_API_KEY` | `api.anthropic.com` |
| `OPENAI_API_KEY` | `api.openai.com` |
| `GEMINI_API_KEY` | `generativelanguage.googleapis.com` |
| `GROQ_API_KEY` | `api.groq.com` |
| `DEEPSEEK_API_KEY` | `api.deepseek.com` |
| `OPENROUTER_API_KEY` | `openrouter.ai` |
| `BRAVE_API_KEY` | `api.search.brave.com` |
| `TELEGRAM_BOT_TOKEN` | `api.telegram.org` |
| `GITHUB_TOKEN` / `GH_TOKEN` | `api.github.com` |

## Secret Proxy (beta)

When `proxy.enabled = true` in `agent.toml`, the vault changes how secrets are surfaced to the agent:

- **Secrets** (`is_secret = true`) are returned as **opaque tokens** (`starpod:v1:<base64(encrypted)>`) instead of plaintext. The encrypted token contains both the real value and the allowed hosts.
- **Config values** (`is_secret = false`) remain plaintext.
- The `VaultGet` tool returns opaque tokens when the proxy is active, plaintext otherwise.
- `inject_vault_env()` at startup produces opaque tokens for secret entries.

The `starpod-proxy` crate runs as a local HTTP/HTTPS proxy that intercepts outbound traffic from tool subprocesses. For HTTP, it scans headers and bodies for tokens. For HTTPS, it uses MITM with ephemeral per-host certificates (signed by a local CA) to decrypt, scan, and re-encrypt traffic. See the [starpod-proxy crate docs](/crates/starpod-proxy) for details.

### Tiered Isolation

The runtime automatically selects the strongest isolation available:

| Tier | Condition | Mechanism |
|------|-----------|-----------|
| **Tier 1** | `starpod serve` + Linux + `CAP_NET_ADMIN` | Network namespace — kernel-enforced, no bypass possible |
| **Tier 0** | All other contexts | Proxy env vars (`HTTP_PROXY`, `HTTPS_PROXY`, `SSL_CERT_FILE`) |
| **Disabled** | `proxy.enabled = false` (default) | Plaintext injection, current behavior |

### Configuration

```toml
# agent.toml
[proxy]
enabled = true  # default: false
```

The proxy toggle is also available in the web UI under **Settings > General > Secret Proxy (beta)**.

## Agent Vault Tools

The agent has four vault tools:

| Tool | Description | Confirmation |
|------|-------------|--------------|
| `VaultGet` | Retrieve a secret (opaque token when proxy active) | No |
| `VaultList` | List all non-system entries with metadata | No |
| `VaultSet` | Store a secret with metadata | **Yes** |
| `VaultDelete` | Delete a secret | **Yes** |

`VaultSet` and `VaultDelete` require user confirmation via the permission system. System keys are blocked from all vault tools.

## System Keys

System-managed secrets (LLM provider keys, service tokens, platform secrets) are protected at two layers:

- **`EnvGet` / `VaultGet` tools** — `is_system_key()` blocks reads and returns an error
- **Bash tool** — system keys are stripped from child process environments via `env_remove()`, preventing `echo $ANTHROPIC_API_KEY` or `env | grep API` from leaking them

See the [starpod-vault crate docs](/crates/starpod-vault) for the full list of system keys.

## Programmatic Use

The vault is available as a Rust library (`starpod_vault::Vault`). System keys (API keys, bot tokens) are stored in the vault and managed via the Settings UI or `starpod init --env`. The agent accesses secrets via the `VaultGet` or `EnvGet` tools.

## Use Cases

- Store API keys for external services the agent calls via `WebFetch`
- Store database credentials
- Store tokens for Slack, GitHub, or other integrations
- Any secret the agent needs across conversations

::: warning
The vault encrypts values at rest, but they are decrypted in memory when accessed. Treat the `.starpod/db/vault.db` file as sensitive.
:::
