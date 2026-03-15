//! # starpod-memory
//!
//! Memory system for Starpod — manages markdown files on disk with a
//! hybrid search pipeline backed by SQLite FTS5 and optional vector embeddings.
//!
//! ## Architecture
//!
//! ```text
//! .starpod/data/
//! ├── memory.db          # SQLite: FTS5 index + vector embeddings
//! ├── SOUL.md            # Agent personality (evergreen)
//! ├── USER.md            # User profile (evergreen)
//! ├── MEMORY.md          # Long-term memory (evergreen)
//! ├── HEARTBEAT.md       # Periodic task instructions (evergreen)
//! ├── memory/
//! │   └── YYYY-MM-DD.md  # Daily logs (subject to temporal decay)
//! └── knowledge/
//!     └── *.md           # Persistent knowledge (evergreen)
//! ```
//!
//! ## Search Pipeline
//!
//! When the `embeddings` feature is enabled and an embedder is configured,
//! [`MemoryStore::hybrid_search`] runs the full pipeline:
//!
//! 1. **FTS5 (BM25)** — keyword search with Porter stemming
//! 2. **Vector search** — cosine similarity against stored embeddings
//! 3. **RRF fusion** — Reciprocal Rank Fusion merges both ranked lists
//! 4. **Temporal decay** — older daily logs are penalized (configurable half-life)
//! 5. **MMR re-ranking** — Maximal Marginal Relevance promotes diversity
//!
//! Without embeddings, [`MemoryStore::search`] provides FTS5 + temporal decay.
//!
//! ## Features
//!
//! - `embeddings` — enables vector search via [`fastembed`] (BGE-Small-EN v1.5, 384 dims, ~45 MB)
//!
//! ## Security
//!
//! All file operations validate paths against traversal attacks, reject
//! non-`.md` extensions, and enforce a 1 MB write size cap via [`scoring::validate_path`]
//! and [`scoring::validate_content_size`].

pub mod defaults;
pub mod embedder;
pub mod fusion;
pub mod indexer;
pub mod schema;
pub mod scoring;
pub mod store;

pub use embedder::Embedder;
pub use store::{MemoryStore, SearchResult};
