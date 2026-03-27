# Project Setup

Starpod uses a flat model — run `starpod init` in any directory to bootstrap an agent. Everything lives in a `.starpod/` directory. No workspace files, no blueprints, no separate instances.

## Initialize

```bash
cd your-project
starpod init
```

This creates a ready-to-run agent with default settings (Anthropic / `claude-haiku-4-5`, agent name "Aster").

## Customize with Flags

```bash
# Custom name and model
starpod init --name "Jarvis" --model openai/gpt-4o

# Seed secrets into the vault
starpod init --env ANTHROPIC_API_KEY=sk-ant-... --env BRAVE_API_KEY=...

# All together
starpod init --name "Ada" --model anthropic/claude-haiku-4-5 --env ANTHROPIC_API_KEY=sk-ant-...
```

### Available Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--name` | Agent display name | `Aster` |
| `--model` | Model in `provider/model` format | `anthropic/claude-haiku-4-5` |
| `--env KEY=VAL` | Seed a secret into the vault (repeatable) | — |

## What Gets Created

```
your-project/
├── .starpod/
│   ├── config/                 Agent configuration (git-tracked)
│   │   ├── agent.toml         Main config (models, server_addr, etc.)
│   │   ├── SOUL.md            Agent personality
│   │   ├── HEARTBEAT.md       Periodic self-reflection (empty by default)
│   │   ├── BOOT.md            Boot instructions (empty by default)
│   │   ├── BOOTSTRAP.md       First-run instructions (empty by default)
│   │   └── frontend.toml     Web UI config
│   ├── skills/                 Agent skills
│   ├── db/                     SQLite databases (gitignored)
│   │   └── vault.db           Encrypted secrets (created if --env is used)
│   └── users/                  Per-user data
├── home/                       Agent's sandboxed filesystem (gitignored)
│   ├── desktop/
│   ├── documents/
│   ├── projects/
│   └── downloads/
└── .gitignore                  Excludes .starpod/db/ and home/
```

## Secrets Management

All secrets live in the **encrypted vault** (`vault.db`). There are no `.env` files.

Seed secrets at init time:

```bash
starpod init --env ANTHROPIC_API_KEY=sk-ant-... --env TELEGRAM_BOT_TOKEN=123:ABC...
```

Or manage them later through the web UI Settings page after running `starpod dev`.

At startup (`dev`, `serve`, `repl`, `chat`), vault contents are automatically injected into the process environment so the agent and its tools can use them.

## Running the Agent

```bash
# Development mode (opens browser, shows API key)
starpod dev

# Production mode (no browser, no API key display)
starpod serve

# Terminal chat
starpod repl
starpod chat "Hello!"
```

## What's Next?

- [Configuration](/getting-started/configuration) — customize the model, personality, and more
- [Memory](/concepts/memory) — learn how Starpod remembers across conversations
- [Skills](/concepts/skills) — teach your agent new abilities
- [Telegram](/integrations/telegram) — connect Starpod to Telegram
