# Memory

Starpod's memory system combines **markdown files on disk** with a **SQLite FTS5 full-text search index**. The agent can read, write, and search memory — and context is automatically bootstrapped into every conversation.

## File Layout

```
.starpod/
├── SOUL.md          Agent personality and instructions
├── HEARTBEAT.md     Heartbeat task instructions (periodic proactive behavior)
├── BOOT.md          Boot lifecycle prompt (runs every server start)
├── BOOTSTRAP.md     First-init bootstrap (runs once, then cleared)
├── db/
│   └── memory.db    SQLite FTS5 + vector index
├── users/<id>/      Per-user files
│   ├── USER.md      User information and preferences
│   ├── MEMORY.md    General long-term knowledge
│   └── memory/      Daily conversation logs
│       ├── 2026-03-12.md
│       ├── 2026-03-13.md
│       └── 2026-03-14.md
└── skills/          Agent skills (see Skills docs)
```

### Agent-Level Files

| File | Purpose | Auto-loaded? |
|------|---------|:---:|
| `SOUL.md` | Agent personality, instructions, behavioral guidelines | Yes |
| `HEARTBEAT.md` | Heartbeat task instructions | No (read by scheduler) |
| `BOOT.md` | Boot lifecycle prompt | No (runs at startup) |
| `BOOTSTRAP.md` | First-init bootstrap prompt (self-destructing) | No (runs once) |

### Per-User Files

Each user gets their own directory at `.starpod/users/<id>/`:

| File | Purpose | Auto-loaded? |
|------|---------|:---:|
| `USER.md` | User info — name, role, preferences | Yes |
| `MEMORY.md` | General knowledge the agent should always have about this user | Yes |
| `memory/YYYY-MM-DD.md` | Daily conversation logs | Last 3 days |

### Daily Logs

The `memory/` directory inside each user's directory contains daily logs named `YYYY-MM-DD.md`. After each conversation, the agent appends a summary. The **last 3 daily logs** are included in the bootstrap context.

## Context Bootstrap

Context is assembled in two layers:

**Agent-level** (`MemoryStore::bootstrap_context`):
1. `SOUL.md` (up to 20K characters)

**Per-user** (`UserMemoryView::bootstrap_context`):
1. `SOUL.md` (from agent store, up to 20K characters)
2. `USER.md` (from user directory, up to 20K characters)
3. `MEMORY.md` (from user directory, up to 20K characters)
4. Last 3 daily logs (most recent first, from user directory)

This context is injected into the system prompt so the agent always has its identity, user knowledge, and recent history.

## Full-Text Search

All markdown files are indexed in SQLite FTS5 with chunking for efficient retrieval:

| Parameter | Value |
|-----------|-------|
| Chunk size | 1600 characters (~400 tokens) |
| Overlap | 320 characters (~80 tokens) |
| Splitting | Line-aware (never splits mid-line) |

Search results include the source file, matching text, line range, and relevance rank.

## Agent Tools

### MemorySearch

Search the full-text index:

```json
{
  "query": "user's favorite programming language",
  "limit": 5
}
```

### MemoryWrite

Write or update a file:

```json
{
  "file": "notes/rust-patterns.md",
  "content": "# Rust Patterns\n\n..."
}
```

### MemoryAppendDaily

Append to today's daily log:

```json
{
  "text": "User asked about database migrations"
}
```

## CLI

```bash
# Search from the command line
starpod memory search "database migrations" --limit 5

# Rebuild FTS5 index after manual edits
starpod memory reindex
```

## Manual Editing

You can edit any file in `.starpod/` with your text editor. Run `starpod memory reindex` afterward to update the search index.

::: tip
Edit `SOUL.md` to change the agent's personality. Edit `USER.md` to update what the agent knows about you. Changes take effect on the next conversation.
:::
