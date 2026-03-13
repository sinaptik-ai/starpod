mod schema;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use orion_core::{OrionError, Result};
use rusqlite::Connection;

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
    conn: Mutex<Connection>,
    #[allow(dead_code)] // used in future phases for JSONL transcript storage
    sessions_dir: PathBuf,
}

/// Time-gap threshold: gaps shorter than this continue the session.
const SHORT_GAP_MINUTES: i64 = 30;

impl SessionManager {
    /// Create a new SessionManager.
    pub fn new(db_path: &Path, sessions_dir: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir_all(sessions_dir)?;

        let conn = Connection::open(db_path)
            .map_err(|e| OrionError::Database(format!("Failed to open session db: {}", e)))?;

        schema::migrate(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            sessions_dir: sessions_dir.to_path_buf(),
        })
    }

    /// Lock the database connection.
    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("session db mutex poisoned")
    }

    /// Decide whether to continue the most recent session or start a new one.
    ///
    /// Time-gap rules:
    /// - `< 30 min` since last message → continue
    /// - `>= 30 min` → new session (future: call Claude to decide for 30min-2h range)
    pub fn resolve_session(&self) -> Result<SessionDecision> {
        let latest = self.latest_open_session()?;

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
    pub fn create_session(&self) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.db()
            .execute(
                "INSERT INTO session_metadata (id, created_at, last_message_at, is_closed, message_count)
                 VALUES (?1, ?2, ?2, 0, 0)",
                rusqlite::params![id, now],
            )
            .map_err(|e| OrionError::Database(format!("Create session failed: {}", e)))?;

        debug!(session_id = %id, "Created new session");
        Ok(id)
    }

    /// Mark a session as closed with an optional summary.
    pub fn close_session(&self, id: &str, summary: &str) -> Result<()> {
        self.db()
            .execute(
                "UPDATE session_metadata SET is_closed = 1, summary = ?2 WHERE id = ?1",
                rusqlite::params![id, summary],
            )
            .map_err(|e| OrionError::Database(format!("Close session failed: {}", e)))?;

        debug!(session_id = %id, "Closed session");
        Ok(())
    }

    /// Update the last_message_at timestamp and increment message_count.
    pub fn touch_session(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db()
            .execute(
                "UPDATE session_metadata SET last_message_at = ?2, message_count = message_count + 1 WHERE id = ?1",
                rusqlite::params![id, now],
            )
            .map_err(|e| OrionError::Database(format!("Touch session failed: {}", e)))?;
        Ok(())
    }

    /// Record token usage for a turn.
    pub fn record_usage(&self, session_id: &str, usage: &UsageRecord, turn: u32) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db()
            .execute(
                "INSERT INTO usage_stats (session_id, turn, input_tokens, output_tokens, cache_read, cache_write, cost_usd, model, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    session_id,
                    turn as i64,
                    usage.input_tokens as i64,
                    usage.output_tokens as i64,
                    usage.cache_read as i64,
                    usage.cache_write as i64,
                    usage.cost_usd,
                    usage.model,
                    now,
                ],
            )
            .map_err(|e| OrionError::Database(format!("Record usage failed: {}", e)))?;

        Ok(())
    }

    /// List sessions, most recent first.
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionMeta>> {
        let conn = self.db();
        let mut stmt = conn
            .prepare(
                "SELECT id, created_at, last_message_at, is_closed, summary, message_count
                 FROM session_metadata
                 ORDER BY last_message_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| OrionError::Database(format!("Prepare failed: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(SessionMeta {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    last_message_at: row.get(2)?,
                    is_closed: row.get::<_, i64>(3)? != 0,
                    summary: row.get(4)?,
                    message_count: row.get(5)?,
                })
            })
            .map_err(|e| OrionError::Database(format!("Query failed: {}", e)))?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(
                row.map_err(|e| OrionError::Database(format!("Row read failed: {}", e)))?,
            );
        }
        Ok(sessions)
    }

    /// Get a specific session by ID.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        let result = self
            .db()
            .query_row(
                "SELECT id, created_at, last_message_at, is_closed, summary, message_count
                 FROM session_metadata WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(SessionMeta {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        last_message_at: row.get(2)?,
                        is_closed: row.get::<_, i64>(3)? != 0,
                        summary: row.get(4)?,
                        message_count: row.get(5)?,
                    })
                },
            );

        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(OrionError::Database(format!("Get session failed: {}", e))),
        }
    }

    /// Get total usage stats for a session.
    pub fn session_usage(&self, session_id: &str) -> Result<UsageSummary> {
        let result = self.db().query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_read), 0), COALESCE(SUM(cache_write), 0),
                    COALESCE(SUM(cost_usd), 0.0), COUNT(*)
             FROM usage_stats WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok(UsageSummary {
                    total_input_tokens: row.get::<_, i64>(0)? as u64,
                    total_output_tokens: row.get::<_, i64>(1)? as u64,
                    total_cache_read: row.get::<_, i64>(2)? as u64,
                    total_cache_write: row.get::<_, i64>(3)? as u64,
                    total_cost_usd: row.get(4)?,
                    total_turns: row.get::<_, i64>(5)? as u32,
                })
            },
        ).map_err(|e| OrionError::Database(format!("Usage query failed: {}", e)))?;

        Ok(result)
    }

    /// Find the most recent open (not closed) session.
    fn latest_open_session(&self) -> Result<Option<SessionMeta>> {
        let result = self.db().query_row(
            "SELECT id, created_at, last_message_at, is_closed, summary, message_count
             FROM session_metadata
             WHERE is_closed = 0
             ORDER BY last_message_at DESC
             LIMIT 1",
            [],
            |row| {
                Ok(SessionMeta {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    last_message_at: row.get(2)?,
                    is_closed: false,
                    summary: row.get(4)?,
                    message_count: row.get(5)?,
                })
            },
        );

        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(OrionError::Database(format!("Latest session query failed: {}", e))),
        }
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

    fn setup() -> (TempDir, SessionManager) {
        let tmp = TempDir::new().unwrap();
        let mgr = SessionManager::new(
            &tmp.path().join("session.db"),
            &tmp.path().join("sessions"),
        )
        .unwrap();
        (tmp, mgr)
    }

    #[test]
    fn test_create_and_get_session() {
        let (_tmp, mgr) = setup();
        let id = mgr.create_session().unwrap();

        let session = mgr.get_session(&id).unwrap().unwrap();
        assert_eq!(session.id, id);
        assert!(!session.is_closed);
        assert_eq!(session.message_count, 0);
    }

    #[test]
    fn test_close_session() {
        let (_tmp, mgr) = setup();
        let id = mgr.create_session().unwrap();

        mgr.close_session(&id, "Discussed Rust memory management").unwrap();

        let session = mgr.get_session(&id).unwrap().unwrap();
        assert!(session.is_closed);
        assert_eq!(session.summary.as_deref(), Some("Discussed Rust memory management"));
    }

    #[test]
    fn test_touch_session() {
        let (_tmp, mgr) = setup();
        let id = mgr.create_session().unwrap();

        mgr.touch_session(&id).unwrap();
        mgr.touch_session(&id).unwrap();

        let session = mgr.get_session(&id).unwrap().unwrap();
        assert_eq!(session.message_count, 2);
    }

    #[test]
    fn test_resolve_session_new_when_empty() {
        let (_tmp, mgr) = setup();

        match mgr.resolve_session().unwrap() {
            SessionDecision::New => {} // expected
            SessionDecision::Continue(_) => panic!("Should be New when no sessions exist"),
        }
    }

    #[test]
    fn test_resolve_session_continue_recent() {
        let (_tmp, mgr) = setup();
        let id = mgr.create_session().unwrap();
        mgr.touch_session(&id).unwrap();

        match mgr.resolve_session().unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New => panic!("Should continue recent session"),
        }
    }

    #[test]
    fn test_resolve_session_new_when_closed() {
        let (_tmp, mgr) = setup();
        let id = mgr.create_session().unwrap();
        mgr.touch_session(&id).unwrap();
        mgr.close_session(&id, "done").unwrap();

        match mgr.resolve_session().unwrap() {
            SessionDecision::New => {} // expected
            SessionDecision::Continue(_) => panic!("Should not continue closed session"),
        }
    }

    #[test]
    fn test_list_sessions() {
        let (_tmp, mgr) = setup();
        mgr.create_session().unwrap();
        mgr.create_session().unwrap();
        mgr.create_session().unwrap();

        let sessions = mgr.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 3);
    }

    #[test]
    fn test_record_and_query_usage() {
        let (_tmp, mgr) = setup();
        let id = mgr.create_session().unwrap();

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
        .unwrap();

        let summary = mgr.session_usage(&id).unwrap();
        assert_eq!(summary.total_input_tokens, 1800);
        assert_eq!(summary.total_output_tokens, 900);
        assert_eq!(summary.total_turns, 2);
        assert!((summary.total_cost_usd - 0.018).abs() < 0.001);
    }
}
