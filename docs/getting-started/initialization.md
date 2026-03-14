# Project Setup

Starpod is project-scoped — each directory where you run `starpod agent init` gets its own `.starpod/` folder with config, memory, credentials, and skills.

## Interactive Wizard

```bash
cd your-project
starpod agent init
```

The wizard walks you through:
- Your name and timezone
- Agent name and personality
- Model selection
- Optional Telegram bot setup

## Skip the Wizard

```bash
starpod agent init --default
```

## Custom Flags

```bash
starpod agent init \
  --name "Alice" \
  --timezone "Europe/Rome" \
  --agent-name "Jarvis" \
  --soul "You are a helpful coding assistant" \
  --model "claude-opus-4-6"
```

### Available Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--name` | Your display name | System username |
| `--timezone` | IANA timezone | Auto-detected |
| `--agent-name` | Agent's display name | `Aster` |
| `--soul` | Personality/instructions | Empty |
| `--model` | Claude model to use | `claude-haiku-4-5` |
| `--default` | Skip the wizard | — |

## What Gets Created

```
.starpod/
├── config.toml      Project configuration
└── data/
    ├── SOUL.md      Agent personality (from --soul or wizard)
    ├── USER.md      Your name and info
    ├── MEMORY.md    General knowledge (starts empty)
    ├── memory/      Daily conversation logs
    ├── knowledge/   Knowledge base documents
    └── skills/      Skill definitions
```

## Multiple Projects

Each project is fully independent. Different agents, different personalities, different memory:

```bash
cd ~/work/backend
starpod agent init --agent-name "Backend Bot" --model "claude-sonnet-4-6"

cd ~/personal/notes
starpod agent init --agent-name "Journal" --soul "You help me reflect on my day"
```

Starpod walks up from the current directory to find the nearest `.starpod/` folder, just like Git finds `.git/`.
