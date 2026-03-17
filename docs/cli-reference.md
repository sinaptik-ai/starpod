# CLI Reference

The `starpod` binary provides commands for all major features.

## Workspace

### `starpod init`

Initialize a new workspace in the current directory.

```bash
starpod init                        # Interactive wizard
starpod init --default              # Skip wizard, use defaults
```

The interactive wizard prompts for:
- **Provider** — Anthropic, OpenAI, Gemini, Groq, DeepSeek, OpenRouter, or Ollama
- **Model** — pre-filled with the provider's default
- **API key** — saved to `.env` (skipped if already in environment or not needed)
- **First agent** — optionally create an agent immediately

| Flag | Description |
|------|-------------|
| `--default` | Skip the wizard, use Anthropic / `claude-sonnet-4-6` |

Creates: `starpod.toml`, `agents/`, `skills/`, `.env`, `.gitignore`.

## Agent

### `starpod agent new <name>`

Create a new agent in the workspace.

```bash
starpod agent new my-agent
starpod agent new my-agent --agent-name "Jarvis" --model "claude-opus-4-6"
```

| Flag | Description | Default |
|------|-------------|---------|
| `--agent-name` | Agent's display name | `Aster` |
| `--soul` | Personality/instructions | Generic helpful assistant |
| `--model` | LLM model | `claude-sonnet-4-6` |
| `--default` | Skip interactive prompts | — |

### `starpod agent list`

List all agents in the workspace.

### `starpod serve`

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

Skills follow the [AgentSkills](https://agentskills.io) open format.

### `starpod agent skills list`

List all skills with their descriptions.

```bash
starpod agent skills list
```

### `starpod agent skills show`

Show a skill's metadata and full instructions.

```bash
starpod agent skills show code-review
```

### `starpod agent skills create`

Create a new AgentSkills-compatible skill with YAML frontmatter.

```bash
# With inline instructions
starpod agent skills create "code-review" \
  --description "Review code for bugs and style issues." \
  --body "Check for error handling, edge cases, and security."

# Instructions from a file
starpod agent skills create "code-review" \
  --description "Review code for bugs and style issues." \
  --file code-review-instructions.md
```

| Flag | Description |
|------|-------------|
| `--description`, `-d` | What the skill does and when to use it (required) |
| `--body`, `-b` | Inline markdown instructions |
| `--file`, `-f` | Read instructions from a file |

### `starpod agent skills delete`

Delete a skill and its directory.

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
