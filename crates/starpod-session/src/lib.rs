mod schema;

use std::path::Path;
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
    /// Time-gap sessions via email — new session after inactivity threshold (24h).
    Email,
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Main => "main",
            Channel::Telegram => "telegram",
            Channel::Email => "email",
        }
    }

    pub fn from_channel_str(s: &str) -> Self {
        match s {
            "telegram" => Channel::Telegram,
            "email" => Channel::Email,
            _ => Channel::Main,
        }
    }

}

/// Decision from session resolution on whether to continue or start a new session.
#[derive(Debug, Clone)]
pub enum SessionDecision {
    /// Continue an existing session (contains session ID).
    Continue(String),
    /// Start a new session. If a previous session was auto-closed (e.g. time-gap),
    /// `closed_session_id` carries its ID so callers can export it.
    New { closed_session_id: Option<String> },
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
    pub user_id: String,
    pub is_read: bool,
    /// Cron job name or `"__heartbeat__"` if this session was triggered by a scheduled job.
    /// `None` for regular user sessions.
    pub triggered_by: Option<String>,
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
    pub user_id: String,
}

/// Manages session lifecycle — creation, channel-aware resolution, closure, and usage tracking.
pub struct SessionManager {
    pool: SqlitePool,
}

impl SessionManager {
    /// Create a new SessionManager.
    pub async fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

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

        Ok(Self { pool })
    }

    /// Create a SessionManager from an existing pool (for testing).
    #[cfg(test)]
    async fn from_pool(pool: SqlitePool) -> Result<Self> {
        schema::run_migrations(&pool).await?;
        Ok(Self { pool })
    }

    /// Resolve session for a given channel and key.
    ///
    /// - **Main** (explicit): always continues the matching open session if one exists.
    /// - **Telegram** (time-gap): continues if last message was within the gap threshold,
    ///   otherwise auto-closes the old session and returns `New`.
    ///
    /// `gap_minutes` is the inactivity gap from config. Pass `None` for explicit
    /// channels that don't use time-gap sessions.
    pub async fn resolve_session(
        &self,
        channel: &Channel,
        key: &str,
        gap_minutes: Option<i64>,
    ) -> Result<SessionDecision> {
        self.resolve_session_for_user(channel, key, gap_minutes, "admin").await
    }

    /// Resolve session for a given channel, key, and user.
    pub async fn resolve_session_for_user(
        &self,
        channel: &Channel,
        key: &str,
        gap_minutes: Option<i64>,
        user_id: &str,
    ) -> Result<SessionDecision> {
        let row = sqlx::query(
            "SELECT id, last_message_at
             FROM session_metadata
             WHERE channel = ?1 AND channel_session_key = ?2 AND is_closed = 0 AND user_id = ?3
             ORDER BY last_message_at DESC
             LIMIT 1",
        )
        .bind(channel.as_str())
        .bind(key)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Resolve session query failed: {}", e)))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(SessionDecision::New { closed_session_id: None }),
        };

        let session_id: String = row.get("id");

        // For explicit channels (no gap), always continue.
        let gap_threshold = match gap_minutes {
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
            Ok(SessionDecision::New { closed_session_id: Some(session_id) })
        }
    }

    /// Create a new session for a channel and key, returning its ID.
    pub async fn create_session(
        &self,
        channel: &Channel,
        key: &str,
    ) -> Result<String> {
        self.create_session_full(channel, key, "admin", None).await
    }

    /// Create a new session for a channel, key, and user, returning its ID.
    pub async fn create_session_for_user(
        &self,
        channel: &Channel,
        key: &str,
        user_id: &str,
    ) -> Result<String> {
        self.create_session_full(channel, key, user_id, None).await
    }

    /// Create a new session with full metadata, including an optional trigger source.
    ///
    /// `triggered_by` records the cron job name (e.g. `"daily-digest"`) or
    /// `"__heartbeat__"` when the session is created by the scheduler.
    pub async fn create_session_full(
        &self,
        channel: &Channel,
        key: &str,
        user_id: &str,
        triggered_by: Option<&str>,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO session_metadata (id, created_at, last_message_at, is_closed, message_count, channel, channel_session_key, user_id, triggered_by)
             VALUES (?1, ?2, ?2, 0, 0, ?3, ?4, ?5, ?6)",
        )
        .bind(&id)
        .bind(&now)
        .bind(channel.as_str())
        .bind(key)
        .bind(user_id)
        .bind(triggered_by)
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

    /// Mark a session as read or unread.
    pub async fn mark_read(&self, id: &str, is_read: bool) -> Result<()> {
        sqlx::query("UPDATE session_metadata SET is_read = ?2 WHERE id = ?1")
            .bind(id)
            .bind(is_read as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Mark read failed: {}", e)))?;
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
            let mut end = 100;
            while end > 0 && !title.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &title[..end])
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
            "INSERT INTO usage_stats (session_id, turn, input_tokens, output_tokens, cache_read, cache_write, cost_usd, model, user_id, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(session_id)
        .bind(turn as i64)
        .bind(usage.input_tokens as i64)
        .bind(usage.output_tokens as i64)
        .bind(usage.cache_read as i64)
        .bind(usage.cache_write as i64)
        .bind(usage.cost_usd)
        .bind(&usage.model)
        .bind(&usage.user_id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Record usage failed: {}", e)))?;

        Ok(())
    }

    /// List sessions, most recent first.
    pub async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionMeta>> {
        let rows = sqlx::query(
            "SELECT id, created_at, last_message_at, is_closed, summary, title, message_count, channel, channel_session_key, user_id, is_read, triggered_by
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
            "SELECT id, created_at, last_message_at, is_closed, summary, title, message_count, channel, channel_session_key, user_id, is_read, triggered_by
             FROM session_metadata WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Get session failed: {}", e)))?;

        Ok(row.map(|r| session_meta_from_row(&r)))
    }

    /// Get total usage stats for a session.
    ///
    /// `total_input_tokens` includes uncached, cache-read, and cache-write
    /// tokens so the caller gets the true context size. Cache breakdown is
    /// available via `total_cache_read` / `total_cache_write`.
    pub async fn session_usage(&self, session_id: &str) -> Result<UsageSummary> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(input_tokens + cache_read + cache_write), 0) as ti, COALESCE(SUM(output_tokens), 0) as to_,
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

    /// Get a full cost overview with breakdowns by user and model.
    ///
    /// If `since` is provided (RFC 3339 timestamp), only usage after that time is included.
    pub async fn cost_overview(&self, since: Option<&str>) -> Result<CostOverview> {
        let (where_clause, bind_val) = match since {
            Some(ts) => ("WHERE timestamp >= ?1", Some(ts)),
            None => ("", None),
        };

        // Total
        let total_sql = format!(
            "SELECT COALESCE(SUM(cost_usd), 0.0) as cost,
                    COALESCE(SUM(input_tokens + cache_read + cache_write), 0) as ti,
                    COALESCE(SUM(output_tokens), 0) as to_,
                    COALESCE(SUM(cache_read), 0) as cr,
                    COALESCE(SUM(cache_write), 0) as cw,
                    COUNT(*) as turns
             FROM usage_stats {}",
            where_clause
        );
        let mut q = sqlx::query(&total_sql);
        if let Some(ts) = bind_val {
            q = q.bind(ts);
        }
        let total_row = q.fetch_one(&self.pool).await
            .map_err(|e| StarpodError::Database(format!("Cost total query failed: {}", e)))?;

        // By user
        let user_sql = format!(
            "SELECT user_id,
                    COALESCE(SUM(cost_usd), 0.0) as cost,
                    COALESCE(SUM(input_tokens + cache_read + cache_write), 0) as ti,
                    COALESCE(SUM(output_tokens), 0) as to_,
                    COALESCE(SUM(cache_read), 0) as cr,
                    COALESCE(SUM(cache_write), 0) as cw,
                    COUNT(*) as turns
             FROM usage_stats {} GROUP BY user_id ORDER BY cost DESC",
            where_clause
        );
        let mut q = sqlx::query(&user_sql);
        if let Some(ts) = bind_val {
            q = q.bind(ts);
        }
        let user_rows = q.fetch_all(&self.pool).await
            .map_err(|e| StarpodError::Database(format!("Cost by-user query failed: {}", e)))?;

        // By model
        let model_sql = format!(
            "SELECT model,
                    COALESCE(SUM(cost_usd), 0.0) as cost,
                    COALESCE(SUM(input_tokens + cache_read + cache_write), 0) as ti,
                    COALESCE(SUM(output_tokens), 0) as to_,
                    COALESCE(SUM(cache_read), 0) as cr,
                    COALESCE(SUM(cache_write), 0) as cw,
                    COUNT(*) as turns
             FROM usage_stats {} GROUP BY model ORDER BY cost DESC",
            where_clause
        );
        let mut q = sqlx::query(&model_sql);
        if let Some(ts) = bind_val {
            q = q.bind(ts);
        }
        let model_rows = q.fetch_all(&self.pool).await
            .map_err(|e| StarpodError::Database(format!("Cost by-model query failed: {}", e)))?;

        // By day + model
        let day_sql = format!(
            "SELECT DATE(timestamp) as day, COALESCE(model, 'unknown') as model,
                    COALESCE(SUM(cost_usd), 0.0) as cost
             FROM usage_stats {} GROUP BY day, model ORDER BY day ASC",
            where_clause
        );
        let mut q = sqlx::query(&day_sql);
        if let Some(ts) = bind_val {
            q = q.bind(ts);
        }
        let day_rows = q.fetch_all(&self.pool).await
            .map_err(|e| StarpodError::Database(format!("Cost by-day query failed: {}", e)))?;

        // Group day rows into DayCostSummary
        let mut by_day: Vec<DayCostSummary> = Vec::new();
        for row in &day_rows {
            let date: String = row.get("day");
            let model: String = row.get("model");
            let cost: f64 = row.get::<f64, _>("cost");
            if let Some(last) = by_day.last_mut().filter(|d| d.date == date) {
                last.total_cost_usd += cost;
                last.by_model.push(DayModelCost { model, cost_usd: cost });
            } else {
                by_day.push(DayCostSummary {
                    date,
                    total_cost_usd: cost,
                    by_model: vec![DayModelCost { model, cost_usd: cost }],
                });
            }
        }

        // By tool (from session_messages)
        let tool_sql = format!(
            "SELECT json_extract(sm.content, '$.name') AS tool_name,
                    COUNT(*) AS invocations,
                    COALESCE(SUM(
                      CASE WHEN tr.content IS NOT NULL
                           AND json_extract(tr.content, '$.is_error') = 1
                      THEN 1 ELSE 0 END
                    ), 0) AS errors
             FROM session_messages sm
             LEFT JOIN session_messages tr
               ON tr.session_id = sm.session_id
               AND tr.role = 'tool_result'
               AND json_extract(tr.content, '$.tool_use_id') = json_extract(sm.content, '$.id')
             WHERE sm.role = 'tool_use'
               {}
             GROUP BY tool_name
             ORDER BY invocations DESC",
            if bind_val.is_some() { "AND sm.timestamp >= ?1" } else { "" }
        );
        let mut q = sqlx::query(&tool_sql);
        if let Some(ts) = bind_val {
            q = q.bind(ts);
        }
        let tool_rows = q.fetch_all(&self.pool).await
            .map_err(|e| StarpodError::Database(format!("Cost by-tool query failed: {}", e)))?;

        let by_tool: Vec<ToolUsageSummary> = tool_rows.iter().map(|r| ToolUsageSummary {
            tool_name: r.try_get("tool_name").unwrap_or_else(|_| "unknown".to_string()),
            invocations: r.get::<i64, _>("invocations") as u32,
            errors: r.get::<i64, _>("errors") as u32,
        }).collect();

        Ok(CostOverview {
            total_cost_usd: total_row.get::<f64, _>("cost"),
            total_input_tokens: total_row.get::<i64, _>("ti") as u64,
            total_output_tokens: total_row.get::<i64, _>("to_") as u64,
            total_cache_read: total_row.get::<i64, _>("cr") as u64,
            total_cache_write: total_row.get::<i64, _>("cw") as u64,
            total_turns: total_row.get::<i64, _>("turns") as u32,
            by_user: user_rows.iter().map(|r| UserCostSummary {
                user_id: r.get("user_id"),
                total_cost_usd: r.get::<f64, _>("cost"),
                total_input_tokens: r.get::<i64, _>("ti") as u64,
                total_output_tokens: r.get::<i64, _>("to_") as u64,
                total_cache_read: r.get::<i64, _>("cr") as u64,
                total_cache_write: r.get::<i64, _>("cw") as u64,
                total_turns: r.get::<i64, _>("turns") as u32,
            }).collect(),
            by_model: model_rows.iter().map(|r| ModelCostSummary {
                model: r.try_get("model").unwrap_or_else(|_| "unknown".to_string()),
                total_cost_usd: r.get::<f64, _>("cost"),
                total_input_tokens: r.get::<i64, _>("ti") as u64,
                total_output_tokens: r.get::<i64, _>("to_") as u64,
                total_cache_read: r.get::<i64, _>("cr") as u64,
                total_cache_write: r.get::<i64, _>("cw") as u64,
                total_turns: r.get::<i64, _>("turns") as u32,
            }).collect(),
            by_day,
            by_tool,
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
                let mut end = 100;
                while end > 0 && !content.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &content[..end])
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
        user_id: row.try_get("user_id").unwrap_or_else(|_| "admin".to_string()),
        is_read: row.try_get::<i64, _>("is_read").unwrap_or(1) != 0,
        triggered_by: row.try_get("triggered_by").unwrap_or(None),
    }
}

/// Aggregated usage summary for a session.
///
/// ## Token accounting
///
/// `total_input_tokens` is the **total** input context size across all turns,
/// i.e. `SUM(input_tokens + cache_read + cache_write)` from the per-turn
/// records. This is what the UI displays as "X in".
///
/// `total_cache_read` and `total_cache_write` are the cached subsets of
/// that total — useful for showing cache efficiency (e.g. "2.1k cached").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    /// Total input tokens (uncached + cache_read + cache_write).
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    /// Tokens served from prompt cache.
    pub total_cache_read: u64,
    /// Tokens written to prompt cache.
    pub total_cache_write: u64,
    pub total_cost_usd: f64,
    pub total_turns: u32,
}

/// Cost summary per user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCostSummary {
    pub user_id: String,
    pub total_cost_usd: f64,
    /// Total input tokens (uncached + cache_read + cache_write).
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read: u64,
    pub total_cache_write: u64,
    pub total_turns: u32,
}

/// Cost summary per model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostSummary {
    pub model: String,
    pub total_cost_usd: f64,
    /// Total input tokens (uncached + cache_read + cache_write).
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read: u64,
    pub total_cache_write: u64,
    pub total_turns: u32,
}

/// Cost summary for a single day, broken down by model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayCostSummary {
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Cost per model on this day.
    pub by_model: Vec<DayModelCost>,
    /// Total cost for this day.
    pub total_cost_usd: f64,
}

/// Cost for a single model on a single day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayModelCost {
    pub model: String,
    pub cost_usd: f64,
}

/// Aggregated tool invocation statistics, grouped by tool name.
///
/// Extracted from `session_messages` rows with `role = "tool_use"` and
/// `role = "tool_result"`.  The error count is derived by joining each
/// `tool_use` to its matching `tool_result` and checking the `is_error` flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUsageSummary {
    /// The tool name (e.g. `"MemorySearch"`, `"VaultGet"`).
    pub tool_name: String,
    /// Total number of times this tool was invoked.
    pub invocations: u32,
    /// How many of those invocations resulted in an error.
    pub errors: u32,
}

/// Full cost overview with breakdowns by user and model.
///
/// All `total_input_tokens` fields include cached tokens — see [`UsageSummary`]
/// for the full accounting explanation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostOverview {
    pub total_cost_usd: f64,
    /// Total input tokens (uncached + cache_read + cache_write).
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read: u64,
    pub total_cache_write: u64,
    pub total_turns: u32,
    pub by_user: Vec<UserCostSummary>,
    pub by_model: Vec<ModelCostSummary>,
    pub by_day: Vec<DayCostSummary>,
    /// Tool invocation counts, sorted by invocations descending.
    pub by_tool: Vec<ToolUsageSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup() -> (TempDir, SessionManager) {
        let tmp = TempDir::new().unwrap();
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let mgr = SessionManager::from_pool(pool)
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
            SessionDecision::New { .. } => {} // expected
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
            SessionDecision::New { .. } => panic!("Should continue recent session"),
        }
    }

    #[tokio::test]
    async fn test_resolve_session_new_when_closed() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "key-1").await.unwrap();
        mgr.touch_session(&id).await.unwrap();
        mgr.close_session(&id, "done").await.unwrap();

        match mgr.resolve_session(&Channel::Main, "key-1", None).await.unwrap() {
            SessionDecision::New { .. } => {} // expected
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
                user_id: "admin".into(),
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
                user_id: "admin".into(),
            },
            2,
        )
        .await
        .unwrap();

        let summary = mgr.session_usage(&id).await.unwrap();
        // total_input_tokens includes input_tokens + cache_read + cache_write
        // Turn 1: 1000 + 200 + 100 = 1300, Turn 2: 800 + 150 + 50 = 1000
        assert_eq!(summary.total_input_tokens, 2300);
        assert_eq!(summary.total_output_tokens, 900);
        assert_eq!(summary.total_turns, 2);
        assert!((summary.total_cost_usd - 0.018).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_usage_cache_breakdown() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "cache-test").await.unwrap();

        // Turn 1: cache miss — all tokens go to cache_write
        mgr.record_usage(&id, &UsageRecord {
            input_tokens: 500, output_tokens: 200, cache_read: 0, cache_write: 4000,
            cost_usd: 0.05, model: "claude-sonnet".into(), user_id: "admin".into(),
        }, 1).await.unwrap();

        // Turn 2: cache hit — most tokens served from cache
        mgr.record_usage(&id, &UsageRecord {
            input_tokens: 100, output_tokens: 300, cache_read: 4000, cache_write: 0,
            cost_usd: 0.01, model: "claude-sonnet".into(), user_id: "admin".into(),
        }, 2).await.unwrap();

        let summary = mgr.session_usage(&id).await.unwrap();

        // total_input_tokens = (500 + 0 + 4000) + (100 + 4000 + 0) = 8600
        assert_eq!(summary.total_input_tokens, 8600);
        assert_eq!(summary.total_output_tokens, 500);
        // Cache breakdown preserved separately
        assert_eq!(summary.total_cache_read, 4000);
        assert_eq!(summary.total_cache_write, 4000);
        assert_eq!(summary.total_turns, 2);
        assert!((summary.total_cost_usd - 0.06).abs() < 0.001);
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
            SessionDecision::New { .. } => panic!("Should continue with same key"),
        }

        // Different key → new
        match mgr.resolve_session(&Channel::Main, "xyz", None).await.unwrap() {
            SessionDecision::New { .. } => {} // expected
            SessionDecision::Continue(_) => panic!("Different key should get new session"),
        }
    }

    #[tokio::test]
    async fn test_telegram_time_gap() {
        let (_tmp, mgr) = setup().await;
        let gap = Some(360); // 6h, as configured via [channels.telegram] gap_minutes

        // Create a telegram session
        let id = mgr.create_session(&Channel::Telegram, "chat-123").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        // Within 6h → continue
        match mgr.resolve_session(&Channel::Telegram, "chat-123", gap).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New { .. } => panic!("Should continue within gap"),
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
        match mgr.resolve_session(&Channel::Telegram, "chat-123", gap).await.unwrap() {
            SessionDecision::New { .. } => {} // expected
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
    async fn test_telegram_custom_gap_override() {
        let (_tmp, mgr) = setup().await;

        // Create a Telegram session
        let id = mgr.create_session(&Channel::Telegram, "chat-gap").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        // Set last_message_at to 2 hours ago
        let two_hours_ago = (Utc::now() - Duration::hours(2)).to_rfc3339();
        sqlx::query("UPDATE session_metadata SET last_message_at = ?1 WHERE id = ?2")
            .bind(&two_hours_ago)
            .bind(&id)
            .execute(&mgr.pool)
            .await
            .unwrap();

        // gap_minutes=60 (1h) — 2h ago exceeds 1h → should be New
        match mgr.resolve_session(&Channel::Telegram, "chat-gap", Some(60)).await.unwrap() {
            SessionDecision::New { .. } => {} // expected
            SessionDecision::Continue(_) => panic!("Should start new session when 2h > 1h gap"),
        }

        // The old session was auto-closed, create a fresh one and backdate it again
        let id2 = mgr.create_session(&Channel::Telegram, "chat-gap").await.unwrap();
        mgr.touch_session(&id2).await.unwrap();
        let two_hours_ago = (Utc::now() - Duration::hours(2)).to_rfc3339();
        sqlx::query("UPDATE session_metadata SET last_message_at = ?1 WHERE id = ?2")
            .bind(&two_hours_ago)
            .bind(&id2)
            .execute(&mgr.pool)
            .await
            .unwrap();

        // gap_minutes=180 (3h) — 2h ago is within 3h → should Continue
        match mgr.resolve_session(&Channel::Telegram, "chat-gap", Some(180)).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id2),
            SessionDecision::New { .. } => panic!("Should continue session when 2h < 3h gap"),
        }
    }

    #[tokio::test]
    async fn test_main_channel_ignores_gap() {
        let (_tmp, mgr) = setup().await;

        // Create a Main session
        let id = mgr.create_session(&Channel::Main, "main-gap").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        // Without a gap_minutes override, Main channel always continues (explicit)
        match mgr.resolve_session(&Channel::Main, "main-gap", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New { .. } => panic!("Main channel should always continue without gap override"),
        }

        // Even backdating last_message_at to 24h ago, Main without gap override still continues
        let old = (Utc::now() - Duration::hours(24)).to_rfc3339();
        sqlx::query("UPDATE session_metadata SET last_message_at = ?1 WHERE id = ?2")
            .bind(&old)
            .bind(&id)
            .execute(&mgr.pool)
            .await
            .unwrap();

        match mgr.resolve_session(&Channel::Main, "main-gap", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New { .. } => panic!("Main channel should continue even with old last_message_at when gap_minutes is None"),
        }
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
            SessionDecision::New { .. } => panic!("Main should find its session"),
        }
        match mgr.resolve_session(&Channel::Telegram, "shared-key", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, tg_id),
            SessionDecision::New { .. } => panic!("Telegram should find its session"),
        }
    }

    #[tokio::test]
    async fn test_auto_close_returns_closed_session_id() {
        let (_tmp, mgr) = setup().await;
        let gap = Some(60); // 1h

        // Create and backdate a Telegram session
        let id = mgr.create_session(&Channel::Telegram, "export-test").await.unwrap();
        mgr.touch_session(&id).await.unwrap();
        mgr.save_message(&id, "user", "Hello!").await.unwrap();
        mgr.save_message(&id, "assistant", "Hi there!").await.unwrap();

        let two_hours_ago = (Utc::now() - Duration::hours(2)).to_rfc3339();
        sqlx::query("UPDATE session_metadata SET last_message_at = ?1 WHERE id = ?2")
            .bind(&two_hours_ago)
            .bind(&id)
            .execute(&mgr.pool)
            .await
            .unwrap();

        // Resolve should return New with the closed session's ID
        match mgr.resolve_session(&Channel::Telegram, "export-test", gap).await.unwrap() {
            SessionDecision::New { closed_session_id } => {
                assert_eq!(closed_session_id, Some(id.clone()), "Should return the closed session ID");
            }
            SessionDecision::Continue(_) => panic!("Should start new session after 2h > 1h gap"),
        }

        // First resolve with no prior session → New without closed ID
        match mgr.resolve_session(&Channel::Main, "fresh-key", None).await.unwrap() {
            SessionDecision::New { closed_session_id } => {
                assert!(closed_session_id.is_none(), "No prior session means no closed ID");
            }
            SessionDecision::Continue(_) => panic!("Should be new"),
        }
    }

    #[tokio::test]
    async fn test_auto_close_closed_id_is_correct_session() {
        let (_tmp, mgr) = setup().await;
        let gap = Some(60); // 1h

        // Create two Telegram sessions for different keys
        let id_a = mgr.create_session(&Channel::Telegram, "chat-a").await.unwrap();
        mgr.touch_session(&id_a).await.unwrap();
        mgr.save_message(&id_a, "user", "Message in chat A").await.unwrap();
        mgr.save_message(&id_a, "assistant", "Reply in chat A").await.unwrap();

        let id_b = mgr.create_session(&Channel::Telegram, "chat-b").await.unwrap();
        mgr.touch_session(&id_b).await.unwrap();
        mgr.save_message(&id_b, "user", "Message in chat B").await.unwrap();

        // Backdate only chat-a beyond the gap
        let old_time = (Utc::now() - Duration::hours(2)).to_rfc3339();
        sqlx::query("UPDATE session_metadata SET last_message_at = ?1 WHERE id = ?2")
            .bind(&old_time)
            .bind(&id_a)
            .execute(&mgr.pool)
            .await
            .unwrap();

        // Resolve chat-a → should auto-close and return its ID
        match mgr.resolve_session(&Channel::Telegram, "chat-a", gap).await.unwrap() {
            SessionDecision::New { closed_session_id } => {
                assert_eq!(
                    closed_session_id,
                    Some(id_a.clone()),
                    "closed_session_id must match the session that was auto-closed"
                );
            }
            SessionDecision::Continue(_) => panic!("Should start new session after gap"),
        }

        // Verify the closed session's messages are still accessible
        let messages = mgr.get_messages(&id_a).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "Message in chat A");
        assert_eq!(messages[1].content, "Reply in chat A");

        // Verify chat-b is unaffected (still open, still continuable)
        match mgr.resolve_session(&Channel::Telegram, "chat-b", gap).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id_b),
            SessionDecision::New { .. } => panic!("chat-b should still be continuable"),
        }
    }

    #[tokio::test]
    async fn test_no_closed_id_for_main_channel() {
        let (_tmp, mgr) = setup().await;

        // Create a Main session and backdate it far in the past
        let id = mgr.create_session(&Channel::Main, "main-key").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        let old_time = (Utc::now() - Duration::hours(48)).to_rfc3339();
        sqlx::query("UPDATE session_metadata SET last_message_at = ?1 WHERE id = ?2")
            .bind(&old_time)
            .bind(&id)
            .execute(&mgr.pool)
            .await
            .unwrap();

        // Main channel uses gap_minutes=None → never auto-closes
        match mgr.resolve_session(&Channel::Main, "main-key", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New { .. } => panic!("Main channel should never auto-close"),
        }

        // Even with a fresh key (no session), New should have closed_session_id=None
        match mgr.resolve_session(&Channel::Main, "new-main-key", None).await.unwrap() {
            SessionDecision::New { closed_session_id } => {
                assert!(
                    closed_session_id.is_none(),
                    "Main channel should never produce a closed_session_id"
                );
            }
            SessionDecision::Continue(_) => panic!("No session for this key, should be New"),
        }
    }

    #[tokio::test]
    async fn test_no_closed_id_when_session_manually_closed() {
        let (_tmp, mgr) = setup().await;
        let gap = Some(60); // 1h

        // Create a Telegram session and manually close it
        let id = mgr.create_session(&Channel::Telegram, "manual-close").await.unwrap();
        mgr.touch_session(&id).await.unwrap();
        mgr.save_message(&id, "user", "Hello").await.unwrap();
        mgr.close_session(&id, "Manually closed by user").await.unwrap();

        // Resolve should return New with closed_session_id=None because
        // there's no open session to auto-close
        match mgr.resolve_session(&Channel::Telegram, "manual-close", gap).await.unwrap() {
            SessionDecision::New { closed_session_id } => {
                assert!(
                    closed_session_id.is_none(),
                    "Manually closed session should not produce closed_session_id on resolve"
                );
            }
            SessionDecision::Continue(_) => panic!("Closed session should not be continued"),
        }
    }

    #[tokio::test]
    async fn test_cost_overview_empty() {
        let (_tmp, mgr) = setup().await;

        let overview = mgr.cost_overview(None).await.unwrap();
        assert_eq!(overview.total_cost_usd, 0.0);
        assert_eq!(overview.total_input_tokens, 0);
        assert_eq!(overview.total_output_tokens, 0);
        assert_eq!(overview.total_turns, 0);
        assert!(overview.by_user.is_empty());
        assert!(overview.by_model.is_empty());
    }

    #[tokio::test]
    async fn test_cost_overview_by_user() {
        let (_tmp, mgr) = setup().await;
        let sid = mgr.create_session(&Channel::Main, "cost-test").await.unwrap();

        // Record usage for two different users
        mgr.record_usage(&sid, &UsageRecord {
            input_tokens: 1000, output_tokens: 500, cache_read: 0, cache_write: 0,
            cost_usd: 0.05, model: "claude-sonnet".into(), user_id: "alice".into(),
        }, 1).await.unwrap();

        mgr.record_usage(&sid, &UsageRecord {
            input_tokens: 2000, output_tokens: 800, cache_read: 0, cache_write: 0,
            cost_usd: 0.10, model: "claude-sonnet".into(), user_id: "bob".into(),
        }, 2).await.unwrap();

        mgr.record_usage(&sid, &UsageRecord {
            input_tokens: 500, output_tokens: 200, cache_read: 0, cache_write: 0,
            cost_usd: 0.02, model: "claude-haiku".into(), user_id: "alice".into(),
        }, 3).await.unwrap();

        let overview = mgr.cost_overview(None).await.unwrap();

        // Totals
        assert_eq!(overview.total_turns, 3);
        assert!((overview.total_cost_usd - 0.17).abs() < 0.001);
        assert_eq!(overview.total_input_tokens, 3500);
        assert_eq!(overview.total_output_tokens, 1500);

        // By user (sorted by cost desc)
        assert_eq!(overview.by_user.len(), 2);
        assert_eq!(overview.by_user[0].user_id, "bob");
        assert!((overview.by_user[0].total_cost_usd - 0.10).abs() < 0.001);
        assert_eq!(overview.by_user[0].total_turns, 1);
        assert_eq!(overview.by_user[1].user_id, "alice");
        assert!((overview.by_user[1].total_cost_usd - 0.07).abs() < 0.001);
        assert_eq!(overview.by_user[1].total_turns, 2);

        // By model (sorted by cost desc)
        assert_eq!(overview.by_model.len(), 2);
        assert_eq!(overview.by_model[0].model, "claude-sonnet");
        assert!((overview.by_model[0].total_cost_usd - 0.15).abs() < 0.001);
        assert_eq!(overview.by_model[1].model, "claude-haiku");
        assert!((overview.by_model[1].total_cost_usd - 0.02).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_cost_overview_since_filter() {
        let (_tmp, mgr) = setup().await;
        let sid = mgr.create_session(&Channel::Main, "cost-filter").await.unwrap();

        // Record usage now
        mgr.record_usage(&sid, &UsageRecord {
            input_tokens: 1000, output_tokens: 500, cache_read: 0, cache_write: 0,
            cost_usd: 0.05, model: "claude-sonnet".into(), user_id: "admin".into(),
        }, 1).await.unwrap();

        // "Since" far in the future should return nothing
        let future = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let overview = mgr.cost_overview(Some(&future)).await.unwrap();
        assert_eq!(overview.total_turns, 0);
        assert_eq!(overview.total_cost_usd, 0.0);

        // "Since" far in the past should return everything
        let past = (Utc::now() - Duration::days(365)).to_rfc3339();
        let overview = mgr.cost_overview(Some(&past)).await.unwrap();
        assert_eq!(overview.total_turns, 1);
        assert!((overview.total_cost_usd - 0.05).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_cost_overview_user_id_recorded() {
        let (_tmp, mgr) = setup().await;
        let sid = mgr.create_session(&Channel::Main, "uid-test").await.unwrap();

        mgr.record_usage(&sid, &UsageRecord {
            input_tokens: 100, output_tokens: 50, cache_read: 0, cache_write: 0,
            cost_usd: 0.01, model: "m".into(), user_id: "user-42".into(),
        }, 1).await.unwrap();

        let overview = mgr.cost_overview(None).await.unwrap();
        assert_eq!(overview.by_user.len(), 1);
        assert_eq!(overview.by_user[0].user_id, "user-42");
        assert_eq!(overview.by_user[0].total_input_tokens, 100);
        assert_eq!(overview.by_user[0].total_output_tokens, 50);
    }

    #[tokio::test]
    async fn test_cost_overview_cache_breakdown() {
        let (_tmp, mgr) = setup().await;
        let sid = mgr.create_session(&Channel::Main, "cache-cost").await.unwrap();

        // Alice: cache miss (writes to cache)
        mgr.record_usage(&sid, &UsageRecord {
            input_tokens: 200, output_tokens: 100, cache_read: 0, cache_write: 3000,
            cost_usd: 0.04, model: "claude-sonnet".into(), user_id: "alice".into(),
        }, 1).await.unwrap();

        // Alice: cache hit (reads from cache)
        mgr.record_usage(&sid, &UsageRecord {
            input_tokens: 50, output_tokens: 150, cache_read: 3000, cache_write: 0,
            cost_usd: 0.01, model: "claude-sonnet".into(), user_id: "alice".into(),
        }, 2).await.unwrap();

        // Bob: no caching
        mgr.record_usage(&sid, &UsageRecord {
            input_tokens: 800, output_tokens: 400, cache_read: 0, cache_write: 0,
            cost_usd: 0.03, model: "claude-haiku".into(), user_id: "bob".into(),
        }, 3).await.unwrap();

        let overview = mgr.cost_overview(None).await.unwrap();

        // Totals: input = (200+0+3000) + (50+3000+0) + (800+0+0) = 7050
        assert_eq!(overview.total_input_tokens, 7050);
        assert_eq!(overview.total_output_tokens, 650);
        assert_eq!(overview.total_cache_read, 3000);
        assert_eq!(overview.total_cache_write, 3000);

        // By user: alice first (higher cost)
        assert_eq!(overview.by_user.len(), 2);
        let alice = overview.by_user.iter().find(|u| u.user_id == "alice").unwrap();
        assert_eq!(alice.total_input_tokens, 6250); // (200+3000) + (50+3000)
        assert_eq!(alice.total_cache_read, 3000);
        assert_eq!(alice.total_cache_write, 3000);

        let bob = overview.by_user.iter().find(|u| u.user_id == "bob").unwrap();
        assert_eq!(bob.total_input_tokens, 800);
        assert_eq!(bob.total_cache_read, 0);
        assert_eq!(bob.total_cache_write, 0);

        // By model
        let sonnet = overview.by_model.iter().find(|m| m.model == "claude-sonnet").unwrap();
        assert_eq!(sonnet.total_cache_read, 3000);
        assert_eq!(sonnet.total_cache_write, 3000);

        let haiku = overview.by_model.iter().find(|m| m.model == "claude-haiku").unwrap();
        assert_eq!(haiku.total_cache_read, 0);
        assert_eq!(haiku.total_cache_write, 0);
    }

    // ── Read/unread state tests ────────────────────────────────────────

    #[tokio::test]
    async fn test_new_session_is_read_by_default() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "key").await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert!(session.is_read, "New sessions should default to is_read=true");
    }

    #[tokio::test]
    async fn test_mark_read_false() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "key").await.unwrap();

        mgr.mark_read(&id, false).await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert!(!session.is_read, "Session should be unread after mark_read(false)");
    }

    #[tokio::test]
    async fn test_mark_read_true() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Main, "key").await.unwrap();

        // Mark unread, then mark read again
        mgr.mark_read(&id, false).await.unwrap();
        mgr.mark_read(&id, true).await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert!(session.is_read, "Session should be read after mark_read(true)");
    }

    #[tokio::test]
    async fn test_list_sessions_includes_is_read() {
        let (_tmp, mgr) = setup().await;
        let id1 = mgr.create_session(&Channel::Main, "key1").await.unwrap();
        let id2 = mgr.create_session(&Channel::Main, "key2").await.unwrap();

        mgr.mark_read(&id1, false).await.unwrap();

        let sessions = mgr.list_sessions(10).await.unwrap();
        let s1 = sessions.iter().find(|s| s.id == id1).unwrap();
        let s2 = sessions.iter().find(|s| s.id == id2).unwrap();

        assert!(!s1.is_read, "Session 1 should be unread");
        assert!(s2.is_read, "Session 2 should still be read");
    }

    #[tokio::test]
    async fn test_mark_read_nonexistent_session_succeeds() {
        let (_tmp, mgr) = setup().await;
        // Should not error — just a no-op UPDATE matching zero rows
        mgr.mark_read("nonexistent-id", true).await.unwrap();
    }

    // --- Email channel tests ---

    #[test]
    fn test_email_channel_as_str() {
        assert_eq!(Channel::Email.as_str(), "email");
    }

    #[test]
    fn test_email_channel_from_str() {
        assert_eq!(Channel::from_channel_str("email"), Channel::Email);
    }

    #[test]
    fn test_unknown_channel_defaults_to_main() {
        assert_eq!(Channel::from_channel_str("unknown"), Channel::Main);
        assert_eq!(Channel::from_channel_str(""), Channel::Main);
    }

    #[tokio::test]
    async fn test_create_email_session() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Email, "user@example.com").await.unwrap();

        let session = mgr.get_session(&id).await.unwrap().unwrap();
        assert_eq!(session.channel, "email");
        assert_eq!(session.channel_session_key.as_deref(), Some("user@example.com"));
    }

    #[tokio::test]
    async fn test_resolve_email_session_continues_for_same_sender() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Email, "sender@test.com").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        match mgr.resolve_session(&Channel::Email, "sender@test.com", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, id),
            SessionDecision::New { .. } => panic!("Should continue recent email session"),
        }
    }

    #[tokio::test]
    async fn test_resolve_email_session_new_for_different_sender() {
        let (_tmp, mgr) = setup().await;
        let id = mgr.create_session(&Channel::Email, "sender-a@test.com").await.unwrap();
        mgr.touch_session(&id).await.unwrap();

        match mgr.resolve_session(&Channel::Email, "sender-b@test.com", None).await.unwrap() {
            SessionDecision::New { .. } => {} // expected — different sender
            SessionDecision::Continue(_) => panic!("Should not continue session for different sender"),
        }
    }

    #[tokio::test]
    async fn test_email_and_telegram_sessions_are_separate() {
        let (_tmp, mgr) = setup().await;
        let email_id = mgr.create_session(&Channel::Email, "user@test.com").await.unwrap();
        let tg_id = mgr.create_session(&Channel::Telegram, "user@test.com").await.unwrap();

        assert_ne!(email_id, tg_id);

        // Each channel resolves independently
        mgr.touch_session(&email_id).await.unwrap();
        mgr.touch_session(&tg_id).await.unwrap();

        match mgr.resolve_session(&Channel::Email, "user@test.com", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, email_id),
            SessionDecision::New { .. } => panic!("Should continue email session"),
        }
        match mgr.resolve_session(&Channel::Telegram, "user@test.com", None).await.unwrap() {
            SessionDecision::Continue(sid) => assert_eq!(sid, tg_id),
            SessionDecision::New { .. } => panic!("Should continue telegram session"),
        }
    }

    // ── Tool usage stats tests ────────────────────────────────────────

    #[tokio::test]
    async fn test_cost_overview_by_tool_empty() {
        let (_tmp, mgr) = setup().await;

        let overview = mgr.cost_overview(None).await.unwrap();
        assert!(overview.by_tool.is_empty(), "No tool messages → empty by_tool");
    }

    #[tokio::test]
    async fn test_cost_overview_by_tool_counts() {
        let (_tmp, mgr) = setup().await;
        let sid = mgr.create_session(&Channel::Main, "tool-test").await.unwrap();

        // Simulate 3 MemorySearch invocations (all successful)
        for i in 0..3 {
            let tool_use = serde_json::json!({
                "type": "tool_use",
                "id": format!("tu_mem_{i}"),
                "name": "MemorySearch",
                "input": {"query": "test"}
            });
            mgr.save_message(&sid, "tool_use", &tool_use.to_string()).await.unwrap();

            let tool_result = serde_json::json!({
                "type": "tool_result",
                "tool_use_id": format!("tu_mem_{i}"),
                "content": "some result",
                "is_error": false
            });
            mgr.save_message(&sid, "tool_result", &tool_result.to_string()).await.unwrap();
        }

        // Simulate 2 VaultGet invocations: 1 success, 1 error
        let tool_use = serde_json::json!({
            "type": "tool_use", "id": "tu_vault_0", "name": "VaultGet",
            "input": {"key": "api_key"}
        });
        mgr.save_message(&sid, "tool_use", &tool_use.to_string()).await.unwrap();
        let tool_result = serde_json::json!({
            "type": "tool_result", "tool_use_id": "tu_vault_0",
            "content": "secret-value", "is_error": false
        });
        mgr.save_message(&sid, "tool_result", &tool_result.to_string()).await.unwrap();

        let tool_use = serde_json::json!({
            "type": "tool_use", "id": "tu_vault_1", "name": "VaultGet",
            "input": {"key": "missing"}
        });
        mgr.save_message(&sid, "tool_use", &tool_use.to_string()).await.unwrap();
        let tool_result = serde_json::json!({
            "type": "tool_result", "tool_use_id": "tu_vault_1",
            "content": "key not found", "is_error": true
        });
        mgr.save_message(&sid, "tool_result", &tool_result.to_string()).await.unwrap();

        let overview = mgr.cost_overview(None).await.unwrap();

        // Sorted by invocations DESC: MemorySearch(3), VaultGet(2)
        assert_eq!(overview.by_tool.len(), 2);
        assert_eq!(overview.by_tool[0].tool_name, "MemorySearch");
        assert_eq!(overview.by_tool[0].invocations, 3);
        assert_eq!(overview.by_tool[0].errors, 0);
        assert_eq!(overview.by_tool[1].tool_name, "VaultGet");
        assert_eq!(overview.by_tool[1].invocations, 2);
        assert_eq!(overview.by_tool[1].errors, 1);
    }

    #[tokio::test]
    async fn test_cost_overview_by_tool_since_filter() {
        let (_tmp, mgr) = setup().await;
        let sid = mgr.create_session(&Channel::Main, "tool-filter").await.unwrap();

        // Save a tool_use message now
        let tool_use = serde_json::json!({
            "type": "tool_use", "id": "tu_1", "name": "CronList", "input": {}
        });
        mgr.save_message(&sid, "tool_use", &tool_use.to_string()).await.unwrap();

        // "Since" far in the future should exclude it
        let future = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let overview = mgr.cost_overview(Some(&future)).await.unwrap();
        assert!(overview.by_tool.is_empty());

        // "Since" far in the past should include it
        let past = (Utc::now() - Duration::days(365)).to_rfc3339();
        let overview = mgr.cost_overview(Some(&past)).await.unwrap();
        assert_eq!(overview.by_tool.len(), 1);
        assert_eq!(overview.by_tool[0].tool_name, "CronList");
        assert_eq!(overview.by_tool[0].invocations, 1);
    }

    #[tokio::test]
    async fn test_cost_overview_by_tool_without_result() {
        let (_tmp, mgr) = setup().await;
        let sid = mgr.create_session(&Channel::Main, "tool-no-result").await.unwrap();

        // tool_use without a matching tool_result (e.g. stream interrupted)
        let tool_use = serde_json::json!({
            "type": "tool_use", "id": "tu_orphan", "name": "SkillList", "input": {}
        });
        mgr.save_message(&sid, "tool_use", &tool_use.to_string()).await.unwrap();

        let overview = mgr.cost_overview(None).await.unwrap();
        assert_eq!(overview.by_tool.len(), 1);
        assert_eq!(overview.by_tool[0].tool_name, "SkillList");
        assert_eq!(overview.by_tool[0].invocations, 1);
        assert_eq!(overview.by_tool[0].errors, 0, "No result means no error, not an error");
    }
}
