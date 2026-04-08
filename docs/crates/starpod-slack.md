# starpod-slack

Slack bot interface built on Slack's [Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/). The bot opens an outbound WebSocket — no public URL, webhook, or signature verification required — so it works identically on a laptop, in `starpod dev`, and on an ephemeral VM with no inbound networking.

## API

```rust
// Shared agent (co-hosted with gateway)
starpod_slack::run_with_agent_and_auth(agent, auth, app_token, bot_token).await?;

// Lower-level: bring your own envelope handler
let handler = Arc::new(starpod_slack::LoggingHandler);
starpod_slack::run_with_handler(&app_token, handler).await?;
```

`run_with_agent_and_auth` only returns on a non-retriable error (invalid tokens, `auth.test` failure). Transient transport failures are retried internally with exponential backoff (1s → 2s → 4s → 8s → 16s → 30s).

## Tokens

Slack Socket Mode requires two tokens, both stored in the encrypted vault:

| Env var | Prefix | Purpose |
|---------|--------|---------|
| `SLACK_APP_TOKEN` | `xapp-` | Opens the Socket Mode WebSocket via `apps.connections.open` |
| `SLACK_BOT_TOKEN` | `xoxb-` | All outbound Web API calls (`chat.postMessage`, `auth.test`, file downloads) |

The crate validates the `xapp-` prefix client-side before any network call so a swapped token fails immediately with a clear error.

## Features

- **Socket Mode transport** — outbound WebSocket, automatic ack within Slack's ~3s window
- **Reconnect loop** — graceful handling of Slack-initiated `disconnect` envelopes (immediate reconnect) and transport failures (capped exponential backoff)
- **Event filtering** — only `app_mention` and `message.im` (DMs) trigger an agent turn; everything else is logged and dropped
- **Self-loop guard** — drops events authored by the bot's own `bot_user_id`
- **Per-event dedup** — SQLite-backed (`slack_events_seen`), survives reconnect races; background sweeper deletes rows older than 24h
- **Threaded replies** — every reply is posted into the triggering message's thread (or starts one)
- **File uploads** — downloads attachments through the bot token, validates them against `[attachments]`, and forwards them to the agent (vision-capable models)
- **Markdown → mrkdwn** — converts standard Markdown to Slack's mrkdwn dialect
- **Message splitting** — chunks long messages (~3,500 chars per chunk) at paragraph or line boundaries, never inside a code fence
- **Two streaming modes** — `final_only` (default) or `all_messages`

## Markdown → Slack mrkdwn

| Input | Output |
|-------|--------|
| `**bold**` | `*bold*` |
| `*italic*` | `_italic_` |
| `~~strike~~` | `~strike~` |
| `# Heading` | `*Heading*` |
| `[text](url)` | `<url\|text>` |
| `` `inline` `` | `` `inline` `` |
| `` ```code``` `` | `` ```code``` `` |

## Session Routing

- Channel: `Slack`
- Session key: `{team_id}:{channel_id}:{thread_root_ts}` — every Slack thread is its own continuous conversation
- 6-hour inactivity timeout by default — auto-closes the previous session

## Tests

54 unit tests + 1 doc-test covering:

- Envelope parsing for every Socket Mode variant + forward-compat fallback
- Backoff schedule monotonicity / cap
- App-token prefix validation
- Markdown → mrkdwn conversion (bold, italic, lists, links, code, multi-byte UTF-8)
- Message splitting at paragraph boundaries
- Self-loop guard / mention stripping
- Dedup insert + sweeper
- WebSocket ticket scrubbing for safe logging
- Error retriability classification
