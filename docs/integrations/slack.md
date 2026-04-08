# Slack Bot

Starpod runs as a Slack bot via [Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/), sharing the same agent instance as the web UI and API. Socket Mode uses an outbound WebSocket — your Starpod instance never needs a public URL, webhook, or signing-secret verification.

## Setup

The fastest path is the in-app guided setup: open **Settings → Connectors → Slack** in the web UI. The wizard walks you through manifest install, token generation, and validation in four steps. The instructions below are the manual equivalent for users who prefer the CLI.

### 1. Create a Slack App from Manifest

1. Open [https://api.slack.com/apps](https://api.slack.com/apps) and click **Create New App → From a manifest**.
2. Choose the workspace where you want the bot installed.
3. Paste the manifest below (the in-app wizard provides a one-click copy):

```json
{
  "display_information": { "name": "Starpod", "description": "Personal AI assistant" },
  "features": {
    "bot_user": { "display_name": "Starpod", "always_online": true }
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "app_mentions:read",
        "channels:history",
        "channels:join",
        "channels:read",
        "chat:write",
        "chat:write.public",
        "files:read",
        "files:write",
        "groups:history",
        "groups:read",
        "im:history",
        "im:read",
        "im:write",
        "mpim:history",
        "mpim:read",
        "users:read"
      ]
    }
  },
  "settings": {
    "event_subscriptions": {
      "bot_events": ["app_mention", "message.im"]
    },
    "interactivity": { "is_enabled": true },
    "org_deploy_enabled": false,
    "socket_mode_enabled": true,
    "token_rotation_enabled": false
  }
}
```

4. Click **Create**, then **Install to Workspace** and approve the scopes.

### 2. Generate Both Tokens

Slack Socket Mode requires two tokens — they live in different parts of the admin UI:

| Token | Where |
|-------|-------|
| `SLACK_APP_TOKEN` (`xapp-…`) | **Basic Information → App-Level Tokens → Generate Token and Scopes** with the `connections:write` scope |
| `SLACK_BOT_TOKEN` (`xoxb-…`) | **OAuth & Permissions → Bot User OAuth Token** (visible after install) |

### 3. Save the Tokens

Store both in the encrypted vault via `starpod init` or the web UI Settings page:

```bash
starpod init \
  --env SLACK_APP_TOKEN=xapp-1-... \
  --env SLACK_BOT_TOKEN=xoxb-...
```

Tokens are stored encrypted — never in plaintext config files.

### 4. Enable the Channel and Start the Server

```toml
[channels.slack]
enabled = true
# Optional — defaults shown
gap_minutes = 360          # 6h inactivity timeout per thread
stream_mode = "final_only" # or "all_messages"
```

```bash
starpod dev
```

You should see `slack bot authenticated` followed by `slack hello — ready to receive events` in the logs. Mention the bot in any channel it's been added to (`@Starpod hello`) or send it a DM.

## Streaming Modes

| Mode | Behavior |
|------|----------|
| `final_only` (default) | Waits for all agent turns to complete, posts the final message |
| `all_messages` | Posts each assistant message immediately as the stream arrives |

## Features

- **Socket Mode** — outbound WebSocket, no inbound networking required
- **Threaded replies** — every reply is posted into the triggering message's thread
- **DMs and `@mentions`** — both supported; everything else is filtered out
- **File uploads** — images and documents are downloaded with the bot token and forwarded to vision-capable models. Configurable via [`[attachments]`](/getting-started/configuration#attachments)
- **Markdown rendering** — converts to Slack's mrkdwn dialect with code-block preservation
- **Message splitting** — respects Slack's per-message limit (~3,500 chars per chunk to leave headroom for code-fence framing) at paragraph boundaries
- **Per-event dedup** — survives reconnect races; SQLite-backed sweeper trims old rows automatically
- **Self-loop guard** — drops events authored by the bot itself

## Markdown Conversion

| Markdown | Slack mrkdwn |
|----------|--------------|
| `**bold**` | `*bold*` |
| `*italic*` | `_italic_` |
| `~~strike~~` | `~strike~` |
| `# Heading` | `*Heading*` |
| `[text](url)` | `<url\|text>` |
| `` `inline` `` | `` `inline` `` |
| `` ```code``` `` | `` ```code``` `` |

## Session Behavior

Slack uses **time-gap sessions scoped to a thread**: messages in the same Slack thread continue the same Starpod session. The session key is `{team_id}:{channel_id}:{thread_root_ts}`, so a top-level message starts a fresh session per thread. After `gap_minutes` of inactivity (default 6h) the previous session is auto-closed and the next message starts a new one.

## Troubleshooting

| Symptom | Likely cause |
|---------|--------------|
| `slack auth.test failed` on startup | `SLACK_BOT_TOKEN` is missing, revoked, or from a different workspace |
| `apps.connections.open failed: not_allowed_token_type` | `SLACK_APP_TOKEN` is `xoxb-…` instead of `xapp-…` |
| `apps.connections.open failed: missing_scope` | App-level token wasn't generated with `connections:write` |
| File download returned HTML | Bot is missing the `files:read` scope — re-install the app after adding it |
| Bot doesn't reply in a channel | Add the bot to the channel (`/invite @Starpod`) — it only sees channels it's a member of |
| Events arrive but no reply | Check `[channels.slack].enabled = true` and the gateway logs for handler errors |
