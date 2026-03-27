# Quick Start

Get Starpod running in under 2 minutes.

## 1. Initialize

```bash
cd your-project
starpod init
```

Or with an API key:

```bash
starpod init --env ANTHROPIC_API_KEY="sk-ant-..."
```

## 2. Start the server

```bash
starpod dev
```

```
  ╭──────────────────────────────────────────╮
  │       Nova  ·  AI Assistant              │
  ╰──────────────────────────────────────────╯

  Server 127.0.0.1:3000
  API Key sp-abc123...
```

Your browser opens automatically. Start chatting.

## Alternative: CLI

One-shot message:

```bash
starpod chat "What files are in this directory?"
```

Interactive REPL:

```bash
starpod repl
```

## What's Next?

- [Configuration](/getting-started/configuration) — customize the model, personality, and more
- [Memory](/concepts/memory) — learn how Starpod remembers across conversations
- [Skills](/concepts/skills) — teach your agent new abilities
- [Telegram](/integrations/telegram) — connect Starpod to Telegram
