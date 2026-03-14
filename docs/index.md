---
layout: home
hero:
  name: Starpod
  text: Your Local AI Assistant
  tagline: A local-first personal AI assistant platform built in Rust, powered by Claude. Per-project memory, skills, scheduling, and encrypted credentials — no cloud state.
  actions:
    - theme: brand
      text: Get Started
      link: /getting-started/installation
    - theme: alt
      text: View on GitHub
      link: https://github.com/gabrieleventuri/starpod-rs
features:
  - icon: 🧠
    title: Persistent Memory
    details: Markdown files on disk with SQLite FTS5 full-text search. The agent remembers across conversations — personality, user knowledge, daily logs, and a searchable knowledge base.
  - icon: 🔐
    title: Encrypted Vault
    details: AES-256-GCM encrypted credential storage with audit logging. Store API keys, tokens, and secrets that the agent can access securely at runtime.
  - icon: ⚡
    title: Self-Extending Skills
    details: Markdown-based skill files injected into every system prompt. The agent can create, update, and delete its own skills at runtime — teaching itself new behaviors.
  - icon: ⏰
    title: Cron & Scheduling
    details: Interval, cron expression, and one-shot schedules. Jobs run through the full agent pipeline with tool access. Results are delivered via Telegram notifications.
  - icon: 🌐
    title: Multi-Channel
    details: Web UI with streaming, Telegram bot, interactive REPL, one-shot CLI, and a full HTTP/WebSocket API. All channels share the same agent instance and session history.
  - icon: 📁
    title: Project-Scoped
    details: "Each project gets its own .starpod/ directory — isolated config, memory, credentials, and skills. No global state. Works like Git: walks up to find the nearest .starpod/ folder."
---
