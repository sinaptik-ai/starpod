# starpod-memory

Persistent memory system: markdown files on disk + SQLite FTS5 full-text search index.

## Architecture

```
.starpod/
├── SOUL.md            # Agent personality (agent-level)
├── HEARTBEAT.md       # Periodic tasks (agent-level)
├── db/
│   └── memory.db      # FTS5 index + vector embeddings
└── users/<id>/
    ├── USER.md        # User profile (per-user)
    ├── MEMORY.md      # Long-term memory (per-user)
    └── memory/
        └── YYYY-MM-DD.md  # Daily logs (per-user, temporal decay)
```

- **MemoryStore** manages agent-level files (SOUL.md, lifecycle files) and the FTS5 index.
- **UserMemoryView** overlays per-user files on top of the shared agent store.

## API

```rust
// Agent-level store
let store = MemoryStore::new(&agent_home, &db_dir).await?;

// Agent-level bootstrap context (SOUL.md only)
let context = store.bootstrap_context()?;

// Per-user view
let view = UserMemoryView::new(Arc::new(store), user_dir)?;

// User-level bootstrap (SOUL.md + USER.md + MEMORY.md + daily logs)
let context = view.bootstrap_context(20_000)?;

// Full-text search
let results = store.search("database migrations", 5).await?;

// Rebuild FTS5 index
store.reindex().await?;
```

## SearchResult

```rust
pub struct SearchResult {
    pub source: String,      // File path
    pub text: String,        // Matching chunk
    pub line_start: usize,
    pub line_end: usize,
    pub rank: f64,           // Rank score (more negative = more relevant)
}
```

## Chunking

Text is split into chunks for the FTS5 index:

| Parameter | Default | Description |
|-----------|---------|-------------|
| Chunk size | 1600 chars (~400 tokens) | Configurable via `[memory] chunk_size` |
| Overlap | 320 chars (~80 tokens) | Configurable via `[memory] chunk_overlap` |
| Splitting | Line-aware | Splits on line boundaries |

All chunking parameters are configurable in `agent.toml` under the `[memory]` section.

## Bootstrap Context

**Agent-level** (`MemoryStore::bootstrap_context()`): SOUL.md only.

**User-level** (`UserMemoryView::bootstrap_context()`): SOUL.md + USER.md + MEMORY.md + last 3 daily logs.

The per-file character cap is configurable via `[memory] bootstrap_file_cap` in `agent.toml` (default: 20000).

## Tests

30+ unit tests covering seeding, search, chunking, temporal decay, vector search, hybrid search, path validation, content size limits, and user view routing.
