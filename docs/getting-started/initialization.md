# Project Setup

Starpod uses a workspace model — each directory where you run `starpod init` gets a `starpod.toml`, `agents/`, and `skills/` directory. Agents are **blueprints** (git-tracked config + personality); runtime state lives in `.instances/` (gitignored).

## Interactive Wizard

```bash
cd your-project
starpod init
```

The wizard walks you through:
1. **Provider selection** — pick from Anthropic, OpenAI, Gemini, Groq, DeepSeek, OpenRouter, or Ollama
2. **Model** — pre-filled with the default for your chosen provider (e.g. `claude-sonnet-4-6` for Anthropic)
3. **API key** — masked input, saved to `.env`. Skipped if already set in your environment or if the provider doesn't need one (Ollama)
4. **First agent** — optionally create your first agent right away with a slug and display name

## Skip the Wizard

```bash
starpod init --default
```

Uses Anthropic / `claude-sonnet-4-6` with no API key and no agent.

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
| `--model` | Claude model to use | `claude-sonnet-4-6` |
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
        │   ├── agent.toml  Copied from blueprint
        │   ├── SOUL.md     Copied from blueprint
        │   ├── .env        From .env.dev (dev) or .env (prod)
        │   ├── db/         SQLite databases
        │   └── users/
        │       └── admin/  Auto-created default user
        │           ├── USER.md
        │           ├── MEMORY.md
        │           └── memory/
        ├── reports/        Agent-created files
        └── ...             Full filesystem sandbox
```

- **`starpod.toml`** — workspace-level defaults shared across all agents.
- **`agents/<name>/agent.toml`** — per-agent overrides (deep-merged on top of workspace config).
- **`.env`** — API key for your chosen provider (e.g. `ANTHROPIC_API_KEY=sk-ant-...`).
- **`.instances/`** — runtime state, never committed. Created automatically by `starpod dev`.

## Multiple Agents

Each agent in the workspace can have its own model, personality, and memory:

```bash
starpod agent new backend-bot --agent-name "Backend Bot" --model "claude-sonnet-4-6"
starpod agent new journal --agent-name "Journal" --soul "You help me reflect on my day"
```

Run a specific agent with:

```bash
starpod dev backend-bot
starpod dev journal --port 3001
```

## Production Deployment

Build a standalone `.starpod/` from a blueprint (no workspace required):

```bash
starpod build --agent agents/my-agent --output /srv/my-agent --env .env
cd /srv/my-agent
starpod serve
```

`starpod serve` walks up from the current directory to find the nearest `.starpod/agent.toml`, so it works from any subdirectory of the deployment target.
