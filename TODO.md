# Orion RS — TODO

## Completed

- [x] **SQLite Migrations** — Each crate owns its migrations via `sqlx::migrate!("./migrations")` with versioned `.sql` files
- [x] **Skills** — `orion-skills` crate, filesystem-based `skills/<name>/SKILL.md`, 4 agent tools (SkillCreate/Update/Delete/List), injected into system prompt, CLI subcommands
- [x] **Scheduling / Cron** — `orion-cron` crate with interval/cron-expression/one-shot schedules, SQLite storage, background scheduler (30s tick), 4 agent tools (CronAdd/List/Remove/Runs), CLI subcommands, auto-start in gateway
- [x] **sqlx migration** — Replaced rusqlite + custom migration runner with sqlx across all 4 DB crates (memory, vault, session, cron). Async `SqlitePool`, connection pooling, built-in migration system

## Planned

- [ ] **Hooks crate** — Extract hook logic from agent-sdk into a standalone `orion-hooks` crate so Orion can define its own lifecycle hooks independently of the SDK
- [ ] **Gateway auto-start** — Figure out a way to auto-start the gateway (launchd on macOS, systemd on Linux, or a background daemon mode)

## Future / Nice-to-Have

- [ ] **Conversation compression**
- [ ] **Group followup messages**
- [ ] **Multi-provider support** — Trait-based LLM provider abstraction (OpenAI, Gemini, DeepSeek, Ollama, etc.) with per-session model switching
- [ ] **Sandboxed execution** — Docker / Apple Container sandboxing for command execution
- [ ] **Multi-channel access** — Telegram, Discord integrations alongside existing HTTP/WS + CLI
- [ ] **Metrics & tracing** — Prometheus metrics, OpenTelemetry tracing for observability
- [ ] **Rate limiting & auth** — Per-IP throttling, proper login/session auth beyond optional API key
