# Project Setup

Starpod uses a workspace model — each directory where you run `starpod init` gets a `starpod.toml`, `agents/`, and `skills/` directory. Agents are **blueprints** (git-tracked config + personality); runtime state lives in `.instances/` (gitignored).

## Interactive Wizard

```bash
cd your-project
starpod init
```

The wizard walks you through:
1. **Provider selection** — pick from Anthropic, OpenAI, Gemini, Groq, DeepSeek, OpenRouter, or Ollama
2. **Model** — pre-filled with the default for your chosen provider (e.g. `claude-haiku-4-5` for Anthropic)
3. **API key** — masked input, saved to `.env`. Skipped if already set in your environment or if the provider doesn't need one (Ollama)
4. **First agent** — optionally create your first agent right away with a slug and display name

## Skip the Wizard

```bash
starpod init --default
```

Uses Anthropic / `claude-haiku-4-5` with no API key and no agent.

## Create Agents

After initializing, create agents with:

```bash
starpod agent new my-agent
starpod agent new my-agent --agent-name "Jarvis" --soul "You are a coding assistant" --model "claude-opus-4-6"
```

### Available Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--agent-name` | Agent's display name | Agent name |
| `--soul` | Personality/instructions | Empty |
| `--model` | Claude model to use | `claude-haiku-4-5` |
| `--default` | Skip the wizard | — |

## What Gets Created

### Workspace (git-tracked)

```
your-project/
├── starpod.toml          Workspace config (provider, model, defaults)
├── .env                  API key (gitignored)
├── .gitignore            Includes .env, .instances/, */db/
├── agents/
│   └── my-agent/         BLUEPRINT (git-tracked)
│       ├── agent.toml    Agent-specific overrides
│       ├── SOUL.md       Agent personality
│       └── files/        Template files synced to instance
└── skills/               Shared skills
```

### Runtime (gitignored, created by `starpod dev`)

```
your-project/
└── .instances/
    └── my-agent/           Agent's filesystem sandbox
        ├── .starpod/       Internal state (like .git/)
        │   ├── .env        Secrets (from .env.dev or .env)
        │   ├── config/     Blueprint-managed (overwritten on build)
        │   │   ├── agent.toml
        │   │   ├── SOUL.md
        │   │   ├── HEARTBEAT.md
        │   │   ├── BOOT.md
        │   │   └── BOOTSTRAP.md
        │   ├── skills/     Merged on build
        │   ├── db/         SQLite databases (runtime)
        │   └── users/
        │       ├── admin/  Auto-created default users (runtime)
        │       │   ├── USER.md
        │       │   ├── MEMORY.md
        │       │   └── memory/
        │       └── user/
        │           ├── USER.md
        │           ├── MEMORY.md
        │           └── memory/
        ├── reports/        Agent-created files
        └── ...             Full filesystem sandbox
```

- **`starpod.toml`** — workspace-level defaults shared across all agents.
- **`agents/<name>/`** — agent **blueprints** (git-tracked). Each contains `agent.toml`, `SOUL.md`, and optional lifecycle files. This is the source of truth for what the agent *is*.
- **`.env`** — API key for your chosen provider (e.g. `ANTHROPIC_API_KEY=sk-ant-...`).
- **`.instances/`** — agent **instances** (gitignored). Created automatically by `starpod dev`. Contains databases, memory, user data — everything the agent accumulates at runtime. Blueprint files are copied into `.starpod/config/` and refreshed on every `starpod dev`, but runtime data (`db/`, `users/`) is always preserved.

## Multiple Agents

Each agent in the workspace can have its own model, personality, and memory:

```bash
starpod agent new backend-bot --agent-name "Backend Bot" --model "claude-haiku-4-5"
starpod agent new journal --agent-name "Journal" --soul "You help me reflect on my day"
```

Run a specific agent with:

```bash
starpod dev backend-bot
starpod dev journal --port 3001
```

## Production Deployment

In production, there's no workspace — you build a standalone instance directly from a blueprint:

```bash
starpod build --agent agents/my-agent --output /srv/my-agent --env .env
cd /srv/my-agent
starpod serve
```

`starpod build` takes the blueprint and creates a self-contained `.starpod/` directory with `config/` (from the blueprint), `skills/`, `db/`, and `users/`. The `.env` is copied once via `--env`. On subsequent builds to the same output, `config/` is refreshed but runtime data is preserved — same semantics as `starpod dev`.

`starpod serve` walks up from the current directory to find the nearest `.starpod/config/agent.toml`, so it works from any subdirectory of the deployment target.
