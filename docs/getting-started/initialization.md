# Project Setup

Starpod uses a workspace model — each directory where you run `starpod init` gets a `starpod.toml`, `agents/`, and `skills/` directory.

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

```
your-project/
├── starpod.toml        Workspace config (provider, model, defaults)
├── .env                API key (gitignored)
├── .gitignore          Includes .env and */data/
├── agents/
│   └── my-agent/       (if created during init)
│       ├── agent.toml  Agent-specific overrides
│       ├── SOUL.md     Agent personality
│       ├── USER.md     User profile (starts empty)
│       ├── MEMORY.md   Memory index (starts empty)
│       ├── data/       SQLite databases
│       ├── memory/     Daily logs
│       └── knowledge/  Knowledge base
└── skills/             Shared skills
```

- **`starpod.toml`** — workspace-level defaults shared across all agents.
- **`agents/<name>/agent.toml`** — per-agent overrides (deep-merged on top of workspace config).
- **`.env`** — API key for your chosen provider (e.g. `ANTHROPIC_API_KEY=sk-ant-...`).

## Multiple Agents

Each agent in the workspace can have its own model, personality, and memory:

```bash
starpod agent new backend-bot --agent-name "Backend Bot" --model "claude-sonnet-4-6"
starpod agent new journal --agent-name "Journal" --soul "You help me reflect on my day"
```

Run a specific agent with:

```bash
starpod serve -a backend-bot
starpod repl -a journal
```
