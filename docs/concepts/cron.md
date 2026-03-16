# Cron & Scheduling

Starpod includes a built-in scheduler for running agent prompts on a schedule. Jobs execute through the same `StarpodAgent::chat()` pipeline as interactive conversations — full tool access and memory.

## Schedule Types

### Interval

Run every N milliseconds:

```json
{ "kind": "interval", "every_ms": 3600000 }
```

### Cron Expression

6-field cron expressions (with seconds), evaluated in the user's timezone:

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
4. Send notification via Telegram (if configured)
5. Compute next run time (or delete if one-shot)

## Notifications

Completed cron jobs can send results to Telegram. The notification includes the job name, result summary, and success/failure status.

## Agent Tools

| Tool | Description |
|------|-------------|
| `CronAdd` | Create a job (`name`, `prompt`, `schedule`, `delete_after_run`) |
| `CronList` | List all jobs with next run times |
| `CronRemove` | Remove a job by name |
| `CronRuns` | View execution history (`name`, `limit`) |

## CLI

```bash
starpod agent cron list                         # List all jobs
starpod agent cron remove "morning-reminder"    # Remove a job
starpod agent cron runs "morning-reminder" -l 10 # View run history
```

## Lifecycle Prompts

Starpod also includes **lifecycle prompts** — files that trigger agent behavior at key moments: `BOOTSTRAP.md` (first init), `BOOT.md` (every server start), and `HEARTBEAT.md` (every 30 minutes). The heartbeat is implemented as a reserved cron job, but unlike regular cron jobs it reads a holistic prompt from disk rather than running a discrete task.

See [Lifecycle Prompts](/concepts/heartbeat) for full details.

## Timezone

Cron expressions use the user's timezone from `config.toml`:

```toml
[user]
timezone = "America/New_York"
```

::: warning
Without a timezone, cron expressions are evaluated in UTC.
:::
