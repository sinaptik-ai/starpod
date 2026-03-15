use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::Local;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::debug;

use starpod_core::{StarpodError, Result};

use crate::defaults;
use crate::indexer::reindex_source;
use crate::schema;

/// Maximum characters to include from a single file in bootstrap context.
const BOOTSTRAP_FILE_CAP: usize = 20_000;

/// A search result from the FTS5 index.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Source file the chunk came from.
    pub source: String,
    /// The matching text chunk.
    pub text: String,
    /// Starting line number (1-indexed).
    pub line_start: usize,
    /// Ending line number.
    pub line_end: usize,
    /// FTS5 rank score (lower = better match).
    pub rank: f64,
}

/// The main memory store -- manages markdown files on disk and an FTS5 index in SQLite.
pub struct MemoryStore {
    data_dir: PathBuf,
    pool: SqlitePool,
}

impl MemoryStore {
    /// Create a new MemoryStore, initializing directories, database, and default files.
    pub async fn new(data_dir: &Path) -> Result<Self> {
        // Create directory structure
        std::fs::create_dir_all(data_dir)
            .map_err(StarpodError::Io)?;
        std::fs::create_dir_all(data_dir.join("memory"))
            .map_err(StarpodError::Io)?;
        std::fs::create_dir_all(data_dir.join("knowledge"))
            .map_err(StarpodError::Io)?;

        // Open SQLite pool
        let db_path = data_dir.join("memory.db");
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
            data_dir: data_dir.to_path_buf(),
            pool,
        };

        // Seed default files if they don't exist
        store.seed_defaults()?;

        // Initial index
        store.reindex().await?;

        Ok(store)
    }

    /// Seed default markdown files on first run.
    fn seed_defaults(&self) -> Result<()> {
        let files = [
            ("SOUL.md", defaults::DEFAULT_SOUL),
            ("USER.md", defaults::DEFAULT_USER),
            ("MEMORY.md", defaults::DEFAULT_MEMORY),
            ("HEARTBEAT.md", defaults::DEFAULT_HEARTBEAT),
        ];

        for (name, content) in &files {
            let path = self.data_dir.join(name);
            if !path.exists() {
                debug!(file = %name, "Seeding default file");
                std::fs::write(&path, content)?;
            }
        }

        Ok(())
    }

    /// Build bootstrap context from SOUL.md + USER.md + MEMORY.md + recent daily logs.
    ///
    /// Each file is capped at `BOOTSTRAP_FILE_CAP` characters.
    pub fn bootstrap_context(&self) -> Result<String> {
        let mut parts = Vec::new();

        // Core files
        for name in &["SOUL.md", "USER.md", "MEMORY.md"] {
            let content = self.read_file(name)?;
            let capped = if content.len() > BOOTSTRAP_FILE_CAP {
                &content[..BOOTSTRAP_FILE_CAP]
            } else {
                &content
            };
            parts.push(format!("--- {} ---\n{}", name, capped));
        }

        // Recent daily logs (last 3 days)
        let today = Local::now().format("%Y-%m-%d").to_string();
        let memory_dir = self.data_dir.join("memory");
        if memory_dir.exists() {
            let mut entries: Vec<_> = std::fs::read_dir(&memory_dir)
                .map_err(StarpodError::Io)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map_or(false, |ext| ext == "md")
                })
                .collect();
            entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
            entries.truncate(3);

            for entry in entries {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    let capped = if content.len() > BOOTSTRAP_FILE_CAP {
                        &content[..BOOTSTRAP_FILE_CAP]
                    } else {
                        &content
                    };
                    parts.push(format!("--- daily/{} ---\n{}", name, capped));
                }
            }
        }

        let _ = today; // used implicitly by date ordering

        Ok(parts.join("\n\n"))
    }

    /// Full-text search across all indexed content.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
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

        let mut results = Vec::new();
        for row in rows {
            results.push(SearchResult {
                source: row.get::<String, _>("source"),
                text: row.get::<String, _>("chunk_text"),
                line_start: row.get::<i64, _>("line_start") as usize,
                line_end: row.get::<i64, _>("line_end") as usize,
                rank: row.get::<f64, _>("rank"),
            });
        }

        Ok(results)
    }

    /// Read a file from the data directory.
    pub fn read_file(&self, name: &str) -> Result<String> {
        let path = self.data_dir.join(name);
        if !path.exists() {
            return Ok(String::new());
        }
        std::fs::read_to_string(&path).map_err(StarpodError::Io)
    }

    /// Write a file to the data directory and reindex it.
    pub async fn write_file(&self, name: &str, content: &str) -> Result<()> {
        let path = self.data_dir.join(name);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&path, content)?;

        // Reindex this file
        reindex_source(&self.pool, name, content).await?;

        Ok(())
    }

    /// Append a timestamped entry to today's daily log.
    pub async fn append_daily(&self, text: &str) -> Result<()> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let filename = format!("memory/{}.md", today);
        let path = self.data_dir.join(&filename);

        let timestamp = Local::now().format("%H:%M:%S").to_string();
        let entry = format!("\n## {}\n{}\n", timestamp, text);

        let mut content = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            format!("# Daily Log — {}\n", today)
        };

        content.push_str(&entry);
        std::fs::write(&path, &content)?;

        // Reindex the daily file
        reindex_source(&self.pool, &filename, &content).await?;

        Ok(())
    }

    /// Full reindex of all markdown files in the data directory.
    pub async fn reindex(&self) -> Result<()> {
        // Clear all existing entries
        sqlx::query("DELETE FROM memory_fts")
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to clear FTS: {}", e)))?;

        // Index top-level .md files
        self.index_dir(&self.data_dir.clone(), "").await?;

        // Index memory/ subdirectory
        let memory_dir = self.data_dir.join("memory");
        if memory_dir.exists() {
            self.index_dir(&memory_dir, "memory/").await?;
        }

        // Index knowledge/ subdirectory
        let knowledge_dir = self.data_dir.join("knowledge");
        if knowledge_dir.exists() {
            self.index_dir(&knowledge_dir, "knowledge/").await?;
        }

        Ok(())
    }

    /// Index all .md files in a directory with a source prefix.
    async fn index_dir(&self, dir: &Path, prefix: &str) -> Result<()> {
        let entries = std::fs::read_dir(dir).map_err(StarpodError::Io)?;

        for entry in entries {
            let entry = entry.map_err(StarpodError::Io)?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "md") {
                let filename = entry.file_name().to_string_lossy().to_string();
                let source = format!("{}{}", prefix, filename);
                let content = std::fs::read_to_string(&path)?;
                reindex_source(&self.pool, &source, &content).await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_new_seeds_defaults() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::new(tmp.path()).await.unwrap();

        // Default files should exist
        assert!(tmp.path().join("SOUL.md").exists());
        assert!(tmp.path().join("USER.md").exists());
        assert!(tmp.path().join("MEMORY.md").exists());

        // Should be readable
        let soul = store.read_file("SOUL.md").unwrap();
        assert!(soul.contains("Aster"));
    }

    #[tokio::test]
    async fn test_write_and_search() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::new(tmp.path()).await.unwrap();

        store
            .write_file("knowledge/rust.md", "Rust is a systems programming language focused on safety and performance.")
            .await
            .unwrap();

        let results = store.search("Rust programming", 5).await.unwrap();
        assert!(!results.is_empty());
        assert!(results[0].text.contains("Rust"));
        assert_eq!(results[0].source, "knowledge/rust.md");
    }

    #[tokio::test]
    async fn test_append_daily() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::new(tmp.path()).await.unwrap();

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
        let store = MemoryStore::new(tmp.path()).await.unwrap();

        let ctx = store.bootstrap_context().unwrap();
        assert!(ctx.contains("SOUL.md"));
        assert!(ctx.contains("USER.md"));
        assert!(ctx.contains("MEMORY.md"));
        assert!(ctx.contains("Aster"));
    }

    #[tokio::test]
    async fn test_reindex() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::new(tmp.path()).await.unwrap();

        // Write a file directly (bypassing write_file)
        std::fs::write(
            tmp.path().join("knowledge").join("test.md"),
            "This is about quantum computing and qubits.",
        )
        .unwrap();

        // Reindex should pick it up
        store.reindex().await.unwrap();

        let results = store.search("quantum computing", 5).await.unwrap();
        assert!(!results.is_empty());
    }
}
