# starpod-memory

Persistent memory system: markdown files on disk + SQLite FTS5 full-text search index.

## API

```rust
let store = MemoryStore::new(&data_dir).await?;

// Bootstrap context for system prompt
let context = store.bootstrap_context()?;

// Full-text search
let results = store.search("database migrations", 5).await?;

// File operations
let content = store.read_file("knowledge/rust.md").await?;
store.write_file("knowledge/rust.md", "# Rust\n...").await?;

// Append to today's daily log
store.append_daily("Discussed migration strategy").await?;

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
    pub rank: f64,           // FTS5 rank (lower = more relevant)
}
```

## Chunking

Text is split into chunks for the FTS5 index:

| Parameter | Value |
|-----------|-------|
| Chunk size | ~400 tokens |
| Overlap | 80 tokens |
| Splitting | Line-aware |

## Bootstrap Context

`bootstrap_context()` assembles:

1. `SOUL.md` (capped at 20K chars)
2. `USER.md` (capped at 20K chars)
3. `MEMORY.md` (capped at 20K chars)
4. Last 3 daily logs (most recent first)

Returns a single string for injection into the system prompt.

## Tests

8 unit tests.
