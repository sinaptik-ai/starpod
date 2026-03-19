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
| `--default` | Skip the wizard, use Anthropic / `claude-haiku-4-5` |

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
| `--model` | LLM model | `claude-haiku-4-5` |
| `--default` | Skip interactive prompts | — |

### `starpod agent list`

List all agents in the workspace.

### `starpod dev`

Apply blueprint and start agent in dev mode (workspace only).

```bash
starpod dev my-agent
starpod dev my-agent --port 8080
```

| Flag | Description |
|------|-------------|
| `--port`, `-p` | Port to serve on (overrides config) |

### `starpod serve`

Start the HTTP/WS server with optional Telegram bot.

```bash
starpod serve
starpod serve -a my-agent
```

| Flag | Description |
|------|-------------|
| `--agent`, `-a` | Agent name (required in workspace mode, optional in single-agent) |

In single-agent mode, walks up from the current directory to find the nearest `.starpod/config/agent.toml`. Serves the web UI, REST API, WebSocket endpoint, and (if configured) Telegram bot. All share the same agent instance.

### `starpod chat`

Send a one-shot message.

```bash
starpod chat "What files are in this directory?"
starpod chat -a my-agent "What files are in this directory?"
```

| Flag | Description |
|------|-------------|
| `--agent`, `-a` | Agent name |

### `starpod repl`

Start an interactive REPL with readline support and history.

```bash
starpod repl
starpod repl -a my-agent
```

| Flag | Description |
|------|-------------|
| `--agent`, `-a` | Agent name |

### `starpod build`

Build a standalone `.starpod/` from an agent blueprint. Used for creating deployment-ready agent instances without a workspace.

```bash
starpod build --agent agents/my-agent
starpod build --agent agents/my-agent --skills skills/ --output /srv/my-agent --env .env
```

| Flag | Description | Default |
|------|-------------|---------|
| `--agent` | Path to agent blueprint folder (must contain `agent.toml`) | Required |
| `--skills` | Path to skills folder to include | — |
| `--output` | Where to create the `.starpod/` directory | Current directory |
| `--env` | Path to `.env` file to include | — |

Creates a self-contained `.starpod/` at the output directory, ready for `starpod serve`.

### `starpod deploy`

Deploy stub (future).

```bash
starpod deploy <agent_name>
```

## Memory

### `starpod memory search`

Full-text search across memory files.

```bash
starpod memory search "database migrations"
starpod memory search "rust patterns" --limit 10
```

| Flag | Default | Description |
|------|---------|-------------|
| `--agent`, `-a` | — | Agent name |
| `--limit`, `-l` | `5` | Maximum results |

### `starpod memory reindex`

Rebuild the FTS5 search index.

```bash
starpod memory reindex
```

## Sessions

### `starpod sessions list`

List recent sessions.

```bash
starpod sessions list
starpod sessions list --limit 20
```

| Flag | Default | Description |
|------|---------|-------------|
| `--agent`, `-a` | — | Agent name |
| `--limit`, `-l` | `10` | Maximum sessions |

## Skills

Skills follow the [AgentSkills](https://agentskills.io) open format. All skill commands accept an optional `--agent` / `-a` flag to target a specific agent's skills in workspace mode.

```bash
starpod skill list                          # auto-detect
starpod skill --agent my-agent list         # target specific agent
```

### `starpod skill list`

List all skills with their descriptions.

```bash
starpod skill list
```

### `starpod skill show`

Show a skill's metadata and full instructions.

```bash
starpod skill show code-review
```

### `starpod skill new`

Generate a new AgentSkills-compatible skill using AI. The name is required; description and body are AI-generated from the name (and optional extra context).

```bash
# Name only — AI generates description and instructions
starpod skill new code-review

# With explicit description
starpod skill new code-review \
  --description "Review code for bugs, security issues, and style."

# With extra context for the AI generator
starpod skill new code-review \
  --prompt "Focus on OWASP top 10 and always check error handling"

# Both
starpod skill new code-review \
  --description "Review code for bugs and security." \
  --prompt "Focus on OWASP top 10, always check error handling"
```

| Flag | Description |
|------|-------------|
| `--description`, `-d` | What the skill does and when to use it (overrides AI) |
| `--prompt`, `-p` | Extra instructions or context for the AI generator |

### `starpod skill delete`

Delete a skill and its directory.

```bash
starpod skill delete code-review
```

## Cron

### `starpod cron list`

List all scheduled jobs.

```bash
starpod cron list
```

### `starpod cron remove`

Remove a job by name.

```bash
starpod cron remove "morning-reminder"
```

### `starpod cron runs`

Show recent executions for a job.

```bash
starpod cron runs "morning-reminder"
starpod cron runs "morning-reminder" --limit 20
```

| Flag | Default | Description |
|------|---------|-------------|
| `--limit`, `-l` | `10` | Maximum runs |

### `starpod cron run`

Trigger a cron job immediately.

```bash
starpod cron run "morning-reminder"
```

### `starpod cron edit`

Edit a cron job's properties.

```bash
starpod cron edit "morning-reminder" --prompt "New prompt text"
starpod cron edit "morning-reminder" --schedule "0 9 * * *" --enabled false
```

| Flag | Description |
|------|-------------|
| `--prompt` | New prompt text |
| `--schedule` | New cron schedule expression |
| `--enabled` | Enable or disable the job (`true`/`false`) |
| `--max-retries` | Max retries on failure |
| `--timeout-secs` | Timeout in seconds |
| `--session-mode` | Session mode: `isolated` or `main` |

## Instances

Manage remote cloud instances. Requires `STARPOD_INSTANCE_BACKEND_URL` env var.

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
