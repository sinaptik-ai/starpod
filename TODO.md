# Orion RS — TODO

## Completed

- [x] **SQLite Migrations** — `orion-core::migrate` module with namespace-scoped versioned migrations, transaction-per-migration, all 3 DB crates converted
- [x] **Skills** — `orion-skills` crate, filesystem-based `skills/<name>/SKILL.md`, 4 agent tools (SkillCreate/Update/Delete/List), injected into system prompt, CLI subcommands
- [x] **Scheduling / Cron** — `orion-cron` crate with interval/cron-expression/one-shot schedules, SQLite storage, background scheduler (30s tick), 4 agent tools (CronAdd/List/Remove/Runs), CLI subcommands, auto-start in gateway

## Future / Nice-to-Have

- [ ] **Multi-provider support** — Trait-based LLM provider abstraction (OpenAI, Gemini, DeepSeek, Ollama, etc.) with per-session model switching
- [ ] **Sandboxed execution** — Docker / Apple Container sandboxing for command execution
- [ ] **Multi-channel access** — Telegram, Discord integrations alongside existing HTTP/WS + CLI
- [ ] **Metrics & tracing** — Prometheus metrics, OpenTelemetry tracing for observability
- [ ] **Rate limiting & auth** — Per-IP throttling, proper login/session auth beyond optional API key
