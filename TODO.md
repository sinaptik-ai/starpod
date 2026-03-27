# Starpod CLI Simplification — TODO

## Implemented
- [x] `starpod init` — bootstrap `.starpod/` in current folder, accept `--name`, `--model`, `--env KEY=VAL`
- [x] `starpod dev` — start local server (HTTP + WS + Telegram if configured)
- [x] `starpod serve` — production mode
- [x] `starpod deploy` — stub (coming soon)
- [x] `starpod repl` — interactive terminal chat (rustyline)
- [x] `starpod chat "msg"` — one-off message, print response, exit
- [x] `starpod auth {login,logout,status}` — authentication (kept from old CLI)

## Deferred — CLI
- [ ] `starpod logs` — tail logs of running instance (local or deployed)
- [ ] `starpod status` — health check, last activity, memory/session stats
- [ ] `starpod env set KEY=VAL` / `starpod env list` — manage vault secrets after init
- [ ] `starpod init --template <name>` — initialize from a template

## Deferred — UI Onboarding Wizard

When a user runs `starpod dev` and the agent is not fully configured (e.g. no
API key in vault), the web UI should launch a guided onboarding wizard:

- **Screen 1: Identity** — Agent name (display name; slug auto-generated from it)
  and optional description. Pre-filled from `agent.toml` if already set.

- **Screen 2: Model & Keys** — Pick model (provider/model format from the model
  registry), provide the corresponding API key, optionally add BRAVE_API_KEY for
  web search. All keys stored directly in the vault.

- **Screen 3: Personality** — Either pick "base" (skip) or write a free-form prompt
  describing the agent's personality/purpose. If a prompt is given, the agent
  auto-generates SOUL.md, HEARTBEAT.md, config adjustments, and starter skills
  following best practices.

- **Screen 4: Skills** — Recap of all skills (generated + existing) with toggle
  on/off for each. For each skill, any environment variables declared in the
  skill's metadata (`env.secrets` / `env.variables`) must be provided by the user
  and are stored in the vault.

- **Final: Done** — Agent fully configured. Redirect to chat. All values written
  to `.starpod/config/` files and `.starpod/db/vault.db`.

## Removed (from old CLI)
- `agent` subcommand (new/list/push/pull/diff) — no more blueprint/template concept
- `instance` subcommand — agent IS the instance now
- `secret` subcommand — secrets are instance-specific (vault), managed via `env` (deferred)
- `build` subcommand — no blueprint to build
- `memory` subcommand — accessible through chat/UI tools
- `sessions` subcommand — accessible through chat/UI tools
- `skill` subcommand — accessible through chat/UI tools
- `cron` subcommand — accessible through chat/UI tools
