---
layout: home
hero:
  name: Starpod
  text: Deploy AI Agents. Skip the Infra.
  tagline: One agent definition. Infinite isolated instances. Memory, skills, scheduling, and encrypted credentials — built in Rust, deployed in minutes.
  actions:
    - theme: brand
      text: Get Started
      link: /getting-started/installation
    - theme: alt
      text: View on GitHub
      link: https://github.com/sinaptik-ai/starpod
features:
  - icon: 🧠
    title: Persistent Memory
    details: Markdown on disk with SQLite FTS5 search. Your agent remembers across conversations — personality, user context, daily logs. No external database required.
  - icon: 🔐
    title: Secrets Vault
    details: AES-256-GCM encrypted credential storage with audit logging. API keys, tokens, and secrets your agent accesses at runtime — isolated per instance, never shared.
  - icon: ⚡
    title: Self-Extending Skills
    details: Markdown skill files injected into every prompt. The agent creates, updates, and deletes its own skills at runtime — teaching itself new behaviors without redeployment.
  - icon: ⏰
    title: Cron & Scheduling
    details: Interval, cron, and one-shot schedules. Jobs run through the full agent pipeline with tool access. Results delivered via configured channels.
  - icon: 🌐
    title: Multi-Channel
    details: Web UI, Telegram, REPL, CLI, HTTP/WebSocket API. All channels share the same agent instance and session history. Users interact through channels they already use.
  - icon: 📁
    title: Full Isolation
    details: "Each instance gets its own .starpod/ directory — config, memory, vault, and skills. No cross-contamination. No global state. Works like Git: walks up to find the nearest .starpod/ folder."
---
