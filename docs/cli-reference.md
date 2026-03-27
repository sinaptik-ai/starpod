# CLI Reference

The `starpod` binary provides commands for managing and running your AI agent.

## Global Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--format` | Output format: `text` or `json` | `text` |

## `starpod init`

Bootstrap a new agent in the current directory. Creates `.starpod/` with config, database, and skill directories, plus a `home/` sandbox.

```bash
starpod init
starpod init --name "Jarvis" --model openai/gpt-4o
starpod init --env ANTHROPIC_API_KEY=sk-ant-... --env BRAVE_API_KEY=...
```

| Flag | Description | Default |
|------|-------------|---------|
| `--name` | Agent display name | `Aster` |
| `--model` | Model in `provider/model` format | `anthropic/claude-haiku-4-5` |
| `--env KEY=VAL` | Seed a secret into the vault (repeatable) | — |

Creates:
- `.starpod/config/` — `agent.toml`, `SOUL.md`, `frontend.toml`, `HEARTBEAT.md`, `BOOT.md`, `BOOTSTRAP.md`
- `.starpod/db/` — database directory (vault.db created if `--env` is used)
- `.starpod/skills/` — agent skills
- `.starpod/users/` — per-user data
- `home/` — agent's sandboxed filesystem (desktop, documents, projects, downloads)
- `.gitignore` — adds `.starpod/db/` and `home/`

Fails if `.starpod/` already exists in the current directory.

## `starpod dev`

Start the agent in development mode. Opens the browser with auto-login, displays the API key, and serves the web UI + REST API + WebSocket + Telegram (if configured).

```bash
starpod dev
starpod dev --port 8080
```

| Flag | Description |
|------|-------------|
| `--port`, `-p` | Port to serve on (overrides `server_addr` in config) |

At startup, vault secrets are injected into the process environment so the agent can use them.

## `starpod serve`

Start the agent in production mode. Same as `dev` but without opening the browser or displaying the API key.

```bash
starpod serve
```

## `starpod deploy`

Deploy the agent to a remote instance. Currently a stub — coming soon.

```bash
starpod deploy
```

## `starpod chat`

Send a one-shot message, print the response, and exit. If no `.starpod/` is found, creates an ephemeral instance for the message.

```bash
starpod chat "What files are in this directory?"
```

## `starpod repl`

Start an interactive REPL session with readline support and history. Type `exit` or `quit` to end.

```bash
starpod repl
```

## Auth

Authentication for the Starpod platform (used for `starpod deploy`).

### `starpod auth login`

Authenticate with the Starpod platform. Opens a browser for login, or use `--api-key` for non-interactive (CI/headless) login.

```bash
starpod auth login
starpod auth login --api-key sk-... --email user@example.com
```

| Flag | Description |
|------|-------------|
| `--url` | Backend URL (env: `STARPOD_URL`) |
| `--api-key` | API key for non-interactive login |
| `--email` | Email to associate with the API key |

### `starpod auth logout`

Remove saved credentials.

```bash
starpod auth logout
```

### `starpod auth status`

Show current authentication status. Supports `--format json`.

```bash
starpod auth status
starpod --format json auth status
```
