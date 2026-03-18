# Lifecycle Prompts

Starpod has three **lifecycle prompt files** that let the agent act autonomously at key moments — without being prompted by a user. All three live in `.starpod/` and are **disabled by default** (empty on init). Add instructions to activate them.

| File | When it runs | How often | Session |
|------|-------------|-----------|---------|
| `BOOTSTRAP.md` | First init only | Once, then cleared | Main |
| `BOOT.md` | Every server start | Once per `starpod serve` | Main |
| `HEARTBEAT.md` | Periodically while running | Every 30 minutes | Main |

All three run through the full `chat()` pipeline — the agent has access to memory, tools, vault, and conversation context.

## BOOTSTRAP.md — First-Run Onboarding

Bootstrap runs **once** when the data directory is first created. Use it for interactive onboarding: learning the user's name, setting up the agent's personality, configuring preferences.

After successful execution, the file is automatically cleared so it never runs again.

### Example

```markdown
Hey. I just came online for the first time.

Let's get to know each other:
1. Ask me my name and how I'd like to be addressed
2. Ask about my timezone and communication preferences
3. Ask what I'll primarily use you for

Update USER.md with what you learn. Be conversational, not robotic.
```

### When to use

- Interactive onboarding that only makes sense on first run
- Seeding initial memory/preferences from a conversation
- One-time setup tasks (creating vault entries, configuring integrations)

## BOOT.md — Server Start

Boot runs **every time** `starpod serve` starts. Use it for startup checks, daily briefings, or any "wake up" routine.

### Example

```markdown
Server just started. Quick status check:

1. Review my calendar for today's events
2. Check for any failed cron jobs since last shutdown
3. Summarize anything urgent from memory

If there's nothing notable, stay silent.
```

### When to use

- Morning briefings or daily summaries
- Verifying integrations are working after restart
- Catching up on what happened while the server was down

## HEARTBEAT.md — Ambient Awareness

The heartbeat is Starpod's **background consciousness** — a way for the agent to periodically wake up and act on standing instructions.

### Heartbeat vs. Cron Jobs

Both use the same scheduler, but they serve different purposes:

| | Heartbeat | Cron Jobs |
|---|---|---|
| **Purpose** | Background consciousness — "keep an eye on things" | Discrete scheduled tasks — "do X at time Y" |
| **Defined in** | `HEARTBEAT.md` file | Created via `CronAdd` tool or conversation |
| **Schedule** | Fixed: every 30 minutes | Any cron expression, interval, or one-shot |
| **Prompt** | Single holistic prompt, re-read from disk each run | One prompt per job, stored in the database |
| **Lifecycle** | Opt-in via file content; clear the file to disable | Explicitly created and removed |

**When to use the heartbeat:**
> "Check if anyone messaged me, glance at my calendar, and let me know if anything needs attention."

One holistic instruction that defines what the agent should be aware of. You wouldn't create 5 separate cron jobs for each of those — you'd write it once in `HEARTBEAT.md` and let the agent decide what's worth reporting each cycle.

**When to use a cron job:**
> "Every Monday at 9am, send me a weekly report."

A concrete, self-contained task with a specific schedule and output.

### Example

```markdown
Check the following and notify me if anything needs attention:

- Any unread Telegram messages
- Upcoming calendar events in the next 2 hours
- Any cron jobs that failed since the last heartbeat
- Price alerts I've set up in memory

Be concise. Only message me if there's something actionable.
If nothing needs attention, do nothing.
```

### HeartbeatWake Tool

The agent has a built-in `HeartbeatWake` tool that can trigger the heartbeat outside its normal 30-minute cycle:

| Parameter | Type | Description |
|-----------|------|-------------|
| `mode` | `"now"` \| `"next"` | `"now"` triggers immediately, `"next"` waits for schedule (default) |
| `message` | string | Optional message to prepend to the heartbeat prompt |

This is useful when the agent detects something that warrants an earlier check-in — for example, if a cron job fails, the agent could wake the heartbeat to assess the situation sooner.

## How It Works

```
starpod serve
    │
    ├─ Scheduler starts (heartbeat + cron jobs)
    │
    └─ Lifecycle prompts fire (background):
         │
         ├─ BOOTSTRAP.md has content?
         │      │           │
         │     Yes          No
         │      │           │
         │      ▼           ▼
         │   Run prompt   Skip
         │      │
         │      ▼
         │   Clear file (never runs again)
         │
         ├─ BOOT.md has content?
         │      │           │
         │     Yes          No
         │      │           │
         │      ▼           ▼
         │   Run prompt   Skip
         │
         └─ HEARTBEAT.md has content?
                │           │
               Yes          No
                │           │
                ▼           ▼
          Create cron job  Skip
                │
                ▼
          Every 30 min: re-read file → run if non-empty
```

Bootstrap runs first, then boot, then the heartbeat loop begins. All are independent — you can use any combination.

## Setup

### 1. Initialize your project

```bash
starpod init
```

This creates empty `BOOTSTRAP.md`, `BOOT.md`, and `HEARTBEAT.md` files in `.starpod/`.

### 2. Edit the files you want to activate

```bash
# Edit with your preferred editor
$EDITOR .starpod/BOOT.md
$EDITOR .starpod/HEARTBEAT.md
```

### 3. Start the server

```bash
starpod serve
```

You'll see lifecycle activity in the logs:

```
INFO Lifecycle prompts dispatched
INFO Running boot lifecycle prompt
INFO Boot completed
INFO Created __heartbeat__ cron job (every 30 minutes)
```

### 4. Disable any lifecycle prompt

Clear the file's content. The file stays on disk but execution is skipped when empty.

## Key Details

- **Re-reads from disk**: `HEARTBEAT.md` and `BOOT.md` are read fresh on every execution. You can update them while the server is running — no restart needed (though `BOOT.md` won't re-run until the next restart).
- **Main session**: All lifecycle prompts run in the main session, sharing context with ongoing conversations.
- **Disabled by default**: All three files are empty on `starpod init`. Add content to activate.
- **`BOOTSTRAP.md` is one-shot**: The file is cleared after successful execution. To re-run it, add new content manually.
- **Reserved name**: The `__heartbeat__` cron job name is reserved — you cannot create a regular cron job with this name.
- **Heartbeat retries**: 3 attempts before marking the run as failed, with a 2-hour timeout per execution.
- **Hook integration**: The `Setup` hook event fires with trigger `init` (bootstrap) or `boot` (boot), so external hooks can react to these lifecycle events.

## Tips

- **Be specific about what "no action" means.** Tell the agent to stay silent if nothing needs attention — otherwise you'll get a report every 30 minutes saying "all clear."
- **Keep heartbeat focused.** A heartbeat that tries to check 20 things will be slow and expensive. Prioritize what matters.
- **Use memory.** All lifecycle prompts run through the full agent pipeline. The agent can read and write memory, track state between beats, and use tools.
- **Bootstrap is for conversations.** Unlike boot (which is typically a checklist), bootstrap works best as an interactive onboarding prompt — the agent will respond and you can reply.
