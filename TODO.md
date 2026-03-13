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

- [ ] **Nest utility commands under `agent`** — Move memory, vault, sessions, skills, cron subcommands under `orion agent` (e.g. `orion agent memory search`)
- [ ] **`orion agent apply`** — Sync local `.orion/` config to backend so new instances inherit settings
- [ ] **Instance backend integration** — Connect `orion instance` commands to remote backend for spinning up/managing cloud instances
- [ ] **Hooks crate** — Extract hook logic from agent-sdk into a standalone `orion-hooks` crate so Orion can define its own lifecycle hooks independently of the SDK

## Future / Nice-to-Have

- [ ] **Conversation compression**
- [ ] **Group followup messages**
- [ ] **Multi-provider support** — Trait-based LLM provider abstraction (OpenAI, Gemini, DeepSeek, Ollama, etc.) with per-session model switching
- [ ] **Sandboxed execution** — Docker / Apple Container sandboxing for command execution
- [ ] **Multi-channel access** — Discord integration alongside existing HTTP/WS + CLI + Telegram
- [ ] **Metrics & tracing** — Prometheus metrics, OpenTelemetry tracing for observability
- [ ] **Rate limiting & auth** — Per-IP throttling, proper login/session auth beyond optional API key
