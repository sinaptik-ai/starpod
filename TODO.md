# Starpod RS — TODO

## Completed

- [x] **SQLite Migrations** — Each crate owns its migrations via `sqlx::migrate!("./migrations")` with versioned `.sql` files
- [x] **Skills** — `starpod-skills` crate, filesystem-based `skills/<name>/SKILL.md`, 4 agent tools (SkillCreate/Update/Delete/List), injected into system prompt, CLI subcommands
- [x] **Scheduling / Cron** — `starpod-cron` crate with interval/cron-expression/one-shot schedules, SQLite storage, background scheduler (30s tick), 4 agent tools (CronAdd/List/Remove/Runs), CLI subcommands, auto-start in gateway
- [x] **sqlx migration** — Replaced rusqlite + custom migration runner with sqlx across all 4 DB crates (memory, vault, session, cron). Async `SqlitePool`, connection pooling, built-in migration system
- [x] **Web UI** — Embedded SPA at `/`, streaming WS protocol, minimal dark theme, collapsible tools, clickable URLs
- [x] **Telegram bot** — `starpod-telegram` crate with teloxide, auto-starts alongside gateway when token configured
- [x] **Background Bash** — `run_in_background` support for Bash tool so long-running processes don't block
- [x] **Local-first CLI restructure** — `starpod agent {init, serve, chat, repl}` + `starpod instance {create, list, kill, pause, restart}` stubs. Config from `.starpod/config.toml` per-project, no global config.
- [x] **Agent identity** — `[identity]` config section with `name`, `emoji`, `soul` (personality). Injected into system prompt and used in Telegram /start, CLI header, daily logs.
- [x] **User profile** — `[user]` config section with `name`, `timezone`. Injected into system prompt for personalized responses.
- [x] **Reasoning effort** — `reasoning_effort` config option (low/medium/high) maps to extended thinking budget tokens. Wired through agent-sdk to Claude API.
- [x] **Multi-provider config** — `[providers]` config section with per-provider `api_key`, `base_url`, `models`, `enabled`. `provider` field selects active provider. Currently only Anthropic is implemented.
- [x] **Telegram streaming** — Edit-in-place mode (`stream_mode = "edit_in_place"`) with configurable throttle (`edit_throttle_ms`). Falls back to blocking mode by default.
- [x] **Channel-aware sessions** — Session management scoped by channel (`main`, `telegram`) with per-channel strategies. `main` = explicit sessions (client-controlled via `channel_session_key`), `telegram` = 6h time-gap with auto-close. Multiple concurrent web/REPL sessions supported. Scheduler creates standalone sessions per cron run.

## Planned

### CLI & Config
- [x] **Nest utility commands under `agent`** — Move memory, vault, sessions, skills, cron subcommands under `starpod agent` (e.g. `starpod agent memory search`)
- [ ] **`starpod agent apply`** — Sync local `.starpod/` config (model, tools, skills, system prompt, etc.) to backend so new instances inherit settings
- [ ] **`starpod agent status`** — Show current project config, agent health, DB sizes, active sessions
- [ ] **`.starpod/system_prompt.md`** — Allow custom system prompt per project (loaded from file, merged with defaults)

### Instance Management
- [x] **Instance backend integration** — `starpod-instances` crate with HTTP client connecting to remote backend API. CLI commands (create, list, kill, pause, restart) + gateway API routes. Config via `instance_backend_url` or `STARPOD_INSTANCE_BACKEND_URL` env var.
- [x] **`starpod instance logs <id>`** — Stream logs (newline-delimited JSON) from a running remote instance with colored level output
- [x] **`starpod instance ssh <id>`** — Fetch SSH connection info from backend, spawn native `ssh` process with optional ephemeral key
- [x] **Instance health monitoring** — `HealthMonitor` with configurable heartbeat polling, auto-restart on stale heartbeat, status change callbacks. `starpod instance health <id>` CLI command + `GET /api/instances/:id/health` gateway route.

### Agent Capabilities
- [x] **Conversation compaction** — Summarize/compress older messages when approaching context window limits. Full implementation in `agent-sdk/src/compact.rs` with tool-cycle-aware splitting, configurable `compaction_model`, integrated into agent query loop.
- [ ] **Conversation history / context carry-over** — Load previous session context into new sessions for continuity
- [x] **Group followup messages** — Batch rapid user messages into a single agent turn. Configurable via `followup_mode` (`"inject"` or `"queue"`)
- [x] **Multi-provider implementation** — Trait-based `LlmProvider` abstraction with `AnthropicProvider`, `OpenAiProvider` (also Groq, DeepSeek, OpenRouter, Ollama), and `GeminiProvider`. Runtime switching via `config.provider`. Per-provider cost rates, capabilities, and streaming support.
- [x] **Telegram markdown formatting** — Convert agent response markdown to Telegram MarkdownV2 (escape special chars, map code blocks, bold, italic, links). Uses `ParseMode::MarkdownV2` in starpod-telegram.
- [x] **File attachments** — Image/file uploads in web UI (drag & drop, file picker) and Telegram (photos, documents). Images sent via Claude vision API; non-image files saved to `{data_dir}/downloads/`. 20 MB per-file limit. Claude auto-resizes large images.
- [ ] **MCP (Model Context Protocol) support** — Allow connecting external MCP servers as tool providers. Config structs and builder plumbing exist in `agent-sdk/src/mcp/` but no runtime (no process spawning, connection management, or tool routing).
- [ ] **Loop detection** — Detect repetitive no-progress tool patterns in the agent loop (same tool+params repeated, ping-pong alternation, identical polling outputs). Configurable warning/critical/circuit-breaker thresholds to prevent token waste. Inspired by OpenClaw's guardrail system.
- [ ] **Structured exec approval flow** — Built-in `ask: off | on-miss | always` mode for Bash tool with command allowlists, beyond what hooks can do today. Provides a clear approval UX for shell command execution.
- [ ] **Background process manager** — Dedicated tool to list, poll, log, and kill long-running background Bash sessions. Currently `run_in_background` fires and forgets; this would give the agent visibility into running processes.
- [ ] **Per-provider tool policies** — Restrict which tools are available per provider/model (e.g. give a weaker model fewer tools). Applied after tool presets but before allow/deny lists. Useful for multi-model routing scenarios.

### Infrastructure
- [x] **Hooks crate** — Extract hook logic from agent-sdk into a standalone `starpod-hooks` crate so Starpod can define its own lifecycle hooks independently of the SDK
- [ ] **Sandboxed execution** — OS-level sandboxing for Bash tool (`sandbox-exec` on macOS, `bwrap`/`firejail` on Linux) to enforce file-system boundaries at the kernel level. Currently the path boundary in `ToolExecutor` only guards Read/Write/Edit/Glob/Grep; Bash can still access anything via shell commands.
- [ ] **Metrics & tracing** — Prometheus metrics, OpenTelemetry tracing for observability
- [ ] **Rate limiting & auth** — Per-IP throttling, proper login/session auth beyond optional API key
- [ ] **Multi-channel access** — Discord, Slack, WhatsApp integrations alongside existing HTTP/WS + CLI + Telegram. Channel enum and session routing infrastructure is in place — add new `Channel` variants.
- [ ] **Scheduler channel routing** — Allow cron jobs to route into existing channel sessions (e.g. attach to telegram conversation via `channel = "auto"` config) instead of creating standalone sessions
- [ ] **Persistent agent mode** — Long-running daemon that watches files/events and acts proactively (not just on user messages)
- [ ] **Plugin system** — Load custom tools from external crates or WASM modules at runtime
- [ ] **Provider failover** — Automatic failover to backup provider when primary is down or rate-limited
- [ ] **Voice support** — TTS/STT integration for voice interaction (ElevenLabs, OpenAI, local Piper)

### Web UI
- [x] **Conversation history sidebar** — Browse and resume past sessions
- [ ] **Settings panel** — Edit config, manage API keys, view usage from the UI
- [x] **File upload** — Drag & drop / paperclip button, base64 over WS, 20 MB limit, preview thumbnails
- [ ] **Downloads cleanup policy** — Optional config to auto-delete old downloads (e.g. `downloads_retention_days` in config.toml)
- [x] **Mobile responsive** — Media queries at 768px breakpoint, sidebar/preview as full-screen overlays on mobile, Tailwind responsive utilities, `100dvh` for mobile viewports.
- [ ] **Markdown rendering** — Full markdown support (tables, lists, headings, etc.)
