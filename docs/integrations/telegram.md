# Telegram Bot

Orion can run as a Telegram bot, sharing the same agent instance as the web UI and API.

## Setup

### 1. Create a Bot with BotFather

1. Open Telegram and search for `@BotFather`
2. Send `/newbot`
3. Choose a name (e.g. "My Orion Assistant")
4. Choose a username (must end in `bot`, e.g. `my_orion_bot`)
5. Copy the token (e.g. `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`)

### 2. Add the Token

Either in `.orion/config.toml`:

```toml
[telegram]
bot_token = "123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
```

Or as an environment variable:

```bash
export TELEGRAM_BOT_TOKEN="123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
```

::: tip
`orion agent init` can set this up during the interactive wizard.
:::

### 3. Restrict Access

Send `/start` to your bot — it replies with your user ID. Add it to config:

```toml
[telegram]
allowed_users = [123456789]
```

Multiple users: `allowed_users = [123456789, 987654321]`

The bot won't respond to anyone until you add at least one user ID. `/start` is the only command that works without being allowlisted (so you can discover your ID).

### 4. Start the Server

```bash
orion agent serve
```

You should see `Telegram  connected` in the startup banner.

## Streaming Modes

| Mode | Behavior |
|------|----------|
| `final_only` (default) | Waits for all agent turns to complete, sends the final message |
| `all_messages` | Sends each assistant message immediately as it arrives |

```toml
[telegram]
stream_mode = "all_messages"
```

## Features

- **Typing indicator** — shown while the agent is thinking
- **File uploads** — send photos (vision) and documents (saved to `{data_dir}/downloads/`). Max 20 MB per file
- **Markdown rendering** — converts to Telegram HTML format
- **Message splitting** — splits at line boundaries for Telegram's 4096-char limit
- **Fallback** — sends plain text if HTML parsing fails
- **Cron notifications** — job results are delivered via Telegram

## Markdown Conversion

| Markdown | Telegram HTML |
|----------|--------------|
| `` ```code``` `` | `<pre>code</pre>` |
| `` `inline` `` | `<code>inline</code>` |
| `**bold**` | `<b>bold</b>` |
| `*italic*` | `<i>italic</i>` |
| `~~strike~~` | `<s>strike</s>` |
| `# Heading` | `<b>Heading</b>` |
| `[text](url)` | `<a href="url">text</a>` |

## Session Behavior

Telegram uses **time-gap sessions**: messages within 6 hours continue the same session. After a gap, the old session is auto-closed and a new one starts. The session key is the Telegram chat ID.

## Optional: Customize in BotFather

- `/setdescription` — what users see before starting a chat
- `/setabouttext` — bio on the bot's profile
- `/setuserpic` — profile picture
- `/setcommands` — register `/start` with a description
