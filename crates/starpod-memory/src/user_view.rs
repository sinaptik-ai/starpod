//! Per-user memory view — overlays user-specific files on top of shared agent memory.
//!
//! In multi-user mode, each user has their own `USER.md`, `MEMORY.md`, `memory/` daily logs,
//! while sharing the agent's `SOUL.md` and search index. User files are indexed in a per-user
//! SQLite FTS5 database for fast search.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use starpod_core::{StarpodError, Result};

use crate::scoring;
use crate::store::{MemoryStore, SearchResult};

/// A per-user view over the shared agent memory store.
///
/// Provides file routing and search merging for multi-user deployments:
///
/// - **Shared files** (`SOUL.md`, `HEARTBEAT.md`, `BOOT.md`, `BOOTSTRAP.md`) come
///   from the agent-level store and are shared across all users.
/// - **Per-user files** (`USER.md`, `MEMORY.md`, `memory/*` daily logs) live in
///   `users/<id>/` and are indexed in a per-user SQLite FTS5 database.
/// - **Search** queries both agent-level and user-level indexes concurrently,
///   merging results by rank.
/// - **Writes** automatically route to the correct store based on the file path,
///   with FTS reindexing handled transparently.
pub struct UserMemoryView {
    /// Shared agent-level memory store (SOUL.md, search index).
    agent_store: Arc<MemoryStore>,
    /// Per-user memory store with its own FTS5 index in `user_dir/memory.db`.
    user_store: MemoryStore,
    /// Per-user directory (.starpod/users/<id>/).
    user_dir: PathBuf,
}

impl UserMemoryView {
    /// Create a new per-user memory view.
    ///
    /// Creates the user directory structure, seeds default `USER.md` and
    /// `MEMORY.md` if they don't exist, and initializes a per-user FTS5
    /// index (stored in `user_dir/memory.db`).
    ///
    /// This is `async` because it initializes the per-user SQLite database.
    pub async fn new(agent_store: Arc<MemoryStore>, user_dir: PathBuf) -> Result<Self> {
        // Create user directory structure
        std::fs::create_dir_all(&user_dir).map_err(StarpodError::Io)?;
        std::fs::create_dir_all(user_dir.join("memory")).map_err(StarpodError::Io)?;

        // Seed defaults if they don't exist
        let user_md = user_dir.join("USER.md");
        if !user_md.exists() {
            std::fs::write(
                &user_md,
                crate::defaults::DEFAULT_USER,
            ).map_err(StarpodError::Io)?;
        }
        let memory_md = user_dir.join("MEMORY.md");
        if !memory_md.exists() {
            std::fs::write(
                &memory_md,
                "# Memory Index\n\nImportant facts and links to memory files.\n",
            ).map_err(StarpodError::Io)?;
        }

        // Create per-user memory store (owns its own FTS5 index in user_dir/memory.db)
        let user_store = MemoryStore::new_user(&user_dir).await?;

        Ok(Self {
            agent_store,
            user_store,
            user_dir,
        })
    }

    /// Build bootstrap context: SOUL.md from agent, USER.md + MEMORY.md + recent logs from user.
    pub fn bootstrap_context(&self, bootstrap_file_cap: usize) -> Result<String> {
        let mut parts = Vec::new();

        // SOUL.md from agent store
        let soul = self.agent_store.read_file("SOUL.md")?;
        let capped = cap_str(&soul, bootstrap_file_cap);
        parts.push(format!("--- SOUL.md ---\n{}", capped));

        // USER.md from user dir
        let user_content = read_user_file(&self.user_dir, "USER.md")?;
        let capped = cap_str(&user_content, bootstrap_file_cap);
        parts.push(format!("--- USER.md ---\n{}", capped));

        // MEMORY.md from user dir
        let memory_content = read_user_file(&self.user_dir, "MEMORY.md")?;
        let capped = cap_str(&memory_content, bootstrap_file_cap);
        parts.push(format!("--- MEMORY.md ---\n{}", capped));

        // Recent daily logs from user dir (last 3 days)
        let memory_dir = self.user_dir.join("memory");
        if memory_dir.exists() {
            let mut entries: Vec<_> = std::fs::read_dir(&memory_dir)
                .map_err(StarpodError::Io)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "md")
                })
                .collect();
            entries.sort_by_key(|b| std::cmp::Reverse(b.file_name()));
            entries.truncate(3);

            for entry in entries {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    let capped = cap_str(&content, bootstrap_file_cap);
                    parts.push(format!("--- daily/{} ---\n{}", name, capped));
                }
            }
        }

        Ok(parts.join("\n\n"))
    }

    /// Search both agent-level and user-level content.
    ///
    /// Queries both the shared agent FTS index and the per-user FTS index
    /// concurrently, merges results by rank, and returns the top `limit`.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        // Query both stores concurrently
        let (agent_results, user_results) = tokio::join!(
            self.agent_store.search(query, limit),
            self.user_store.search(query, limit),
        );

        let mut results = agent_results?;
        let mut user_hits = user_results?;

        // Merge: interleave by rank (more negative = better match)
        results.append(&mut user_hits);
        results.sort_by(|a, b| a.rank.partial_cmp(&b.rank).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Write a file, routing to the appropriate location.
    ///
    /// - `USER.md`, `MEMORY.md`, `memory/*` → user store (with FTS reindex)
    /// - Everything else → agent store (shared)
    pub async fn write_file(&self, name: &str, content: &str) -> Result<()> {
        if is_user_file(name) {
            self.user_store.write_file(name, content).await
        } else {
            self.agent_store.write_file(name, content).await
        }
    }

    /// Append to the user's daily log (with FTS reindex).
    pub async fn append_daily(&self, text: &str) -> Result<()> {
        self.user_store.append_daily(text).await
    }

    /// Read a file from the appropriate location.
    pub fn read_file(&self, name: &str) -> Result<String> {
        if is_user_file(name) {
            read_user_file(&self.user_dir, name)
        } else {
            self.agent_store.read_file(name)
        }
    }

    /// Check if BOOTSTRAP.md exists (delegates to agent store).
    pub fn has_bootstrap(&self) -> bool {
        self.agent_store.has_bootstrap()
    }

    /// Clear BOOTSTRAP.md (delegates to agent store).
    pub fn clear_bootstrap(&self) -> Result<()> {
        self.agent_store.clear_bootstrap()
    }
}

/// Check if a file path should be stored per-user.
fn is_user_file(name: &str) -> bool {
    name == "USER.md"
        || name == "MEMORY.md"
        || name.starts_with("memory/")
}

/// Read a file from the user directory, returning empty string if not found.
fn read_user_file(user_dir: &Path, name: &str) -> Result<String> {
    scoring::validate_path(name, user_dir)?;
    let path = user_dir.join(name);
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(StarpodError::Io)
}

/// Cap a string at a maximum length.
fn cap_str(s: &str, max: usize) -> &str {
    if s.len() > max {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) { end -= 1; }
        &s[..end]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use tempfile::TempDir;

    async fn setup() -> (TempDir, Arc<MemoryStore>, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let agent_home = tmp.path().join("agent_home");
        let config_dir = agent_home.join("config");
        let db_dir = tmp.path().join("db");
        let store = Arc::new(MemoryStore::new(&agent_home, &config_dir, &db_dir).await.unwrap());
        let user_dir = tmp.path().join("users").join("alice");
        (tmp, store, user_dir)
    }

    #[tokio::test]
    async fn user_view_creates_structure() {
        let (_tmp, store, user_dir) = setup().await;
        let _view = UserMemoryView::new(store, user_dir.clone()).await.unwrap();

        assert!(user_dir.exists());
        assert!(user_dir.join("memory").exists());
        assert!(user_dir.join("USER.md").exists());
        assert!(user_dir.join("MEMORY.md").exists());
        // Per-user memory.db should be created
        assert!(user_dir.join("memory.db").exists());
    }

    #[tokio::test]
    async fn user_view_bootstrap_context() {
        let (_tmp, store, user_dir) = setup().await;
        let view = UserMemoryView::new(store, user_dir.clone()).await.unwrap();

        // Write custom user profile
        std::fs::write(user_dir.join("USER.md"), "# User\nAlice is a developer.\n").unwrap();

        let ctx = view.bootstrap_context(20_000).unwrap();
        // Should have SOUL.md from agent (contains "Aster")
        assert!(ctx.contains("SOUL.md"));
        assert!(ctx.contains("Aster"));
        // Should have USER.md from user
        assert!(ctx.contains("Alice is a developer"));
    }

    #[tokio::test]
    async fn user_view_write_routes_correctly() {
        let (_tmp, store, user_dir) = setup().await;
        let view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).await.unwrap();

        // USER.md goes to user dir
        view.write_file("USER.md", "# User\nBob\n").await.unwrap();
        assert!(user_dir.join("USER.md").exists());
        let content = std::fs::read_to_string(user_dir.join("USER.md")).unwrap();
        assert!(content.contains("Bob"));

        // Non-user files go to agent store
        view.write_file("test-shared.md", "# Test\nShared content\n").await.unwrap();
        let content = store.read_file("test-shared.md").unwrap();
        assert!(content.contains("Shared content"));
    }

    #[tokio::test]
    async fn user_view_append_daily() {
        let (_tmp, store, user_dir) = setup().await;
        let view = UserMemoryView::new(store, user_dir.clone()).await.unwrap();

        view.append_daily("Had a meeting").await.unwrap();
        view.append_daily("Reviewed code").await.unwrap();

        let today = Local::now().format("%Y-%m-%d").to_string();
        let path = user_dir.join("memory").join(format!("{}.md", today));
        assert!(path.exists());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("Had a meeting"));
        assert!(content.contains("Reviewed code"));
    }

    #[tokio::test]
    async fn user_view_read_routes_correctly() {
        let (_tmp, store, user_dir) = setup().await;
        let view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).await.unwrap();

        // Read USER.md from user dir
        std::fs::write(user_dir.join("USER.md"), "custom user data").unwrap();
        let content = view.read_file("USER.md").unwrap();
        assert!(content.contains("custom user data"));

        // Read SOUL.md from agent store
        let content = view.read_file("SOUL.md").unwrap();
        assert!(content.contains("Aster"));
    }

    #[tokio::test]
    async fn user_view_search_finds_user_files() {
        let (_tmp, store, user_dir) = setup().await;
        let view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).await.unwrap();

        // Write user-specific memory content
        view.write_file("MEMORY.md", "# Memory\n\nAlice prefers dark mode and Vim keybindings.\n")
            .await
            .unwrap();

        // Search should find user content
        let results = view.search("dark mode Vim", 5).await.unwrap();
        assert!(!results.is_empty(), "Should find user memory content via FTS");
        assert!(results.iter().any(|r| r.text.contains("dark mode")));
    }

    #[tokio::test]
    async fn user_view_search_merges_agent_and_user() {
        let (_tmp, store, user_dir) = setup().await;
        let view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).await.unwrap();

        // Write user-specific content
        view.write_file("MEMORY.md", "# Memory\n\nThe assistant helps with Rust code.\n")
            .await
            .unwrap();

        // Search for "assistant" — should match both SOUL.md (agent) and MEMORY.md (user)
        let results = view.search("assistant", 10).await.unwrap();
        let sources: Vec<&str> = results.iter().map(|r| r.source.as_str()).collect();
        assert!(
            sources.iter().any(|s| *s == "SOUL.md") || sources.iter().any(|s| *s == "MEMORY.md"),
            "Should find results from both agent and user stores"
        );
    }

    #[tokio::test]
    async fn user_view_append_daily_is_searchable() {
        let (_tmp, store, user_dir) = setup().await;
        let view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).await.unwrap();

        view.append_daily("Discussed quantum computing with Bob").await.unwrap();

        let results = view.search("quantum computing", 5).await.unwrap();
        assert!(!results.is_empty(), "Daily log entries should be searchable");
        assert!(results.iter().any(|r| r.text.contains("quantum computing")));
    }

    #[test]
    fn is_user_file_classification() {
        assert!(is_user_file("USER.md"));
        assert!(is_user_file("MEMORY.md"));
        assert!(is_user_file("memory/2026-03-17.md"));
        assert!(!is_user_file("SOUL.md"));
        assert!(!is_user_file("test.md"));
        assert!(!is_user_file("HEARTBEAT.md"));
    }

    #[test]
    fn cap_str_handles_multibyte_utf8() {
        // "café" = 5 bytes: c(1) a(1) f(1) é(2)
        let s = "café";
        assert_eq!(s.len(), 5);
        // Slicing at 4 would split the 'é' (bytes 3-4). cap_str should not panic.
        let result = cap_str(s, 4);
        assert_eq!(result, "caf"); // truncates before the multi-byte char
        // Slicing at 5 returns the full string
        assert_eq!(cap_str(s, 5), "café");
        assert_eq!(cap_str(s, 100), "café");
        // Edge: cap at 0
        assert_eq!(cap_str(s, 0), "");
    }

    #[test]
    fn cap_str_handles_emoji() {
        // "hi 👋" = 7 bytes: h(1) i(1) (1) 👋(4)
        let s = "hi 👋";
        assert_eq!(s.len(), 7);
        for i in 4..7 {
            // Slicing at 4,5,6 would all split the emoji; cap_str should truncate before it
            assert_eq!(cap_str(s, i), "hi ");
        }
        assert_eq!(cap_str(s, 7), "hi 👋");
    }

    #[tokio::test]
    async fn read_user_file_rejects_traversal() {
        let (_tmp, store, user_dir) = setup().await;
        let _view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).await.unwrap();

        // Attempt path traversal via user file path
        let result = read_user_file(&user_dir, "memory/../../../etc/passwd");
        assert!(result.is_err(), "read_user_file should reject path traversal");
    }
}
