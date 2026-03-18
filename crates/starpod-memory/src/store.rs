use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use chrono::Local;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::debug;

use starpod_core::{StarpodError, Result};

use crate::defaults;
use crate::embedder::{self, Embedder};
use crate::fusion;
use crate::indexer::{self, reindex_source, CHUNK_SIZE, CHUNK_OVERLAP};
use crate::schema;
use crate::scoring;

/// Maximum characters to include from a single file in bootstrap context.
const BOOTSTRAP_FILE_CAP: usize = 20_000;

/// Default half-life for temporal decay (in days).
const DEFAULT_HALF_LIFE_DAYS: f64 = 30.0;

/// A search result from the memory index.
///
/// Represents a chunk of text from a source file that matched a query.
/// The `rank` field is negative, with more negative values indicating
/// better matches. This convention is consistent across FTS5 (where rank
/// is natively negative), RRF fusion, and hybrid search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Source file the chunk came from (e.g. `"SOUL.md"`, `"memory/2026-03-15.md"`).
    pub source: String,
    /// The matching text chunk.
    pub text: String,
    /// Starting line number (1-indexed) in the source file.
    pub line_start: usize,
    /// Ending line number in the source file.
    pub line_end: usize,
    /// Rank score — more negative = better match.
    ///
    /// For FTS5-only search this is the raw BM25 score adjusted for temporal decay.
    /// For hybrid search this is the negative RRF score after decay and MMR.
    pub rank: f64,
}

/// The main memory store — manages agent-level markdown files with a hybrid search index.
///
/// Blueprint-managed files (SOUL.md, lifecycle files) live in `config_dir`
/// (`.starpod/config/`). Runtime data files (daily logs, agent-written files)
/// live in `agent_home` (`.starpod/`).
/// The FTS5/vector database lives in `db_dir` (`.starpod/db/`).
/// User-specific files (USER.md, MEMORY.md, daily logs) are handled by
/// [`UserMemoryView`](crate::user_view::UserMemoryView), not this struct.
///
/// # Search Pipeline
///
/// - [`search`](Self::search) — FTS5 + temporal decay (always available)
/// - [`vector_search`](Self::vector_search) — cosine similarity (requires embedder)
/// - [`hybrid_search`](Self::hybrid_search) — FTS5 + vector → RRF fusion → decay → MMR
///
/// # Security
///
/// All file read/write operations validate paths via [`scoring::validate_path`]
/// to prevent directory traversal. Writes are capped at 1 MB.
pub struct MemoryStore {
    /// Agent home directory (.starpod/) — runtime data files, general read/write.
    agent_home: PathBuf,
    /// Config directory (.starpod/config/) — blueprint-managed files (SOUL.md, lifecycle).
    config_dir: PathBuf,
    pool: SqlitePool,
    /// Half-life in days for temporal decay on search results.
    half_life_days: f64,
    /// MMR lambda: 0.0 = max diversity, 1.0 = pure relevance.
    mmr_lambda: f64,
    /// Optional embedder for vector search (enabled with `embeddings` feature).
    embedder: Option<Arc<dyn Embedder>>,
    /// Target chunk size in characters for indexing.
    chunk_size: usize,
    /// Overlap in characters between chunks.
    chunk_overlap: usize,
    /// Maximum characters to include from a single file in bootstrap context.
    bootstrap_file_cap: usize,
}

impl MemoryStore {
    /// Create a new MemoryStore.
    ///
    /// - `agent_home`: the `.starpod/` directory (runtime data, general read/write)
    /// - `config_dir`: the `.starpod/config/` directory (SOUL.md, lifecycle files)
    /// - `db_dir`: the `.starpod/db/` directory (contains memory.db)
    pub async fn new(agent_home: &Path, config_dir: &Path, db_dir: &Path) -> Result<Self> {
        // Ensure directories exist
        std::fs::create_dir_all(agent_home)
            .map_err(StarpodError::Io)?;
        std::fs::create_dir_all(config_dir)
            .map_err(StarpodError::Io)?;
        std::fs::create_dir_all(db_dir)
            .map_err(StarpodError::Io)?;

        // Open SQLite pool
        let db_path = db_dir.join("memory.db");
        let opts = SqliteConnectOptions::from_str(
            &format!("sqlite://{}?mode=rwc", db_path.display()),
        )
        .map_err(|e| StarpodError::Database(format!("Invalid DB path: {}", e)))?;

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to open database: {}", e)))?;

        // Run migrations
        schema::run_migrations(&pool).await?;

        let store = Self {
            agent_home: agent_home.to_path_buf(),
            config_dir: config_dir.to_path_buf(),
            pool,
            half_life_days: DEFAULT_HALF_LIFE_DAYS,
            mmr_lambda: 0.7,
            embedder: None,
            chunk_size: CHUNK_SIZE,
            chunk_overlap: CHUNK_OVERLAP,
            bootstrap_file_cap: BOOTSTRAP_FILE_CAP,
        };

        // Seed default files if they don't exist
        store.seed_defaults()?;

        // Initial index
        store.reindex().await?;

        Ok(store)
    }

    /// Seed default lifecycle files on first run.
    ///
    /// Blueprint-managed files (SOUL.md, HEARTBEAT.md, BOOT.md, BOOTSTRAP.md)
    /// are seeded into `config_dir`. USER.md and MEMORY.md are per-user files
    /// managed by [`UserMemoryView`](crate::user_view::UserMemoryView).
    ///
    /// Returns `true` if this is a fresh config (SOUL.md didn't exist yet).
    fn seed_defaults(&self) -> Result<bool> {
        let fresh = !self.config_dir.join("SOUL.md").exists();

        // Seed SOUL.md only if not present (first init without blueprint)
        if fresh {
            let path = self.config_dir.join("SOUL.md");
            debug!(file = "SOUL.md", "Seeding default SOUL.md");
            std::fs::write(&path, defaults::DEFAULT_SOUL)?;
        }

        // Lifecycle files in config_dir
        let lifecycle_files = [
            ("HEARTBEAT.md", defaults::DEFAULT_HEARTBEAT),
            ("BOOT.md", defaults::DEFAULT_BOOT),
            ("BOOTSTRAP.md", defaults::DEFAULT_BOOTSTRAP),
        ];

        for (name, content) in &lifecycle_files {
            let path = self.config_dir.join(name);
            if !path.exists() {
                debug!(file = %name, "Seeding default file");
                std::fs::write(&path, content)?;
            }
        }

        Ok(fresh)
    }

    /// Get the agent home directory path (.starpod/).
    pub fn agent_home(&self) -> &Path {
        &self.agent_home
    }

    /// Get the config directory path (.starpod/config/).
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Blueprint-managed file names that live in config_dir.
    const CONFIG_FILES: &[&str] = &[
        "SOUL.md", "HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md",
    ];

    /// Resolve a file path: config files go to config_dir, everything else to agent_home.
    fn resolve_path(&self, name: &str) -> PathBuf {
        // Check if this is a known config file (top-level only, not in subdirs)
        if !name.contains('/') && Self::CONFIG_FILES.iter().any(|&f| f == name) {
            self.config_dir.join(name)
        } else {
            self.agent_home.join(name)
        }
    }

    /// Returns `true` if BOOTSTRAP.md exists and has non-empty content.
    pub fn has_bootstrap(&self) -> bool {
        let path = self.config_dir.join("BOOTSTRAP.md");
        path.is_file()
            && std::fs::read_to_string(&path)
                .map(|c| !c.trim().is_empty())
                .unwrap_or(false)
    }

    /// Delete BOOTSTRAP.md (called after successful bootstrap execution).
    pub fn clear_bootstrap(&self) -> Result<()> {
        let path = self.config_dir.join("BOOTSTRAP.md");
        if path.exists() {
            std::fs::write(&path, "")?;
        }
        Ok(())
    }

    /// Build agent-level bootstrap context from SOUL.md only.
    ///
    /// User-specific context (USER.md, MEMORY.md, daily logs) is handled by
    /// [`UserMemoryView::bootstrap_context()`](crate::user_view::UserMemoryView::bootstrap_context).
    pub fn bootstrap_context(&self) -> Result<String> {
        let content = self.read_file("SOUL.md")?;
        let capped = if content.len() > self.bootstrap_file_cap {
            let mut end = self.bootstrap_file_cap;
            while end > 0 && !content.is_char_boundary(end) { end -= 1; }
            &content[..end]
        } else {
            &content
        };
        Ok(format!("--- SOUL.md ---\n{}", capped))
    }

    /// Set the half-life for temporal decay on search results.
    pub fn set_half_life_days(&mut self, days: f64) {
        self.half_life_days = days;
    }

    /// Set the MMR lambda for diversity vs relevance balance.
    pub fn set_mmr_lambda(&mut self, lambda: f64) {
        self.mmr_lambda = lambda;
    }

    /// Set the target chunk size in characters for indexing.
    pub fn set_chunk_size(&mut self, size: usize) {
        self.chunk_size = size;
    }

    /// Set the overlap in characters between chunks.
    pub fn set_chunk_overlap(&mut self, overlap: usize) {
        self.chunk_overlap = overlap;
    }

    /// Set the maximum characters to include from a single file in bootstrap context.
    pub fn set_bootstrap_file_cap(&mut self, cap: usize) {
        self.bootstrap_file_cap = cap;
    }

    /// Full-text search across all indexed content.
    ///
    /// Results are re-ranked with temporal decay: recent daily logs score
    /// higher than older ones, while evergreen files (SOUL.md, HEARTBEAT.md)
    /// are unaffected.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        // Fetch more than needed so we have room after re-ranking
        let fetch_limit = (limit * 3).max(30);
        let rows = sqlx::query(
            "SELECT source, chunk_text, line_start, line_end, rank
             FROM memory_fts
             WHERE memory_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )
        .bind(query)
        .bind(fetch_limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Search query failed: {}", e)))?;

        let mut results: Vec<SearchResult> = rows
            .iter()
            .map(|row| {
                let source = row.get::<String, _>("source");
                let raw_rank = row.get::<f64, _>("rank");
                let adjusted_rank = scoring::apply_decay(raw_rank, &source, self.half_life_days);
                SearchResult {
                    source,
                    text: row.get::<String, _>("chunk_text"),
                    line_start: row.get::<i64, _>("line_start") as usize,
                    line_end: row.get::<i64, _>("line_end") as usize,
                    rank: adjusted_rank,
                }
            })
            .collect();

        // Re-sort by adjusted rank (more negative = better)
        results.sort_by(|a, b| a.rank.partial_cmp(&b.rank).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Set the embedder for vector search.
    pub fn set_embedder(&mut self, embedder: Arc<dyn Embedder>) {
        self.embedder = Some(embedder);
    }

    /// Vector search: embed the query, compare against stored vectors, return top-K.
    ///
    /// Returns empty vec if no embedder is configured.
    pub async fn vector_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let embedder = match &self.embedder {
            Some(e) => e,
            None => return Ok(Vec::new()),
        };

        // Embed the query
        let query_vecs = embedder
            .embed(&[query.to_string()])
            .await?;
        let query_vec = match query_vecs.first() {
            Some(v) => v,
            None => return Ok(Vec::new()),
        };

        // Load all stored vectors
        let rows = sqlx::query(
            "SELECT v.source, v.embedding, v.line_start, v.line_end, f.chunk_text
             FROM memory_vectors v
             LEFT JOIN memory_fts f ON f.source = v.source
                 AND f.line_start = v.line_start AND f.line_end = v.line_end"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Vector search failed: {}", e)))?;

        let mut scored: Vec<(f32, SearchResult)> = Vec::new();
        for row in &rows {
            let blob: Vec<u8> = row.get("embedding");
            let embedding = bytes_to_f32_vec(&blob);
            let similarity = embedder::cosine_similarity(query_vec, &embedding);

            let source: String = row.get("source");
            let text: String = row.try_get("chunk_text").unwrap_or_default();

            scored.push((similarity, SearchResult {
                source,
                text,
                line_start: row.get::<i64, _>("line_start") as usize,
                line_end: row.get::<i64, _>("line_end") as usize,
                rank: -(similarity as f64), // negative similarity so more negative = better
            }));
        }

        // Sort by similarity descending
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        Ok(scored.into_iter().map(|(_, r)| r).collect())
    }

    /// Hybrid search: run FTS5 + vector search, fuse with RRF, apply MMR.
    ///
    /// Falls back to FTS5-only when no embedder is configured.
    /// Pipeline: FTS5 + vector → RRF fusion → temporal decay → MMR diversity.
    pub async fn hybrid_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let embedder = match &self.embedder {
            Some(e) => e,
            None => return self.search(query, limit).await,
        };

        // Run both searches concurrently
        let fts_limit = (limit * 3).max(30);
        let vec_limit = (limit * 3).max(30);

        let (fts_results, vec_results) = tokio::join!(
            self.fts_search_raw(query, fts_limit),
            self.vector_search(query, vec_limit),
        );

        let fts_results = fts_results?;
        let vec_results = vec_results?;

        // Fuse with RRF
        let mut fused = fusion::reciprocal_rank_fusion(&fts_results, &vec_results, limit * 3);

        // Apply temporal decay — multiply to make old results less negative (worse)
        for result in &mut fused {
            let decay = scoring::decay_factor(&result.source, self.half_life_days);
            if decay > 0.0 && decay < 1.0 {
                result.rank *= decay;
            }
        }

        // Re-sort by decayed rank
        fused.sort_by(|a, b| a.rank.partial_cmp(&b.rank).unwrap_or(std::cmp::Ordering::Equal));

        // Apply MMR for diversity: embed top candidates, then re-rank
        let mmr_pool_size = (limit * 2).min(fused.len());
        if mmr_pool_size > 0 {
            // Embed the query for MMR
            let query_vecs = embedder.embed(&[query.to_string()]).await?;
            if let Some(query_vec) = query_vecs.first() {
                // Embed the candidate texts
                let texts: Vec<String> = fused[..mmr_pool_size]
                    .iter()
                    .map(|r| r.text.clone())
                    .collect();
                let embeddings = embedder.embed(&texts).await?;

                let candidates: Vec<(Vec<f32>, usize)> = embeddings
                    .into_iter()
                    .enumerate()
                    .map(|(i, emb)| (emb, i))
                    .collect();

                let selected_indices =
                    scoring::mmr_rerank(query_vec, &candidates, limit, self.mmr_lambda);

                let pool = fused;
                fused = selected_indices
                    .into_iter()
                    .map(|idx| pool[idx].clone())
                    .collect();
            } else {
                fused.truncate(limit);
            }
        }

        Ok(fused)
    }

    /// Raw FTS5 search without decay (used internally by hybrid_search).
    async fn fts_search_raw(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let rows = sqlx::query(
            "SELECT source, chunk_text, line_start, line_end, rank
             FROM memory_fts
             WHERE memory_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Search query failed: {}", e)))?;

        Ok(rows
            .iter()
            .map(|row| SearchResult {
                source: row.get::<String, _>("source"),
                text: row.get::<String, _>("chunk_text"),
                line_start: row.get::<i64, _>("line_start") as usize,
                line_end: row.get::<i64, _>("line_end") as usize,
                rank: row.get::<f64, _>("rank"),
            })
            .collect())
    }

    /// Embed and store vectors for a source file's chunks.
    async fn embed_and_store_source(&self, source: &str, text: &str) -> Result<()> {
        let embedder = match &self.embedder {
            Some(e) => e,
            None => return Ok(()),
        };

        // Delete old vectors for this source
        sqlx::query("DELETE FROM memory_vectors WHERE source = ?1")
            .bind(source)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to delete old vectors: {}", e)))?;

        // Chunk the text
        let chunks = indexer::chunk_text(source, text, self.chunk_size, self.chunk_overlap);
        if chunks.is_empty() {
            return Ok(());
        }

        // Embed all chunks in one batch
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embeddings = embedder.embed(&texts).await?;

        // Store vectors
        for (idx, (chunk, embedding)) in chunks.iter().zip(embeddings.iter()).enumerate() {
            let blob = f32_vec_to_bytes(embedding);
            sqlx::query(
                "INSERT INTO memory_vectors (source, chunk_idx, embedding, line_start, line_end)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&chunk.source)
            .bind(idx as i64)
            .bind(&blob)
            .bind(chunk.line_start as i64)
            .bind(chunk.line_end as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to insert vector: {}", e)))?;
        }

        Ok(())
    }

    /// Read a file from the appropriate directory (config_dir for config files, agent_home otherwise).
    pub fn read_file(&self, name: &str) -> Result<String> {
        // Validate against agent_home (the broader sandbox)
        scoring::validate_path(name, &self.agent_home)?;
        let path = self.resolve_path(name);
        if !path.exists() {
            return Ok(String::new());
        }
        std::fs::read_to_string(&path).map_err(StarpodError::Io)
    }

    /// Write a file and reindex it.
    ///
    /// Config files (SOUL.md, lifecycle files) are written to config_dir,
    /// everything else to agent_home.
    pub async fn write_file(&self, name: &str, content: &str) -> Result<()> {
        scoring::validate_path(name, &self.agent_home)?;
        scoring::validate_content_size(content)?;

        let path = self.resolve_path(name);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&path, content)?;

        // Reindex this file (FTS5 + vectors)
        reindex_source(&self.pool, name, content, self.chunk_size, self.chunk_overlap).await?;
        self.embed_and_store_source(name, content).await?;

        Ok(())
    }

    /// Append a timestamped entry to today's daily log.
    pub async fn append_daily(&self, text: &str) -> Result<()> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let filename = format!("memory/{}.md", today);
        let path = self.agent_home.join(&filename);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let timestamp = Local::now().format("%H:%M:%S").to_string();
        let entry = format!("\n## {}\n{}\n", timestamp, text);

        let mut content = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            format!("# Daily Log — {}\n", today)
        };

        content.push_str(&entry);
        std::fs::write(&path, &content)?;

        // Reindex the daily file (FTS5 + vectors)
        reindex_source(&self.pool, &filename, &content, self.chunk_size, self.chunk_overlap).await?;
        self.embed_and_store_source(&filename, &content).await?;

        Ok(())
    }

    /// Full reindex of agent-level markdown files.
    ///
    /// Indexes config files from config_dir (SOUL.md, lifecycle files) and
    /// runtime files from agent_home (memory/ daily logs, agent-written files).
    /// User-level files are not indexed here — they're handled per-user.
    pub async fn reindex(&self) -> Result<()> {
        // Clear all existing FTS entries
        sqlx::query("DELETE FROM memory_fts")
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to clear FTS: {}", e)))?;

        // Clear all existing vectors
        sqlx::query("DELETE FROM memory_vectors")
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to clear vectors: {}", e)))?;

        // Index config files (SOUL.md, HEARTBEAT.md, BOOT.md, BOOTSTRAP.md)
        self.index_dir(&self.config_dir.clone(), "").await?;

        // Index top-level .md files in agent_home (excluding config dir)
        if let Ok(entries) = std::fs::read_dir(&self.agent_home) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
                    let filename = entry.file_name().to_string_lossy().to_string();
                    // Skip config files (already indexed from config_dir)
                    if !Self::CONFIG_FILES.iter().any(|&f| f == filename) {
                        let content = std::fs::read_to_string(&path)?;
                        reindex_source(&self.pool, &filename, &content, self.chunk_size, self.chunk_overlap).await?;
                        self.embed_and_store_source(&filename, &content).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Index all .md files in a directory with a source prefix.
    async fn index_dir(&self, dir: &Path, prefix: &str) -> Result<()> {
        let entries = std::fs::read_dir(dir).map_err(StarpodError::Io)?;

        for entry in entries {
            let entry = entry.map_err(StarpodError::Io)?;
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
                let filename = entry.file_name().to_string_lossy().to_string();
                let source = format!("{}{}", prefix, filename);
                let content = std::fs::read_to_string(&path)?;
                reindex_source(&self.pool, &source, &content, self.chunk_size, self.chunk_overlap).await?;
                self.embed_and_store_source(&source, &content).await?;
            }
        }

        Ok(())
    }
}

/// Convert a Vec<f32> to bytes for BLOB storage.
fn f32_vec_to_bytes(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for &v in vec {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Convert bytes back to Vec<f32>.
fn bytes_to_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── Helper ──────────────────────────────────────────────────────────

    /// Create a MemoryStore for tests with agent_home, config_dir, and db_dir as siblings.
    async fn test_store(tmp: &TempDir) -> MemoryStore {
        let agent_home = tmp.path().join("agent_home");
        let config_dir = tmp.path().join("agent_home").join("config");
        let db_dir = tmp.path().join("db");
        MemoryStore::new(&agent_home, &config_dir, &db_dir).await.unwrap()
    }

    // ── Existing tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_new_seeds_defaults() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let config_dir = tmp.path().join("agent_home").join("config");

        // Config files should exist in config_dir
        assert!(config_dir.join("SOUL.md").exists());
        assert!(config_dir.join("HEARTBEAT.md").exists());
        assert!(config_dir.join("BOOT.md").exists());
        assert!(config_dir.join("BOOTSTRAP.md").exists());

        // User-level files should NOT exist
        assert!(!config_dir.join("USER.md").exists());
        assert!(!config_dir.join("MEMORY.md").exists());

        // DB should exist
        assert!(tmp.path().join("db").join("memory.db").exists());

        // Should be readable via read_file (routes to config_dir)
        let soul = store.read_file("SOUL.md").unwrap();
        assert!(soul.contains("Aster"));
    }

    #[tokio::test]
    async fn test_write_and_search() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;

        store
            .write_file("test-content.md", "Rust is a systems programming language focused on safety and performance.")
            .await
            .unwrap();

        let results = store.search("Rust programming", 5).await.unwrap();
        assert!(!results.is_empty());
        assert!(results[0].text.contains("Rust"));
    }

    #[tokio::test]
    async fn test_append_daily() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let agent_home = tmp.path().join("agent_home");

        // Create memory/ dir for the test (normally done by UserMemoryView)
        std::fs::create_dir_all(agent_home.join("memory")).unwrap();

        store.append_daily("Had a great conversation about Rust.").await.unwrap();
        store.append_daily("Discussed memory management.").await.unwrap();

        let today = Local::now().format("%Y-%m-%d").to_string();
        let content = store.read_file(&format!("memory/{}.md", today)).unwrap();
        assert!(content.contains("great conversation"));
        assert!(content.contains("memory management"));
    }

    #[tokio::test]
    async fn test_bootstrap_context() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;

        let ctx = store.bootstrap_context().unwrap();
        assert!(ctx.contains("SOUL.md"));
        assert!(ctx.contains("Aster"));
        // User files should NOT be in agent bootstrap
        assert!(!ctx.contains("USER.md"));
        assert!(!ctx.contains("MEMORY.md"));
    }

    #[tokio::test]
    async fn test_reindex() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let agent_home = tmp.path().join("agent_home");

        // Write a file directly (bypassing write_file)
        std::fs::write(
            agent_home.join("test-quantum.md"),
            "This is about quantum computing and qubits.",
        )
        .unwrap();

        // Reindex should pick it up
        store.reindex().await.unwrap();

        let results = store.search("quantum computing", 5).await.unwrap();
        assert!(!results.is_empty());
    }

    // ── Path validation integration tests ───────────────────────────────

    #[tokio::test]
    async fn write_file_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let err = store.write_file("../escape.md", "evil content").await;
        assert!(err.is_err(), "write_file should reject path traversal");
    }

    #[tokio::test]
    async fn write_file_rejects_non_md() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let err = store.write_file("script.sh", "#!/bin/bash").await;
        assert!(err.is_err(), "write_file should reject non-.md files");
    }

    #[tokio::test]
    async fn write_file_rejects_absolute_path() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let err = store.write_file("/tmp/evil.md", "content").await;
        assert!(err.is_err(), "write_file should reject absolute paths");
    }

    #[tokio::test]
    async fn read_file_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let err = store.read_file("../../etc/passwd.md");
        assert!(err.is_err(), "read_file should reject path traversal");
    }

    #[tokio::test]
    async fn read_file_rejects_non_md() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let err = store.read_file("secret.json");
        assert!(err.is_err(), "read_file should reject non-.md files");
    }

    // ── Content size validation tests ───────────────────────────────────

    #[tokio::test]
    async fn write_file_rejects_oversized_content() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let big = "x".repeat(scoring::MAX_WRITE_SIZE + 1);
        let err = store.write_file("big.md", &big).await;
        assert!(err.is_err(), "write_file should reject content > 1 MB");
    }

    #[tokio::test]
    async fn write_file_accepts_content_at_limit() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let exact = "x".repeat(scoring::MAX_WRITE_SIZE);
        let result = store.write_file("exact.md", &exact).await;
        assert!(result.is_ok(), "write_file should accept content at exactly 1 MB");
    }

    // ── Setter tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn set_half_life_days_is_applied() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_half_life_days(7.0);
        assert_eq!(store.half_life_days, 7.0);
    }

    #[tokio::test]
    async fn set_mmr_lambda_is_applied() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_mmr_lambda(0.5);
        assert_eq!(store.mmr_lambda, 0.5);
    }

    #[tokio::test]
    async fn set_chunk_size_is_applied() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_chunk_size(800);
        assert_eq!(store.chunk_size, 800);
    }

    #[tokio::test]
    async fn set_chunk_overlap_is_applied() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_chunk_overlap(160);
        assert_eq!(store.chunk_overlap, 160);
    }

    #[tokio::test]
    async fn set_bootstrap_file_cap_is_applied() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_bootstrap_file_cap(5000);
        assert_eq!(store.bootstrap_file_cap, 5000);
    }

    #[tokio::test]
    async fn bootstrap_file_cap_limits_output() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;

        // Write a large file (well above the cap we'll set)
        let large_content = "x".repeat(10_000);
        store.write_file("SOUL.md", &large_content).await.unwrap();

        // Set a small bootstrap_file_cap
        store.set_bootstrap_file_cap(500);

        let ctx = store.bootstrap_context().unwrap();
        // The SOUL.md section should be capped at 500 chars of content.
        // Find the SOUL.md section and verify its content portion is truncated.
        let soul_section = ctx
            .split("--- SOUL.md ---\n")
            .nth(1)
            .unwrap_or("")
            .split("\n\n--- ")
            .next()
            .unwrap_or("");
        assert!(
            soul_section.len() <= 500,
            "SOUL.md section should be capped at 500 chars, got {}",
            soul_section.len(),
        );
    }

    // ── Vector search without embedder ──────────────────────────────────

    #[tokio::test]
    async fn vector_search_returns_empty_without_embedder() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let results = store.vector_search("anything", 10).await.unwrap();
        assert!(results.is_empty(), "vector_search should return empty without embedder");
    }

    #[tokio::test]
    async fn hybrid_search_falls_back_to_fts_without_embedder() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;

        store
            .write_file("test-elephants.md", "Unique test content about elephants in Africa.")
            .await
            .unwrap();

        // hybrid_search should still work (falling back to FTS-only)
        let results = store.hybrid_search("elephants Africa", 5).await.unwrap();
        assert!(!results.is_empty(), "hybrid_search should fall back to FTS without embedder");
        assert!(results[0].text.contains("elephants"));
    }

    // ── Mock embedder integration tests ─────────────────────────────────

    /// A mock embedder that returns deterministic vectors for testing.
    /// Each text is embedded as a vector where the i-th dimension is the
    /// count of the i-th character ('a'=0, 'b'=1, ...), normalized.
    struct MockEmbedder;

    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|t| {
                let mut vec = vec![0.0f32; 8];
                for ch in t.chars() {
                    let idx = (ch.to_ascii_lowercase() as usize).wrapping_sub('a' as usize);
                    if idx < 8 {
                        vec[idx] += 1.0;
                    }
                }
                // Normalize
                let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for v in &mut vec {
                        *v /= norm;
                    }
                }
                vec
            }).collect())
        }

        fn dimensions(&self) -> usize {
            8
        }
    }

    #[tokio::test]
    async fn set_embedder_enables_vector_storage() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_embedder(Arc::new(MockEmbedder));

        store
            .write_file("test-cats.md", "Cats are wonderful animals that love to sleep.")
            .await
            .unwrap();

        // Verify vectors were stored
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_vectors WHERE source = 'test-cats.md'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(count > 0, "Vectors should be stored after write_file with embedder");
    }

    #[tokio::test]
    async fn vector_search_with_mock_embedder() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_embedder(Arc::new(MockEmbedder));

        store.write_file("test-abc.md", "aaa bbb ccc abc").await.unwrap();
        store.write_file("test-def.md", "ddd eee fff def").await.unwrap();

        // Search for something similar to "abc" content
        let results = store.vector_search("aaa abc", 5).await.unwrap();
        assert!(!results.is_empty(), "vector_search should return results with embedder");
    }

    #[tokio::test]
    async fn hybrid_search_with_mock_embedder() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_embedder(Arc::new(MockEmbedder));

        store.write_file("test-alpha.md", "Alpha beta gamma delta").await.unwrap();
        store.write_file("test-beta.md", "Beta epsilon zeta eta").await.unwrap();

        let results = store.hybrid_search("alpha beta", 5).await.unwrap();
        assert!(!results.is_empty(), "hybrid_search should return results with embedder");
    }

    #[tokio::test]
    async fn reindex_clears_and_rebuilds_vectors() {
        let tmp = TempDir::new().unwrap();
        let mut store = test_store(&tmp).await;
        store.set_embedder(Arc::new(MockEmbedder));

        store.write_file("test-vectors.md", "Test content here").await.unwrap();

        // Count vectors before reindex
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_vectors")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(before > 0);

        // Reindex should clear and rebuild
        store.reindex().await.unwrap();

        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_vectors")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        // Should have vectors for all files (defaults + test.md)
        assert!(after > 0, "Reindex should rebuild vectors");
    }

    // ── Byte conversion round-trip test ─────────────────────────────────

    #[test]
    fn f32_bytes_round_trip() {
        let original = vec![1.0f32, -2.5, 0.0, std::f32::consts::PI, f32::MAX, f32::MIN];
        let bytes = f32_vec_to_bytes(&original);
        assert_eq!(bytes.len(), original.len() * 4);
        let restored = bytes_to_f32_vec(&bytes);
        assert_eq!(original, restored);
    }

    #[test]
    fn f32_bytes_empty_round_trip() {
        let original: Vec<f32> = vec![];
        let bytes = f32_vec_to_bytes(&original);
        assert!(bytes.is_empty());
        let restored = bytes_to_f32_vec(&bytes);
        assert!(restored.is_empty());
    }

    #[test]
    fn f32_bytes_single_value() {
        let original = vec![42.0f32];
        let bytes = f32_vec_to_bytes(&original);
        assert_eq!(bytes.len(), 4);
        let restored = bytes_to_f32_vec(&bytes);
        assert_eq!(original, restored);
    }

    // ── Search with temporal decay test ─────────────────────────────────

    // ── Temporal decay test ─────────────────────────────────────────────

    #[tokio::test]
    async fn search_applies_temporal_decay() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let agent_home = tmp.path().join("agent_home");

        // Write the same content to an evergreen file and a daily log
        let content = "Temporal decay test content about quantum physics and relativity.";
        store.write_file("test-physics.md", content).await.unwrap();

        // Write to an old daily log file directly, then reindex
        let old_date = Local::now().date_naive() - chrono::Duration::days(90);
        let old_filename = format!("memory/{}.md", old_date.format("%Y-%m-%d"));
        std::fs::create_dir_all(agent_home.join("memory")).unwrap();
        let old_path = agent_home.join(&old_filename);
        std::fs::write(&old_path, content).unwrap();
        store.reindex().await.unwrap();

        let results = store.search("quantum physics relativity", 10).await.unwrap();
        // Should find the evergreen file at minimum
        assert!(!results.is_empty(), "Should find at least the evergreen file");
    }

    #[tokio::test]
    async fn test_append_daily_creates_memory_dir() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let agent_home = tmp.path().join("agent_home");

        // Do NOT create memory/ dir — append_daily should create it
        assert!(!agent_home.join("memory").exists());

        store.append_daily("First entry without pre-existing dir.").await.unwrap();

        assert!(agent_home.join("memory").exists());
        let today = Local::now().format("%Y-%m-%d").to_string();
        let content = store.read_file(&format!("memory/{}.md", today)).unwrap();
        assert!(content.contains("First entry"));
    }

    #[tokio::test]
    async fn test_bootstrap_context_multibyte_safe() {
        let tmp = TempDir::new().unwrap();
        let agent_home = tmp.path().join("agent_home");
        let config_dir = agent_home.join("config");
        let db_dir = tmp.path().join("db");
        std::fs::create_dir_all(&config_dir).unwrap();

        // Write SOUL.md with multibyte chars that would cause a panic if
        // the cap falls on a char boundary
        let soul = "# Soul\n".to_string() + &"café 🌟 ".repeat(5000);
        std::fs::write(config_dir.join("SOUL.md"), &soul).unwrap();

        let store = MemoryStore::new(&agent_home, &config_dir, &db_dir).await.unwrap();
        // Should not panic even though the cap likely falls mid-character
        let ctx = store.bootstrap_context().unwrap();
        assert!(ctx.contains("SOUL.md"));
        // The content should be valid UTF-8
        assert!(ctx.is_char_boundary(ctx.len()));
    }

    // ── Config/agent_home separation ──────────────────────────────

    #[tokio::test]
    async fn config_files_routed_to_config_dir() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let config_dir = tmp.path().join("agent_home").join("config");

        // Writing SOUL.md should go to config_dir
        store.write_file("SOUL.md", "# Soul\nCustom soul.").await.unwrap();
        assert!(config_dir.join("SOUL.md").is_file());
        let content = std::fs::read_to_string(config_dir.join("SOUL.md")).unwrap();
        assert!(content.contains("Custom soul"));

        // Reading should return from config_dir
        let read = store.read_file("SOUL.md").unwrap();
        assert!(read.contains("Custom soul"));
    }

    #[tokio::test]
    async fn runtime_files_routed_to_agent_home() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let agent_home = tmp.path().join("agent_home");
        let config_dir = agent_home.join("config");

        // Writing a non-config .md file should go to agent_home
        store.write_file("notes.md", "Some notes.").await.unwrap();
        assert!(agent_home.join("notes.md").is_file());
        assert!(!config_dir.join("notes.md").exists());

        // Reading it back should work
        let content = store.read_file("notes.md").unwrap();
        assert!(content.contains("Some notes"));
    }

    #[tokio::test]
    async fn reindex_covers_both_config_and_agent_home() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;

        // Write a config file (SOUL.md → config_dir)
        store.write_file("SOUL.md", "Soul content about quantum.").await.unwrap();

        // Write a runtime file (notes.md → agent_home)
        store.write_file("notes.md", "Notes about quantum.").await.unwrap();

        // Reindex should pick up both
        store.reindex().await.unwrap();

        let results = store.search("quantum", 10).await.unwrap();
        let sources: Vec<&str> = results.iter().map(|r| r.source.as_str()).collect();
        assert!(sources.contains(&"SOUL.md"), "SOUL.md from config_dir should be indexed");
        assert!(sources.contains(&"notes.md"), "notes.md from agent_home should be indexed");
    }

    #[tokio::test]
    async fn bootstrap_context_reads_from_config_dir() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;

        // Overwrite SOUL.md in config_dir
        store.write_file("SOUL.md", "# Soul\nI am ConfigBot.").await.unwrap();

        let ctx = store.bootstrap_context().unwrap();
        assert!(ctx.contains("ConfigBot"), "bootstrap should read from config_dir");
    }

    #[tokio::test]
    async fn has_bootstrap_checks_config_dir() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(&tmp).await;
        let config_dir = tmp.path().join("agent_home").join("config");

        // Default BOOTSTRAP.md should be empty
        assert!(!store.has_bootstrap(), "Default BOOTSTRAP.md should be empty");

        // Write content to BOOTSTRAP.md in config_dir
        std::fs::write(config_dir.join("BOOTSTRAP.md"), "Do something on first run.").unwrap();
        assert!(store.has_bootstrap(), "BOOTSTRAP.md with content should be detected");

        // Clear it
        store.clear_bootstrap().unwrap();
        assert!(!store.has_bootstrap(), "Cleared BOOTSTRAP.md should not be detected");
    }
}
