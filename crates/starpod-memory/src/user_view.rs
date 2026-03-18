//! Per-user memory view — overlays user-specific files on top of shared agent memory.
//!
//! In multi-user mode, each user has their own `USER.md`, `MEMORY.md`, `memory/` daily logs,
//! while sharing the agent's `SOUL.md` and search index.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Local;
use tracing::debug;

use starpod_core::{StarpodError, Result};

use crate::scoring;
use crate::store::{MemoryStore, SearchResult};

/// A per-user view over the shared agent memory store.
///
/// - `SOUL.md`, `HEARTBEAT.md`, `BOOT.md`, `BOOTSTRAP.md` come from the shared agent store.
/// - `USER.md`, `MEMORY.md`, `memory/*` (daily logs) come from the user's directory.
/// - Search queries both the agent-level index and user-level files.
/// - Writes route to the appropriate location based on the file path.
pub struct UserMemoryView {
    /// Shared agent-level memory store (SOUL.md, search index).
    agent_store: Arc<MemoryStore>,
    /// Per-user directory (.starpod/users/<id>/).
    user_dir: PathBuf,
}

impl UserMemoryView {
    /// Create a new per-user memory view.
    ///
    /// Ensures the user directory and required subdirectories exist.
    pub fn new(agent_store: Arc<MemoryStore>, user_dir: PathBuf) -> Result<Self> {
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

        Ok(Self {
            agent_store,
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
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        // Search the agent-level index (includes SOUL.md, knowledge/, etc.)
        let mut results = self.agent_store.search(query, limit).await?;

        // Also search user-level files by checking if they contain the query terms
        // For now, user files are not separately indexed — they'll be found if
        // the agent store includes them. In a full implementation, we'd maintain
        // a separate user-level FTS index.
        // TODO: Add user-level FTS index for per-user memory files

        results.truncate(limit);
        Ok(results)
    }

    /// Write a file, routing to the appropriate location.
    ///
    /// - `USER.md`, `MEMORY.md`, `memory/*` → user directory
    /// - Everything else → agent store (shared)
    pub async fn write_file(&self, name: &str, content: &str) -> Result<()> {
        if is_user_file(name) {
            scoring::validate_path(name, &self.user_dir)?;
            scoring::validate_content_size(content)?;

            let path = self.user_dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            debug!(file = %name, "Wrote user-level file");
            Ok(())
        } else {
            // Delegate to agent store for shared files
            self.agent_store.write_file(name, content).await
        }
    }

    /// Append to the user's daily log.
    pub async fn append_daily(&self, text: &str) -> Result<()> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let filename = format!("memory/{}.md", today);
        let path = self.user_dir.join(&filename);

        let timestamp = Local::now().format("%H:%M:%S").to_string();
        let entry = format!("\n## {}\n{}\n", timestamp, text);

        let mut content = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            format!("# Daily Log — {}\n", today)
        };

        content.push_str(&entry);
        std::fs::write(&path, &content)?;

        Ok(())
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
        let _view = UserMemoryView::new(store, user_dir.clone()).unwrap();

        assert!(user_dir.exists());
        assert!(user_dir.join("memory").exists());
        assert!(user_dir.join("USER.md").exists());
        assert!(user_dir.join("MEMORY.md").exists());
    }

    #[tokio::test]
    async fn user_view_bootstrap_context() {
        let (_tmp, store, user_dir) = setup().await;
        let view = UserMemoryView::new(store, user_dir.clone()).unwrap();

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
        let view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).unwrap();

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
        let view = UserMemoryView::new(store, user_dir.clone()).unwrap();

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
        let view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).unwrap();

        // Read USER.md from user dir
        std::fs::write(user_dir.join("USER.md"), "custom user data").unwrap();
        let content = view.read_file("USER.md").unwrap();
        assert!(content.contains("custom user data"));

        // Read SOUL.md from agent store
        let content = view.read_file("SOUL.md").unwrap();
        assert!(content.contains("Aster"));
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
        let _view = UserMemoryView::new(Arc::clone(&store), user_dir.clone()).unwrap();

        // Attempt path traversal via user file path
        let result = read_user_file(&user_dir, "memory/../../../etc/passwd");
        assert!(result.is_err(), "read_user_file should reject path traversal");
    }
}
