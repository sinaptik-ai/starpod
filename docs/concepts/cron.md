# Cron & Scheduling

Starpod includes a built-in scheduler for running agent prompts on a schedule. Jobs execute through the same `StarpodAgent::chat()` pipeline as interactive conversations — full tool access and memory.

## Schedule Types

### Interval

Run every N milliseconds:

```json
{ "kind": "interval", "every_ms": 3600000 }
```

### Cron Expression

Both 5-field standard and 6-field (with seconds) cron expressions are accepted. 5-field expressions are auto-expanded to 6-field. Evaluated in the user's timezone:

```json
{ "kind": "cron", "expr": "0 0 9 * * MON-FRI" }
```

| Field | Values |
|-------|--------|
| Seconds | 0–59 |
| Minutes | 0–59 |
| Hours | 0–23 |
| Day of month | 1–31 |
| Month | 1–12 |
| Day of week | 0–6 (SUN=0) or MON–SUN |

### One-Shot

Run once at a specific time:

```json
{ "kind": "one_shot", "at": "2026-03-14T15:00:00Z" }
```

Set `delete_after_run: true` to auto-remove after execution.

## Creating Jobs

Ask the agent during a conversation:

> "Remind me every morning at 9am to check my emails"

> "Run a code quality check every hour"

> "In 30 minutes, summarize what we discussed today"

The agent uses `CronAdd` to create the job.

## How It Works

The scheduler runs in the background with a **30-second tick**:

1. Check for due jobs (`next_run_at <= now`)
2. Execute each job's prompt through `StarpodAgent::chat()`
3. Record the run (status, duration, result summary)
4. Send notifications (web UI toast + Telegram if configured)
5. Compute next run time (or delete if one-shot)

## Notifications

When a cron job completes (success or failure), notifications are sent through two channels:

### Web UI (real-time)

All connected WebSocket clients receive a `notification` event. The web UI shows a **toast notification** (auto-dismisses after 6 seconds) and refreshes the session sidebar. Clicking the toast navigates to the cron job's session transcript. New cron sessions appear with an **unread indicator** (blue dot) and are sorted to the top of the sidebar.

### Telegram

If Telegram is configured, the result is sent to all users in `STARPOD_TELEGRAM_ALLOWED_USER_IDS`.

Both channels fire for every job — web push is always active when clients are connected, Telegram is opt-in via configuration.

## Agent Tools

| Tool | Description |
|------|-------------|
| `CronAdd` | Create a job (`name`, `prompt`, `schedule`, `delete_after_run`, `max_retries`, `timeout_secs`, `session_mode`) |
| `CronList` | List all jobs with next run times |
| `CronRemove` | Remove a job by name |
| `CronRuns` | View execution history (`name`, `limit`) |
| `CronRun` | Immediately execute a job by name (manual trigger) |
| `CronUpdate` | Update properties of an existing job (`name`, `prompt`, `enabled`, `max_retries`, `timeout_secs`, `session_mode`) |

## Managing Jobs

Cron jobs are managed through the chat interface (ask the agent to create, list, or remove jobs) or via the web UI. The agent uses `CronAdd`, `CronList`, `CronRemove`, `CronRun`, and `CronUpdate` tools.

## Lifecycle Prompts

Starpod also includes **lifecycle prompts** — files that trigger agent behavior at key moments: `BOOTSTRAP.md` (first init), `BOOT.md` (every server start), and `HEARTBEAT.md` (every 30 minutes). The heartbeat is implemented as a reserved cron job, but unlike regular cron jobs it reads a holistic prompt from disk rather than running a discrete task.

See [Lifecycle Prompts](/concepts/heartbeat) for full details.

## Timezone

Cron expressions use the timezone from the top-level `timezone` field in `agent.toml`:

```toml
timezone = "Europe/Rome"
```

::: warning
Without a timezone, cron expressions are evaluated in UTC.
:::
