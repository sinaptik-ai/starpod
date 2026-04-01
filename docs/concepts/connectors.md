# Connectors

A connector represents a connection to an external service. It answers one question: **"can this agent authenticate to service X?"**

Connectors are auth-agnostic — they don't care whether the credentials are consumed by an API call, a CLI tool, an MCP server, or a skill script. They just ensure the right secrets are in the vault and available at runtime.

## Architecture

```
Template (.toml)              Connector (DB row)             Vault
────────────────              ──────────────────             ─────
"Here's how to create         "I AM the github              Encrypted
 a github connector:           connection:                   key-value
 - needs GITHUB_TOKEN         - secrets: [GITHUB_TOKEN]     store:
 - supports OAuth              - config: {base_url:...}
 - default base_url is..."    - auth: oauth                 GITHUB_TOKEN = [enc]
                               - status: connected"
      │                               │
      │ consumed once                 │ references keys
      │ during setup                  │ by name
      └──────► creates ──────────────┘
```

- **Template** = recipe (`.toml` file, read once during setup, never at runtime)
- **Connector** = the live thing (DB row, source of truth, queried at runtime)
- **Vault** = the safe (encrypted storage, knows nothing about connectors)

## Template Definition

A `.toml` file in `.starpod/connectors/` that describes a service and how to connect to it. Starpod ships built-in templates that are copied here on `starpod init`. Users can edit, delete, or add custom ones.

```toml
# Required
name = "github"
display_name = "GitHub"
description = "Access GitHub repositories, pull requests, issues, and actions"

# Whether multiple instances of this connector can be created.
# When true, setup asks for an instance name and namespaces vault keys.
# When false (default), instance name = template name, vault keys used as-is.
multi_instance = false

# Runtime secrets — logical vault key names this connector needs.
# Omit for OAuth-only connectors.
secrets = ["GITHUB_TOKEN"]
optional_secrets = ["GITHUB_APP_KEY"]

# Default configuration — non-secret values, overridable per-instance.
config = { base_url = "https://api.github.com" }

# Optional: OAuth as an alternative (or only) way to obtain secrets.
[oauth]
authorize_url = "https://github.com/login/oauth/authorize"
token_url = "https://github.com/login/oauth/access_token"
scopes = ["repo", "read:org"]
token_key = "GITHUB_TOKEN"
refresh_key = "GITHUB_REFRESH_TOKEN"        # enables auto-refresh
client_id_key = "GITHUB_CLIENT_ID"          # user's own OAuth app (optional)
client_secret_key = "GITHUB_CLIENT_SECRET"  # if omitted, Starpod's built-in app
```

### Template Field Reference

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | yes | — | Unique identifier. Lowercase alphanumeric + hyphens. |
| `display_name` | string | yes | — | Human-readable label for UI. |
| `description` | string | yes | — | What this connector provides. Shown in UI and system prompt. |
| `multi_instance` | bool | no | `false` | If true, setup asks for an instance name and namespaces vault keys. |
| `secrets` | string[] | no | `[]` | Logical vault key names required at runtime. |
| `optional_secrets` | string[] | no | `[]` | Logical vault keys that enhance functionality but aren't required. |
| `config` | table | no | `{}` | Default non-secret configuration values. |
| `oauth` | table | no | — | OAuth setup path. See below. |

### OAuth Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `authorize_url` | string | yes | OAuth authorization endpoint. |
| `token_url` | string | yes | Token exchange endpoint. |
| `scopes` | string[] | yes | OAuth scopes to request. |
| `token_key` | string | yes | Logical vault key for the access token. |
| `refresh_key` | string | no | Logical vault key for refresh token. Presence enables auto-refresh. |
| `client_id_key` | string | no | Vault key for user-provided client ID. If omitted, Starpod uses its built-in OAuth app. |
| `client_secret_key` | string | no | Vault key for user-provided client secret. |

### Setup Behavior Matrix

| `secrets` | `[oauth]` | Setup Flow |
|-----------|-----------|------------|
| present | absent | Prompt for each secret |
| present | present | User chooses: paste token **or** OAuth sign-in |
| absent | present | OAuth-only (no manual token option) |
| absent | absent | Config-only connector (no auth needed) |

### Multi-Instance Behavior

| `multi_instance` | Setup flow | Instance name | Vault key derivation |
|------------------|------------|---------------|----------------------|
| `false` | No name prompt | Same as template name (`github`) | Logical key as-is (`GITHUB_TOKEN`) |
| `true` | Asks for instance name | User provides (`analytics-db`) | `<INSTANCE>_<LOGICAL_KEY>` (`ANALYTICS_DB_DATABASE_URL`) |

Even with `multi_instance = false`, a user can still force a second instance by providing a name explicitly (e.g., a second GitHub for GHE). The flag controls the default flow.

## Connector Instance (Database)

The connector row is the **source of truth** at runtime. It stores resolved vault keys, config, auth method, and status. The template is never read again after setup.

### SQL Schema

```sql
CREATE TABLE connectors (
    name TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL,
    auth_method TEXT NOT NULL DEFAULT 'token',
    secrets TEXT NOT NULL DEFAULT '[]',
    config TEXT NOT NULL DEFAULT '{}',
    oauth_token_url TEXT,
    oauth_token_key TEXT,
    oauth_refresh_key TEXT,
    oauth_expires_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### Column Reference

| Column | Type | Description |
|--------|------|-------------|
| `name` | TEXT PK | Instance name. Single-instance: same as `type` (`"github"`). Multi-instance: user-provided (`"analytics-db"`). |
| `type` | TEXT | Template name this was created from (`"github"`, `"postgres"`). |
| `display_name` | TEXT | Human-readable name. Copied from template, can be overridden per-instance. |
| `description` | TEXT | What this connection is for. Single-instance: copied from template. Multi-instance: user provides (e.g., "Production analytics warehouse"). |
| `auth_method` | TEXT | `"token"` or `"oauth"`. |
| `secrets` | TEXT | JSON array of **resolved** vault keys (e.g., `["GITHUB_TOKEN"]` or `["ANALYTICS_DB_DATABASE_URL"]`). |
| `config` | TEXT | JSON object of config values. Template defaults merged with instance overrides. |
| `oauth_token_url` | TEXT | Token endpoint URL, for refresh flow. Null for token-based connectors. |
| `oauth_token_key` | TEXT | Vault key holding the access token. Null for token-based. |
| `oauth_refresh_key` | TEXT | Vault key holding the refresh token. Null if no refresh. |
| `oauth_expires_at` | TEXT | ISO 8601 expiry of the access token. Null if no refresh. |
| `status` | TEXT | `"connected"`, `"pending"`, or `"error"`. |
| `created_at` | TEXT | ISO 8601. |
| `updated_at` | TEXT | ISO 8601. |

## Vault

The vault is unchanged — a dumb encrypted key-value store. It knows nothing about connectors. The connector knows which vault keys are its own (via the `secrets` array).

```sql
CREATE TABLE vault_entries (
    key TEXT PRIMARY KEY,
    encrypted_value BLOB NOT NULL,
    nonce BLOB NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

To find which secrets belong to a connector: read the connector's `secrets` column.
To delete a connector's secrets: read `secrets`, then delete those vault keys.

## Setup Flows

Setup can happen from the **web UI** (settings page), **chat** (agent walks user through it), or **`starpod init`** (first run).

### Single-Instance: GitHub (token)

```
User: connect to GitHub
Nova: Paste your GitHub token or sign in with OAuth?
User: ghp_xxxxxxxxxxxx
Nova: ✓ Connected to GitHub (@gabrieleventuri)
```

What happens:
1. Template `github.toml` loaded → `multi_instance = false`, `secrets = ["GITHUB_TOKEN"]`
2. No name prompt needed → instance name = `"github"`
3. Vault key = `GITHUB_TOKEN` (logical key as-is)
4. Vault: `GITHUB_TOKEN` → `[encrypted ghp_xxx]`
5. Connector row inserted:

| name | type | auth_method | secrets | config | status |
|------|------|-------------|---------|--------|--------|
| `github` | `github` | `token` | `["GITHUB_TOKEN"]` | `{"base_url":"https://api.github.com"}` | `connected` |

### Single-Instance: GitHub (OAuth)

```
User: connect to GitHub
Nova: Paste your GitHub token or sign in with OAuth?
User: OAuth
Nova: → Opens browser for GitHub OAuth flow
      ✓ Connected to GitHub (@gabrieleventuri)
```

What happens:
1. Same template, user picks OAuth
2. OAuth flow completes → access token received
3. Vault: `GITHUB_TOKEN` → `[encrypted gho_xxx]`
4. Connector row:

| name | type | auth_method | secrets | oauth_token_url | oauth_token_key | status |
|------|------|-------------|---------|----------------|-----------------|--------|
| `github` | `github` | `oauth` | `["GITHUB_TOKEN"]` | `https://github.com/login/oauth/access_token` | `GITHUB_TOKEN` | `connected` |

Same vault key either way. Runtime doesn't know how it got there.

### OAuth-Only: Google Calendar

```
User: connect Google Calendar
Nova: → Opens browser for Google OAuth
      ✓ Connected to Google Calendar
```

What happens:
1. Template has no `secrets`, only `[oauth]` → OAuth is the only path
2. OAuth flow produces access token + refresh token
3. Vault: `GOOGLE_CALENDAR_TOKEN` → `[encrypted ya29.xxx]`, `GOOGLE_CALENDAR_REFRESH_TOKEN` → `[encrypted 1//xxx]`
4. Connector row:

| name | type | auth_method | secrets | oauth_token_key | oauth_refresh_key | oauth_expires_at | status |
|------|------|-------------|---------|-----------------|-------------------|------------------|--------|
| `google-calendar` | `google-calendar` | `oauth` | `["GOOGLE_CALENDAR_TOKEN"]` | `GOOGLE_CALENDAR_TOKEN` | `GOOGLE_CALENDAR_REFRESH_TOKEN` | `2026-03-31T14:00:00Z` | `connected` |

### Multi-Instance: Two Postgres Databases

```
User: connect my analytics database
Nova: What type of connection? → PostgreSQL
      What should I call this connection?
User: analytics-db
Nova: Enter the database URL:
User: postgres://user:pass@analytics.rds:5432/analytics
Nova: ✓ Connected "analytics-db" (PostgreSQL)
```

What happens:
1. Template `postgres.toml` loaded → `multi_instance = true`, `secrets = ["DATABASE_URL"]`
2. User provides name `"analytics-db"`
3. Vault key derived: `ANALYTICS_DB` + `_` + `DATABASE_URL` = `ANALYTICS_DB_DATABASE_URL`
4. Vault: `ANALYTICS_DB_DATABASE_URL` → `[encrypted postgres://...]`
5. Connector row:

| name | type | display_name | description | secrets | status |
|------|------|-------------|-------------|---------|--------|
| `analytics-db` | `postgres` | PostgreSQL | Production analytics warehouse | `["ANALYTICS_DB_DATABASE_URL"]` | `connected` |

Second database — same flow:

| name | type | description | secrets | status |
|------|------|-------------|---------|--------|
| `users-db` | `postgres` | User service database | `["USERS_DB_DATABASE_URL"]` | `connected` |

Vault:
| key | encrypted_value |
|-----|----------------|
| `ANALYTICS_DB_DATABASE_URL` | `[encrypted postgres://...@analytics]` |
| `USERS_DB_DATABASE_URL` | `[encrypted postgres://...@users]` |

### Multi-Secret: AWS

```
User: connect to AWS
Nova: Enter your AWS Access Key ID:
User: AKIAXXXXXXXX
Nova: Enter your AWS Secret Access Key:
User: wJalXXXXXXXX
Nova: AWS Region? (default: us-east-1)
User: eu-west-1
Nova: ✓ Connected to AWS (eu-west-1)
```

| name | type | secrets | config | status |
|------|------|---------|--------|--------|
| `aws` | `aws` | `["AWS_ACCESS_KEY_ID","AWS_SECRET_ACCESS_KEY"]` | `{"AWS_REGION":"eu-west-1"}` | `connected` |

### Custom Connector

User creates `.starpod/connectors/my-erp.toml`:
```toml
name = "my-erp"
display_name = "Company ERP"
description = "Internal ERP for inventory and order management"
secrets = ["ERP_API_KEY"]
config = { base_url = "https://erp.corp.com/api" }
```

Then sets it up via chat or web UI — identical flow to any built-in template.

## Runtime

### Boot Sequence

1. Query `connectors` table for all rows
2. For each connector:
   - Check vault for keys listed in `secrets` → update `status`
   - If `oauth_refresh_key` is set: check `oauth_expires_at`, auto-refresh if needed
3. Inject env vars for connected connectors
4. Expose connector status in system prompt

No template files are read at runtime. The connector row has everything.

### System Prompt

```xml
<connectors>
  <connector name="github" type="github" status="connected"
    description="Access GitHub repositories, pull requests, issues, and actions" />
  <connector name="analytics-db" type="postgres" status="connected"
    description="Production analytics warehouse" />
  <connector name="users-db" type="postgres" status="pending"
    description="User service database" missing="USERS_DB_DATABASE_URL" />
  <connector name="google-calendar" type="google-calendar" status="connected"
    description="Read and manage Google Calendar events and schedules" />
</connectors>
```

### Token Refresh (OAuth)

When `oauth_refresh_key` is set on a connector:

1. Before using the access token, check `oauth_expires_at`
2. If expired: decrypt refresh token from vault → call `oauth_token_url` → new access token
3. Update vault (new access token) + connector row (new `oauth_expires_at`)
4. If refresh fails: set `status = 'error'` → agent tells user to reconnect

### Env Var Injection

- **Single-instance** (`name == type`): vault keys injected as env vars directly (e.g., `GITHUB_TOKEN`)
- **Multi-instance** (`name != type`): vault keys are already namespaced (e.g., `ANALYTICS_DB_DATABASE_URL`), injected as-is

### Skills Integration

Skills reference connectors by name:

```yaml
---
name: github-pr-review
connectors: [github]
---
```

At skill activation:
- Check `connectors` table: is `github` present with `status = 'connected'`?
- Yes → activate normally
- No → agent tells user to set up the connector

## Templates Directory

```
.starpod/connectors/
├── github.toml
├── google-calendar.toml
├── google-drive.toml
├── slack.toml
├── linear.toml
├── postgres.toml
├── mysql.toml
├── redis.toml
├── stripe.toml
├── aws.toml
├── meta-ads.toml
├── telegram.toml
├── smtp.toml
├── my-erp.toml              # custom
└── ...
```

Seeded on `starpod init`. Users can add, edit, or remove templates freely. Templates are only read during connector setup.
