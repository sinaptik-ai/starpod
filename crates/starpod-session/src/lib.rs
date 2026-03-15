mod schema;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::debug;
use uuid::Uuid;

use starpod_core::{StarpodError, Result};

/// A channel that sessions are scoped to.
#[derive(Debug, Clone, PartialEq)]
pub enum Channel {
    /// Explicit sessions — client controls lifecycle (web, REPL, CLI).
    Main,
    /// Time-gap sessions — new session after inactivity threshold (6h).
    Telegram,
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Main => "main",
            Channel::Telegram => "telegram",
        }
    }

    pub fn from_channel_str(s: &str) -> Self {
        match s {
            "telegram" => Channel::Telegram,
            _ => Channel::Main,
        }
    }

    /// Inactivity gap (in minutes) that triggers a new session.
    /// `None` means no time-gap logic (explicit sessions).
    pub fn gap_minutes(&self) -> Option<i64> {
        match self {
            Channel::Main => None,
            Channel::Telegram => Some(360), // 6 hours
        }
    }
}

/// Decision from session resolution on whether to continue or start a new session.
#[derive(Debug, Clone)]
pub enum SessionDecision {
    /// Continue an existing session (contains session ID).
    Continue(String),
    /// Start a new session.
    New,
}

/// Metadata for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: String,
    pub last_message_at: String,
    pub is_closed: bool,
    pub summary: Option<String>,
    pub title: Option<String>,
    pub message_count: i64,
    pub channel: String,
    pub channel_session_key: Option<String>,
}

/// A stored message in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

/// Usage record for a single turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
    pub model: String,
}

/// Manages session lifecycle — creation, channel-aware resolution, closure, and usage tracking.
pub struct SessionManager {
    pool: SqlitePool,
    #[allow(dead_code)] // used in future phases for JSONL transcript storage
    sessions_dir: PathBuf,
}

impl SessionManager {
    /// Create a new SessionManager.
    pub async fn new(db_path: &Path, sessions_dir: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir_all(sessions_dir)?;

        let opts = SqliteConnectOptions::from_str(
            &format!("sqlite://{}?mode=rwc", db_path.display()),
        )
        .map_err(|e| StarpodError::Database(format!("Invalid DB path: {}", e)))?;

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to open session db: {}", e)))?;

        schema::run_migrations(&pool).await?;

        Ok(Self {
            pool,
            sessions_dir: sessions_dir.to_path_buf(),
        })
    }

    /// Create a SessionManager from an existing pool (for testing).
    #[cfg(test)]
    async fn from_pool(pool: SqlitePool, sessions_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(sessions_dir)?;
        schema::run_migrations(&pool).await?;
        Ok(Self {
            pool,
            sessions_dir: sessions_dir.to_path_buf(),
        })
    }

    /// Resolve session for a given channel and key.
    ///
    /// - **Main** (explicit): always continues the matching open session if one exists.
    /// - **Telegram** (time-gap): continues if last message was within the gap threshold,
    ///   otherwise auto-closes the old session and returns `New`.
    ///
    /// `gap_minutes` overrides the channel's default inactivity gap. Pass `None`
    /// to use the channel's built-in default (e.g. 360 min for Telegram).
    pub async fn resolve_session(
        &self,
        channel: &Channel,
        key: &str,
        gap_minutes: Option<i64>,
    ) -> Result<SessionDecision> {
        let row = sqlx::query(
            "SELECT id, last_message_at
             FROM session_metadata
             WHERE channel = ?1 AND channel_session_key = ?2 AND is_closed = 0
             ORDER BY last_message_at DESC
             LIMIT 1",
        )
        .bind(channel.as_str())
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Resolve session query failed: {}", e)))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(SessionDecision::New),
        };

        let session_id: String = row.get("id");

        // For explicit channels (no gap), always continue.
        // If the caller provided an override, use it; otherwise fall back to the channel default.
        let gap_threshold = match gap_minutes.or_else(|| channel.gap_minutes()) {
            None => {
                debug!(session_id = %session_id, channel = %channel.as_str(), "Continuing session (explicit channel)");
                return Ok(SessionDecision::Continue(session_id));
            }
            Some(gap) => gap,
        };

        // For time-gap channels, check inactivity
        let last_msg_str: String = row.get("last_message_at");
        let last_msg = DateTime::parse_from_rfc3339(&last_msg_str)
            .map_err(|e| StarpodError::Session(format!("Bad timestamp: {}", e)))?
            .with_timezone(&Utc);

        let gap = Utc::now() - last_msg;

        if gap < Duration::minutes(gap_threshold) {
            debug!(session_id = %session_id, gap_mins = gap.num_minutes(), "Continuing session (within gap)");
            Ok(SessionDecision::Continue(session_id))
        } else {
            debug!(session_id = %session_id, gap_mins = gap.num_minutes(), "Auto-closing session (gap exceeded)");
            self.close_session(&session_id, "Auto-closed: inactivity").await?;
            Ok(SessionDecision::New)
        }
    }

    /// Create a new session for a channel and key, returning its ID.
    pub async fn create_session(
        &self,
        channel: &Channel,
        key: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO session_metadata (id, created_at, last_message_at, is_closed, message_count, channel, channel_session_key)
             VALUES (?1, ?2, ?2, 0, 0, ?3, ?4)",
        )
        .bind(&id)
        .bind(&now)
        .bind(channel.as_str())
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Create session failed: {}", e)))?;

        debug!(session_id = %id, channel = %channel.as_str(), key = %key, "Created new session");
        Ok(id)
    }

    /// Mark a session as closed with an optional summary.
    pub async fn close_session(&self, id: &str, summary: &str) -> Result<()> {
        sqlx::query("UPDATE session_metadata SET is_closed = 1, summary = ?2 WHERE id = ?1")
            .bind(id)
            .bind(summary)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Close session failed: {}", e)))?;

        debug!(session_id = %id, "Closed session");
        Ok(())
    }

    /// Update the last_message_at timestamp and increment message_count.
    pub async fn touch_session(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE session_metadata SET last_message_at = ?2, message_count = message_count + 1 WHERE id = ?1",
        )
        .bind(id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Touch session failed: {}", e)))?;
        Ok(())
    }

    /// Set the session title if it hasn't been set yet.
    pub async fn set_title_if_empty(&self, id: &str, title: &str) -> Result<()> {
        let truncated = if title.len() > 100 {
            format!("{}...", &title[..100])
        } else {
            title.to_string()
        };
        sqlx::query(
            "UPDATE session_metadata SET title = ?2 WHERE id = ?1 AND title IS NULL",
        )
        .bind(id)
        .bind(&truncated)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Set title failed: {}", e)))?;
        Ok(())
    }

    /// Record token usage for a turn.
    pub async fn record_usage(&self, session_id: &str, usage: &UsageRecord, turn: u32) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO usage_stats (session_id, turn, input_tokens, output_tokens, cache_read, cache_write, cost_usd, model, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(session_id)
        .bind(turn as i64)
        .bind(usage.input_tokens as i64)
        .bind(usage.output_tokens as i64)
        .bind(usage.cache_read as i64)
        .bind(usage.cache_write as i64)
        .bind(usage.cost_usd)
        .bind(&usage.model)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Record usage failed: {}", e)))?;

        Ok(())
    }

    /// List sessions, most recent first.
    pub async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionMeta>> {
        let rows = sqlx::query(
            "SELECT id, created_at, last_message_at, is_closed, summary, title, message_count, channel, channel_session_key
             FROM session_metadata
             ORDER BY last_message_at DESC
             LIMIT ?1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Query failed: {}", e)))?;

        let sessions: Vec<SessionMeta> = rows
            .iter()
            .map(|row| session_meta_from_row(row))
            .collect();

        Ok(sessions)
    }

    /// Get a specific session by ID.
    pub async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        let row = sqlx::query(
            "SELECT id, created_at, last_message_at, is_closed, summary, title, message_count, channel, channel_session_key
             FROM session_metadata WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Get session failed: {}", e)))?;

        Ok(row.map(|r| session_meta_from_row(&r)))
    }

    /// Get total usage stats for a session.
    pub async fn session_usage(&self, session_id: &str) -> Result<UsageSummary> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(input_tokens), 0) as ti, COALESCE(SUM(output_tokens), 0) as to_,
                    COALESCE(SUM(cache_read), 0) as cr, COALESCE(SUM(cache_write), 0) as cw,
                    COALESCE(SUM(cost_usd), 0.0) as cost, COUNT(*) as turns
             FROM usage_stats WHERE session_id = ?1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Usage query failed: {}", e)))?;

        Ok(UsageSummary {
            total_input_tokens: row.get::<i64, _>("ti") as u64,
            total_output_tokens: row.get::<i64, _>("to_") as u64,
            total_cache_read: row.get::<i64, _>("cr") as u64,
            total_cache_write: row.get::<i64, _>("cw") as u64,
            total_cost_usd: row.get::<f64, _>("cost"),
            total_turns: row.get::<i64, _>("turns") as u32,
        })
    }

    /// Record a compaction event for a session.
    pub async fn record_compaction(
        &self,
        session_id: &str,
        trigger: &str,
        pre_tokens: u64,
        summary: &str,
        messages_compacted: usize,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO compaction_log (session_id, timestamp, trigger, pre_tokens, summary, messages_compacted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(session_id)
        .bind(&now)
        .bind(trigger)
        .bind(pre_tokens as i64)
        .bind(summary)
        .bind(messages_compacted as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Record compaction failed: {}", e)))?;

        debug!(session_id = %session_id, pre_tokens, messages_compacted, "Recorded compaction event");
        Ok(())
    }

    /// Save a message to a session.
    ///
    /// When the first "user" message is saved, the session title is automatically
    /// set to the message text (truncated to 100 chars).
    pub async fn save_message(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO session_messages (session_id, role, content, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(session_id)
        .bind(role)
        .bind(content)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Save message failed: {}", e)))?;

        // Auto-set title from first user message
        if role == "user" {
            let title = if content.len() > 100 {
                format!("{}...", &content[..100])
            } else {
                content.to_string()
            };
            // Only set if title is currently NULL (first message)
            sqlx::query(
                "UPDATE session_metadata SET title = ?2 WHERE id = ?1 AND title IS NULL",
            )
            .bind(session_id)
            .bind(&title)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Set title failed: {}", e)))?;
        }

        Ok(())
    }

    /// Get all messages for a session, ordered by ID.
    pub async fn get_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>> {
        let rows = sqlx::query(
            "SELECT id, session_id, role, content, timestamp
             FROM session_messages
             WHERE session_id = ?1
             ORDER BY id ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Get messages failed: {}", e)))?;

        Ok(rows
            .iter()
            .map(|r| SessionMessage {
                id: r.get("id"),
                session_id: r.get("session_id"),
                role: r.get("role"),
                content: r.get("content"),
                timestamp: r.get("timestamp"),
            })
            .collect())
    }
}

/// Extract a SessionMeta from a database row.
fn session_meta_from_row(row: &sqlx::sqlite::SqliteRow) -> SessionMeta {
    SessionMeta {
        id: row.get("id"),
        created_at: row.get("created_at"),
        last_message_at: row.get("last_message_at"),
        is_closed: row.get::<i64, _>("is_closed") != 0,
        summary: row.get("summary"),
        title: row.get("title"),
        message_count: row.get("message_count"),
        channel: row.get("channel"),
        channel_session_key: row.get("channel_session_key"),
    }
}

/// Aggregated usage summary for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read: u64,
    pub total_cache_write: u64,
    pub total_cost_usd: f64,
    pub total_turns: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup() -> (TempDir, SessionManager) {
        let tmp = TempDir::new().unwrap();
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let mgr = SessionManager::from_pool(pool, &tmp.path().join("sessions"))
            .await
            .unwrap();
        (tmp, mgr)
    }

    #[tokio::test]
    async fn test_create_and_get_session() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "test-key").await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert_eq!(session.id, id);
        assert!(!session.is_closed);
        assert_eq!(session.message_count, 0);
        assert_eq!(session.channel, "main");
        assert_eq!(session.channel_session_key.as_deref(), Some("test-key"));
    }

    #[tokio::test]
    async fn test_close_session() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "test-key").await.unwrap();

        mgr.close_session(&id, "Discussed Rust memory management").await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert!(session.is_closed);
        assert_eq!(session.summary.as_deref(), Some("Discussed Rust memory management"));
    }

    #[tokio::test]
    async fn test_touch_session() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "test-key").await.unwrap();

        mgr.touch_session(&id).await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert_eq!(session.message_count, 2);
    }

    #[tokio::test]
    async fn test_resolve_session_new_when_empty() {
        let (_tmp, mgr) = setup().await;

        match mgr.resolve_session(&Channel::Main, "some-key", None).await.unwrap() {
            SessionDecision::New => {} // expected
            SessionDecision::Continue(_) => panic!("Should be New when no sessions exist"),
        }
    }

    #[tokio::test]
    async fn test_resolve_session_continue_recent() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "key-1").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        match mgr.resolve_session(&Channel::Main, "key-1", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New => panic!("Should continue recent session"),
        }
    }

    #[tokio::test]
    async fn test_resolve_session_new_when_closed() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "key-1").await.unwrap();
        mgr.touch_session(&id).await.unwrap();
        mgr.close_session(&id, "done").await.unwrap();

        match mgr.resolve_session(&Channel::Main, "key-1", None).await.unwrap() {
            SessionDecision::New => {} // expected
            SessionDecision::Continue(_) => panic!("Should not continue closed session"),
        }
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let (_tmp, mgr) = setup().await;
        mgr.create_session(&Channel::Main, "k1").await.unwrap();
        mgr.create_session(&Channel::Main, "k2").await.unwrap();
        mgr.create_session(&Channel::Telegram, "chat-1").await.unwrap();

        let sessions = mgr.list_sessions(10).await.unwrap();
        assert_eq!(sessions.len(), 3);
    }

    #[tokio::test]
    async fn test_record_and_query_usage() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "test-key").await.unwrap();

        mgr.record_usage(
            &id,
            &UsageRecord {
                input_tokens: 1000,
                output_tokens: 500,
                cache_read: 200,
                cache_write: 100,
                cost_usd: 0.01,
                model: "claude-sonnet".into(),
            },
            1,
        )
        .await
        .unwrap();

        mgr.record_usage(
            &id,
            &UsageRecord {
                input_tokens: 800,
                output_tokens: 400,
                cache_read: 150,
                cache_write: 50,
                cost_usd: 0.008,
                model: "claude-sonnet".into(),
            },
            2,
        )
        .await
        .unwrap();

        let summary = mgr.session_usage(&id).await.unwrap();
        assert_eq!(summary.total_input_tokens, 1800);
        assert_eq!(summary.total_output_tokens, 900);
        assert_eq!(summary.total_turns, 2);
        assert!((summary.total_cost_usd - 0.018).abs() < 0.001);
    }

    // --- New channel-specific tests ---

    #[tokio::test]
    async fn test_main_explicit_sessions() {
        let (_tmp, mgr) = setup().await;

        // Create session for key "abc"
        let id = mgr.create_session(&Channel::Main, "abc").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        // Same key → continue
        match mgr.resolve_session(&Channel::Main, "abc", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New => panic!("Should continue with same key"),
        }

        // Different key → new
        match mgr.resolve_session(&Channel::Main, "xyz", None).await.unwrap() {
            SessionDecision::New => {} // expected
            SessionDecision::Continue(_) => panic!("Different key should get new session"),
        }
    }

    #[tokio::test]
    async fn test_telegram_time_gap() {
        let (_tmp, mgr) = setup().await;

        // Create a telegram session
        let id = mgr.create_session(&Channel::Telegram, "chat-123").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        // Within 6h → continue
        match mgr.resolve_session(&Channel::Telegram, "chat-123", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New => panic!("Should continue within gap"),
        }

        // Manually set last_message_at to 7h ago to simulate inactivity
        let old_time = (Utc::now() - Duration::hours(7)).to_rfc3339();
        sqlx::query("UPDATE session_metadata SET last_message_at = ?1 WHERE id = ?2")
            .bind(&old_time)
            .bind(&id)
            .execute(&mgr.pool)
            .await
            .unwrap();

        // Beyond 6h → new (old session auto-closed)
        match mgr.resolve_session(&Channel::Telegram, "chat-123", None).await.unwrap() {
            SessionDecision::New => {} // expected
            SessionDecision::Continue(_) => panic!("Should start new session after 7h gap"),
        }

        // Verify old session was auto-closed
        let old = mgr.get_session(&id).await.unwrap().unwrap();
        assert!(old.is_closed);
        assert_eq!(old.summary.as_deref(), Some("Auto-closed: inactivity"));
    }

    #[tokio::test]
    async fn test_record_compaction() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "test-key").await.unwrap();

        mgr.record_compaction(&id, "auto", 150_000, "Summary of old messages", 12)
            .await
            .unwrap();

        // Verify via raw query
        let row = sqlx::query(
            "SELECT trigger, pre_tokens, summary, messages_compacted FROM compaction_log WHERE session_id = ?1",
        )
        .bind(&id)
        .fetch_one(&mgr.pool)
        .await
        .unwrap();

        assert_eq!(row.get::<String, _>("trigger"), "auto");
        assert_eq!(row.get::<i64, _>("pre_tokens"), 150_000);
        assert_eq!(row.get::<String, _>("summary"), "Summary of old messages");
        assert_eq!(row.get::<i64, _>("messages_compacted"), 12);
    }

    #[tokio::test]
    async fn test_channel_isolation() {
        let (_tmp, mgr) = setup().await;

        // Create sessions with same key on different channels
        let main_id = mgr.create_session(&Channel::Main, "shared-key").await.unwrap();
        let tg_id = mgr.create_session(&Channel::Telegram, "shared-key").await.unwrap();
        mgr.touch_session(&main_id).await.unwrap();
        mgr.touch_session(&tg_id).await.unwrap();

        // Each channel resolves to its own session
        match mgr.resolve_session(&Channel::Main, "shared-key", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, main_id),
            SessionDecision::New => panic!("Main should find its session"),
        }
        match mgr.resolve_session(&Channel::Telegram, "shared-key", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, tg_id),
            SessionDecision::New => panic!("Telegram should find its session"),
        }
    }
}
