# Memory

Starpod's memory system combines **markdown files on disk** with a **SQLite FTS5 full-text search index**. The agent can read, write, and search memory — and context is automatically bootstrapped into every conversation.

## File Layout

```
.starpod/data/
├── SOUL.md          Agent personality and instructions
├── USER.md          User information and preferences
├── MEMORY.md        General long-term knowledge
├── HEARTBEAT.md     Heartbeat task instructions (periodic proactive behavior)
├── BOOT.md          Boot lifecycle prompt (runs every server start)
├── BOOTSTRAP.md     First-init bootstrap (runs once, then cleared)
├── memory/          Daily conversation logs
│   ├── 2026-03-12.md
│   ├── 2026-03-13.md
│   └── 2026-03-14.md
├── knowledge/       Knowledge base documents
│   ├── *.md
│   └── sessions/    Auto-exported session transcripts
│       └── *.md
└── memory.db        SQLite FTS5 index
```

### Core Files

| File | Purpose | Auto-loaded? |
|------|---------|:---:|
| `SOUL.md` | Agent personality, instructions, behavioral guidelines | Yes |
| `USER.md` | User info — name, role, preferences | Yes |
| `MEMORY.md` | General knowledge the agent should always have | Yes |

### Daily Logs

The `memory/` directory contains daily logs named `YYYY-MM-DD.md`. After each conversation, the agent appends a summary. The **last 3 daily logs** are included in the bootstrap context.

### Knowledge Base

The `knowledge/` directory holds topical documents. These are indexed for search but **not** automatically included in the system prompt — the agent uses `MemorySearch` to retrieve relevant chunks on demand.

### Session Transcripts

The `knowledge/sessions/` subdirectory stores auto-exported session transcripts. See [Session Transcript Export](#session-transcript-export) below.

## Context Bootstrap

On every conversation turn, Starpod assembles a context string from:

1. `SOUL.md` (up to 20K characters)
2. `USER.md` (up to 20K characters)
3. `MEMORY.md` (up to 20K characters)
4. Last 3 daily logs (most recent first)

This context is injected into the system prompt so the agent always has its identity, user knowledge, and recent history.

## Full-Text Search

All markdown files are indexed in SQLite FTS5 with chunking for efficient retrieval:

| Parameter | Value |
|-----------|-------|
| Chunk size | ~400 tokens |
| Overlap | 80 tokens |
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
  "file": "knowledge/rust-patterns.md",
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
starpod agent memory search "database migrations" --limit 5

# Rebuild FTS5 index after manual edits
starpod agent memory reindex
```

## Session Transcript Export

When a session is closed (e.g. auto-closed after Telegram inactivity), Starpod exports the full conversation transcript to `knowledge/sessions/` as a markdown file. This makes past conversations searchable via `MemorySearch`.

Each transcript includes:
- **Metadata header**: date, channel, message count, summary
- **Full message history**: every user and assistant message

Exported files are named `{title-slug}-{session-id-prefix}.md` and are immediately indexed for search.

### Configuration

Session export is enabled by default. To disable it:

```toml
[memory]
export_sessions = false
```

### How It Works

1. A session is auto-closed (e.g. Telegram time-gap exceeded)
2. `SessionDecision::New { closed_session_id: Some(id) }` is returned
3. The agent fetches all messages from the closed session
4. Messages are formatted as markdown and written to `knowledge/sessions/`
5. The file is indexed (FTS5 + optional vector embeddings) immediately

Unlike daily logs, session transcripts in `knowledge/` are **evergreen** — they are not subject to temporal decay in search ranking.

## Manual Editing

You can edit any file in `.starpod/data/` with your text editor. Run `starpod agent memory reindex` afterward to update the search index.

::: tip
Edit `SOUL.md` to change the agent's personality. Edit `USER.md` to update what the agent knows about you. Changes take effect on the next conversation.
:::
