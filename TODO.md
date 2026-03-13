# Orion RS — TODO

## Completed

- [x] **SQLite Migrations** — Each crate owns its migrations via `sqlx::migrate!("./migrations")` with versioned `.sql` files
- [x] **Skills** — `orion-skills` crate, filesystem-based `skills/<name>/SKILL.md`, 4 agent tools (SkillCreate/Update/Delete/List), injected into system prompt, CLI subcommands
- [x] **Scheduling / Cron** — `orion-cron` crate with interval/cron-expression/one-shot schedules, SQLite storage, background scheduler (30s tick), 4 agent tools (CronAdd/List/Remove/Runs), CLI subcommands, auto-start in gateway
- [x] **sqlx migration** — Replaced rusqlite + custom migration runner with sqlx across all 4 DB crates (memory, vault, session, cron). Async `SqlitePool`, connection pooling, built-in migration system
- [x] **Web UI** — Embedded SPA at `/`, streaming WS protocol, minimal dark theme, collapsible tools, clickable URLs
- [x] **Telegram bot** — `orion-telegram` crate with teloxide, auto-starts alongside gateway when token configured
- [x] **Background Bash** — `run_in_background` support for Bash tool so long-running processes don't block
- [x] **Local-first CLI restructure** — `orion agent {init, serve, chat, repl}` + `orion instance {create, list, kill, pause, restart}` stubs. Config from `.orion/config.toml` per-project, no global config.

## Planned

### CLI & Config
- [ ] **Nest utility commands under `agent`** — Move memory, vault, sessions, skills, cron subcommands under `orion agent` (e.g. `orion agent memory search`)
- [ ] **`orion agent apply`** — Sync local `.orion/` config (model, tools, skills, system prompt, etc.) to backend so new instances inherit settings
- [ ] **`orion agent status`** — Show current project config, agent health, DB sizes, active sessions
- [ ] **`.orion/system_prompt.md`** — Allow custom system prompt per project (loaded from file, merged with defaults)

### Instance Management
- [ ] **Instance backend integration** — Connect `orion instance` commands to remote backend API for spinning up/managing cloud instances
- [ ] **`orion instance logs <id>`** — Stream logs from a running remote instance
- [ ] **`orion instance ssh <id>`** — Open a shell into a remote instance
- [ ] **Instance health monitoring** — Heartbeat, auto-restart on crash, resource usage tracking

### Agent Capabilities
- [ ] **Conversation compression** — Summarize old turns to stay within context window on long sessions
- [ ] **Conversation history / context carry-over** — Load previous session context into new sessions for continuity
- [ ] **Group followup messages** — Batch rapid user messages into a single agent turn
- [ ] **Multi-provider support** — Trait-based LLM provider abstraction (OpenAI, Gemini, DeepSeek, Ollama, etc.) with per-project model switching
- [ ] **Streaming for Telegram** — Send partial responses as edits instead of waiting for full response
- [ ] **File attachments** — Support image/file uploads in web UI and Telegram (vision, document analysis)
- [ ] **MCP (Model Context Protocol) support** — Allow connecting external MCP servers as tool providers

### Infrastructure
- [ ] **Hooks crate** — Extract hook logic from agent-sdk into a standalone `orion-hooks` crate so Orion can define its own lifecycle hooks independently of the SDK
- [ ] **Sandboxed execution** — Docker / Apple Container sandboxing for command execution
- [ ] **Metrics & tracing** — Prometheus metrics, OpenTelemetry tracing for observability
- [ ] **Rate limiting & auth** — Per-IP throttling, proper login/session auth beyond optional API key
- [ ] **Multi-channel access** — Discord, Slack integrations alongside existing HTTP/WS + CLI + Telegram
- [ ] **Persistent agent mode** — Long-running daemon that watches files/events and acts proactively (not just on user messages)
- [ ] **Plugin system** — Load custom tools from external crates or WASM modules at runtime

### Web UI
- [ ] **Conversation history sidebar** — Browse and resume past sessions
- [ ] **Settings panel** — Edit config, manage API keys, view usage from the UI
- [ ] **File upload** — Drag & drop files into chat
- [ ] **Mobile responsive** — Better layout on small screens
- [ ] **Markdown rendering** — Full markdown support (tables, lists, headings, etc.)
