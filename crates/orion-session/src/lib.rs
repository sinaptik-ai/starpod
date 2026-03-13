mod schema;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::debug;
use uuid::Uuid;

use orion_core::{OrionError, Result};

/// Decision from time-gap analysis on whether to continue or start a new session.
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
    pub message_count: i64,
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

/// Manages session lifecycle — creation, time-gap resolution, closure, and usage tracking.
pub struct SessionManager {
    pool: SqlitePool,
    #[allow(dead_code)] // used in future phases for JSONL transcript storage
    sessions_dir: PathBuf,
}

/// Time-gap threshold: gaps shorter than this continue the session.
const SHORT_GAP_MINUTES: i64 = 30;

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
        .map_err(|e| OrionError::Database(format!("Invalid DB path: {}", e)))?;

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(|e| OrionError::Database(format!("Failed to open session db: {}", e)))?;

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

    /// Decide whether to continue the most recent session or start a new one.
    ///
    /// Time-gap rules:
    /// - `< 30 min` since last message → continue
    /// - `>= 30 min` → new session (future: call Claude to decide for 30min-2h range)
    pub async fn resolve_session(&self) -> Result<SessionDecision> {
        let latest = self.latest_open_session().await?;

        let session = match latest {
            Some(s) => s,
            None => return Ok(SessionDecision::New),
        };

        let last_msg = DateTime::parse_from_rfc3339(&session.last_message_at)
            .map_err(|e| OrionError::Session(format!("Bad timestamp: {}", e)))?
            .with_timezone(&Utc);

        let gap = Utc::now() - last_msg;

        if gap < Duration::minutes(SHORT_GAP_MINUTES) {
            debug!(session_id = %session.id, gap_mins = gap.num_minutes(), "Continuing session (short gap)");
            Ok(SessionDecision::Continue(session.id))
        } else {
            debug!(gap_mins = gap.num_minutes(), "Starting new session (gap too large)");
            Ok(SessionDecision::New)
        }
    }

    /// Create a new session and return its ID.
    pub async fn create_session(&self) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO session_metadata (id, created_at, last_message_at, is_closed, message_count)
             VALUES (?1, ?2, ?2, 0, 0)",
        )
        .bind(&id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| OrionError::Database(format!("Create session failed: {}", e)))?;

        debug!(session_id = %id, "Created new session");
        Ok(id)
    }

    /// Mark a session as closed with an optional summary.
    pub async fn close_session(&self, id: &str, summary: &str) -> Result<()> {
        sqlx::query("UPDATE session_metadata SET is_closed = 1, summary = ?2 WHERE id = ?1")
            .bind(id)
            .bind(summary)
            .execute(&self.pool)
            .await
            .map_err(|e| OrionError::Database(format!("Close session failed: {}", e)))?;

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
        .map_err(|e| OrionError::Database(format!("Touch session failed: {}", e)))?;
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
        .map_err(|e| OrionError::Database(format!("Record usage failed: {}", e)))?;

        Ok(())
    }

    /// List sessions, most recent first.
    pub async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionMeta>> {
        let rows = sqlx::query(
            "SELECT id, created_at, last_message_at, is_closed, summary, message_count
             FROM session_metadata
             ORDER BY last_message_at DESC
             LIMIT ?1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| OrionError::Database(format!("Query failed: {}", e)))?;

        let sessions: Vec<SessionMeta> = rows
            .iter()
            .map(|row| SessionMeta {
                id: row.get("id"),
                created_at: row.get("created_at"),
                last_message_at: row.get("last_message_at"),
                is_closed: row.get::<i64, _>("is_closed") != 0,
                summary: row.get("summary"),
                message_count: row.get("message_count"),
            })
            .collect();

        Ok(sessions)
    }

    /// Get a specific session by ID.
    pub async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        let row = sqlx::query(
            "SELECT id, created_at, last_message_at, is_closed, summary, message_count
             FROM session_metadata WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| OrionError::Database(format!("Get session failed: {}", e)))?;

        Ok(row.map(|r| SessionMeta {
            id: r.get("id"),
            created_at: r.get("created_at"),
            last_message_at: r.get("last_message_at"),
            is_closed: r.get::<i64, _>("is_closed") != 0,
            summary: r.get("summary"),
            message_count: r.get("message_count"),
        }))
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
        .map_err(|e| OrionError::Database(format!("Usage query failed: {}", e)))?;

        Ok(UsageSummary {
            total_input_tokens: row.get::<i64, _>("ti") as u64,
            total_output_tokens: row.get::<i64, _>("to_") as u64,
            total_cache_read: row.get::<i64, _>("cr") as u64,
            total_cache_write: row.get::<i64, _>("cw") as u64,
            total_cost_usd: row.get::<f64, _>("cost"),
            total_turns: row.get::<i64, _>("turns") as u32,
        })
    }

    /// Find the most recent open (not closed) session.
    async fn latest_open_session(&self) -> Result<Option<SessionMeta>> {
        let row = sqlx::query(
            "SELECT id, created_at, last_message_at, is_closed, summary, message_count
             FROM session_metadata
             WHERE is_closed = 0
             ORDER BY last_message_at DESC
             LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| OrionError::Database(format!("Latest session query failed: {}", e)))?;

        Ok(row.map(|r| SessionMeta {
            id: r.get("id"),
            created_at: r.get("created_at"),
            last_message_at: r.get("last_message_at"),
            is_closed: false,
            summary: r.get("summary"),
            message_count: r.get("message_count"),
        }))
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
        let id = mgr.create_session().await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert_eq!(session.id, id);
        assert!(!session.is_closed);
        assert_eq!(session.message_count, 0);
    }

    #[tokio::test]
    async fn test_close_session() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session().await.unwrap();

        mgr.close_session(&id, "Discussed Rust memory management").await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert!(session.is_closed);
        assert_eq!(session.summary.as_deref(), Some("Discussed Rust memory management"));
    }

    #[tokio::test]
    async fn test_touch_session() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session().await.unwrap();

        mgr.touch_session(&id).await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert_eq!(session.message_count, 2);
    }

    #[tokio::test]
    async fn test_resolve_session_new_when_empty() {
        let (_tmp, mgr) = setup().await;

        match mgr.resolve_session().await.unwrap() {
            SessionDecision::New => {} // expected
            SessionDecision::Continue(_) => panic!("Should be New when no sessions exist"),
        }
    }

    #[tokio::test]
    async fn test_resolve_session_continue_recent() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session().await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        match mgr.resolve_session().await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New => panic!("Should continue recent session"),
        }
    }

    #[tokio::test]
    async fn test_resolve_session_new_when_closed() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session().await.unwrap();
        mgr.touch_session(&id).await.unwrap();
        mgr.close_session(&id, "done").await.unwrap();

        match mgr.resolve_session().await.unwrap() {
            SessionDecision::New => {} // expected
            SessionDecision::Continue(_) => panic!("Should not continue closed session"),
        }
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let (_tmp, mgr) = setup().await;
        mgr.create_session().await.unwrap();
        mgr.create_session().await.unwrap();
        mgr.create_session().await.unwrap();

        let sessions = mgr.list_sessions(10).await.unwrap();
        assert_eq!(sessions.len(), 3);
    }

    #[tokio::test]
    async fn test_record_and_query_usage() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session().await.unwrap();

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
}
