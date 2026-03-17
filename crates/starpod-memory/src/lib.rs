//! # starpod-memory
//!
//! Memory system for Starpod — manages markdown files on disk with a
//! hybrid search pipeline backed by SQLite FTS5 and optional vector embeddings.
//!
//! ## Architecture
//!
//! ```text
//! .starpod/
//! ├── SOUL.md            # Agent personality (evergreen)
//! ├── HEARTBEAT.md       # Periodic task instructions (evergreen)
//! ├── BOOT.md            # Startup instructions (evergreen)
//! ├── BOOTSTRAP.md       # One-time init (self-destructing)
//! ├── db/
//! │   └── memory.db      # SQLite: FTS5 index + vector embeddings
//! └── users/<id>/
//!     ├── USER.md        # User profile (per-user)
//!     ├── MEMORY.md      # Long-term memory (per-user)
//!     └── memory/
//!         └── YYYY-MM-DD.md  # Daily logs (per-user, temporal decay)
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
pub mod user_view;

pub use embedder::Embedder;
pub use store::{MemoryStore, SearchResult};
pub use user_view::UserMemoryView;
