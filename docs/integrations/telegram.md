# Telegram Bot

Starpod can run as a Telegram bot, sharing the same agent instance as the web UI and API.

## Setup

### 1. Create a Bot with BotFather

1. Open Telegram and search for `@BotFather`
2. Send `/newbot`
3. Choose a name (e.g. "My Starpod Assistant")
4. Choose a username (must end in `bot`, e.g. `my_starpod_bot`)
5. Copy the token (e.g. `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`)

### 2. Add the Token

Store the token in the vault via `starpod init` or the web UI Settings page:

```bash
# At init time
starpod init --env TELEGRAM_BOT_TOKEN=123456789:ABCdefGHIjklMNOpqrsTUVwxyz

# Or manage later via Settings > Channels in the web UI
```

The token is stored in the encrypted vault — never in plaintext config files.

### 3. Restrict Access

Send `/start` to your bot — it replies with your user ID. Add it to config:

```toml
[channels.telegram]
allowed_users = [123456789]
```

Multiple users: `allowed_users = [123456789, 987654321]`

The bot won't respond to anyone until you add at least one user ID. `/start` is the only command that works without being allowlisted (so you can discover your ID).

### 4. Start the Server

```bash
starpod dev
```

You should see `Telegram  connected` in the startup banner.

## Streaming Modes

| Mode | Behavior |
|------|----------|
| `final_only` (default) | Waits for all agent turns to complete, sends the final message |
| `all_messages` | Sends each assistant message immediately as it arrives |

```toml
[channels.telegram]
stream_mode = "all_messages"
```

## Features

- **Typing indicator** — shown while the agent is thinking
- **File uploads** — send photos (vision) and documents (saved to `{project_root}/downloads/`). Configurable via [`[attachments]`](/getting-started/configuration#attachments) settings
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

Telegram uses **time-gap sessions**: messages within 6 hours continue the same session. After a gap, the old session is auto-closed and a new one starts. The session key is the Telegram chat ID. The gap is configurable via `gap_minutes` in `[channels.telegram]` (default: 360 minutes / 6 hours).

## Optional: Customize in BotFather

- `/setdescription` — what users see before starting a chat
- `/setabouttext` — bio on the bot's profile
- `/setuserpic` — profile picture
- `/setcommands` — register `/start` with a description
