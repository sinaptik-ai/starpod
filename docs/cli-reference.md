# CLI Reference

The `starpod` binary provides commands for all major features.

## Agent

### `starpod agent init`

Initialize a new `.starpod/` project directory.

```bash
starpod agent init                  # Interactive wizard
starpod agent init --default        # Skip wizard, use defaults
starpod agent init --name "Alice" --model "claude-opus-4-6"
```

| Flag | Description |
|------|-------------|
| `--name` | Your display name |
| `--timezone` | IANA timezone |
| `--agent-name` | Agent's display name |
| `--soul` | Personality/instructions |
| `--model` | Claude model |
| `--default` | Skip the wizard |

### `starpod agent serve`

Start the HTTP/WS server with optional Telegram bot.

```bash
starpod agent serve
```

Serves the web UI, REST API, WebSocket endpoint, and (if configured) Telegram bot. All share the same agent instance.

### `starpod agent chat`

Send a one-shot message.

```bash
starpod agent chat "What files are in this directory?"
```

### `starpod agent repl`

Start an interactive REPL with readline support and history.

```bash
starpod agent repl
```

## Memory

### `starpod agent memory search`

Full-text search across memory and knowledge files.

```bash
starpod agent memory search "database migrations"
starpod agent memory search "rust patterns" --limit 10
```

| Flag | Default | Description |
|------|---------|-------------|
| `--limit`, `-l` | `5` | Maximum results |

### `starpod agent memory reindex`

Rebuild the FTS5 search index.

```bash
starpod agent memory reindex
```

## Vault

### `starpod agent vault set`

Encrypt and store a credential.

```bash
starpod agent vault set github_token "ghp_xxxxxxxxxxxx"
```

### `starpod agent vault get`

Retrieve a decrypted credential.

```bash
starpod agent vault get github_token
```

### `starpod agent vault delete`

Delete a stored credential.

```bash
starpod agent vault delete github_token
```

### `starpod agent vault list`

List all stored keys (values are not shown).

```bash
starpod agent vault list
```

## Sessions

### `starpod agent sessions list`

List recent sessions.

```bash
starpod agent sessions list
starpod agent sessions list --limit 20
```

| Flag | Default | Description |
|------|---------|-------------|
| `--limit`, `-l` | `10` | Maximum sessions |

## Skills

### `starpod agent skills list`

List all skills.

```bash
starpod agent skills list
```

### `starpod agent skills show`

Show a skill's content.

```bash
starpod agent skills show code-review
```

### `starpod agent skills create`

Create a new skill.

```bash
starpod agent skills create "code-review" --content "Always check for error handling"
starpod agent skills create "code-review" --file code-review.md
```

| Flag | Description |
|------|-------------|
| `--content`, `-c` | Inline skill content |
| `--file`, `-f` | Read content from a file |

### `starpod agent skills delete`

Delete a skill.

```bash
starpod agent skills delete code-review
```

## Cron

### `starpod agent cron list`

List all scheduled jobs.

```bash
starpod agent cron list
```

### `starpod agent cron remove`

Remove a job by name.

```bash
starpod agent cron remove "morning-reminder"
```

### `starpod agent cron runs`

Show recent executions for a job.

```bash
starpod agent cron runs "morning-reminder"
starpod agent cron runs "morning-reminder" --limit 20
```

| Flag | Default | Description |
|------|---------|-------------|
| `--limit`, `-l` | `10` | Maximum runs |

## Instances

Manage remote cloud instances. Requires `instance_backend_url` in config or `STARPOD_INSTANCE_BACKEND_URL` env var.

### `starpod instance create`

Create a new remote instance.

```bash
starpod instance create
starpod instance create --name "my-bot" --region "us-east-1"
```

| Flag | Description |
|------|-------------|
| `--name`, `-n` | Display name for the instance |
| `--region`, `-r` | Deployment region |

### `starpod instance list`

List all instances with status and region.

```bash
starpod instance list
```

### `starpod instance kill`

Terminate a running instance.

```bash
starpod instance kill <id>
```

### `starpod instance pause`

Suspend a running instance.

```bash
starpod instance pause <id>
```

### `starpod instance restart`

Resume a paused instance.

```bash
starpod instance restart <id>
```

### `starpod instance logs`

Stream logs from a running instance. Output is colored by log level (error=red, warn=yellow, info=green, debug=dim).

```bash
starpod instance logs <id>
starpod instance logs <id> --tail 100
```

| Flag | Default | Description |
|------|---------|-------------|
| `--tail`, `-t` | `50` | Number of recent log lines to stream |

### `starpod instance ssh`

Open an SSH session to a running instance. Fetches connection info from the backend and spawns a native `ssh` process. Ephemeral keys are written to a temp file and cleaned up after the session.

```bash
starpod instance ssh <id>
```

### `starpod instance health`

Display health metrics for an instance: CPU%, memory, disk, uptime, and last heartbeat.

```bash
starpod instance health <id>
```
