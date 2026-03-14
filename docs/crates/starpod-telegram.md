# starpod-telegram

Telegram bot interface built with [teloxide](https://github.com/teloxide/teloxide).

## API

```rust
// Standalone (creates its own agent)
starpod_telegram::run(config, token).await?;

// Shared agent (co-hosted with gateway)
starpod_telegram::run_with_agent(agent, token).await?;

// With user allowlist
starpod_telegram::run_with_agent_filtered(agent, token, allowed_users).await?;

// Send a notification (for cron job results)
starpod_telegram::send_notification(&token, &user_ids, "Job completed").await;
```

## Features

- **Typing indicator** — shown while processing
- **Allowlist** — only specified user IDs can chat
- **Markdown conversion** — converts to Telegram HTML
- **Message splitting** — respects 4096-char limit, prefers line boundaries
- **Fallback** — plain text if HTML parsing fails
- **Two streaming modes** — `final_only` (default) or `all_messages`
- **Cron notifications** — delivers job results to allowed users

## Markdown → Telegram HTML

| Input | Output |
|-------|--------|
| `` ```code``` `` | `<pre>code</pre>` |
| `` `inline` `` | `<code>inline</code>` |
| `**bold**` | `<b>bold</b>` |
| `*italic*` | `<i>italic</i>` |
| `~~strike~~` | `<s>strike</s>` |
| `# Heading` | `<b>Heading</b>` |
| `[text](url)` | `<a href="url">text</a>` |

## Session Routing

- Channel: `Telegram`
- Session key: chat ID as string
- 6-hour inactivity timeout — auto-closes old session

## Tests

4 unit tests.
