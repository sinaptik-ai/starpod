use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use tracing::warn;
use uuid::Uuid;

use starpod_core::{Result, StarpodError};

use crate::types::*;

/// Manages cron jobs in SQLite.
pub struct CronStore {
    pool: SqlitePool,
    default_max_retries: u32,
    default_timeout_secs: u64,
}

impl CronStore {
    /// Create a `CronStore` from a shared pool.
    ///
    /// The pool should already have migrations applied (via `CoreDb`).
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self {
            pool,
            default_max_retries: 3,
            default_timeout_secs: 7200,
        }
    }

    /// Set the default max retries for new jobs added via `add_job()`.
    pub fn set_default_max_retries(&mut self, v: u32) {
        self.default_max_retries = v;
    }

    /// Set the default timeout (in seconds) for new jobs added via `add_job()`.
    pub fn set_default_timeout_secs(&mut self, v: u64) {
        self.default_timeout_secs = v;
    }

    /// Add a new cron job. Returns the job ID.
    pub async fn add_job(
        &self,
        name: &str,
        prompt: &str,
        schedule: &Schedule,
        delete_after_run: bool,
        user_tz: Option<&str>,
    ) -> Result<String> {
        self.add_job_full(
            name,
            prompt,
            schedule,
            delete_after_run,
            user_tz,
            self.default_max_retries,
            self.default_timeout_secs as u32,
            SessionMode::Isolated,
            None,
        )
        .await
    }

    /// Add a new cron job with full options. Returns the job ID.
    pub async fn add_job_full(
        &self,
        name: &str,
        prompt: &str,
        schedule: &Schedule,
        delete_after_run: bool,
        user_tz: Option<&str>,
        max_retries: u32,
        timeout_secs: u32,
        session_mode: SessionMode,
        user_id: Option<&str>,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let (stype, svalue) = schedule_to_db(schedule);
        let next_run = compute_next_run(schedule, None, user_tz)?;
        let delete_flag = delete_after_run as i64;

        sqlx::query(
            "INSERT INTO cron_jobs (id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, next_run_at, max_retries, timeout_secs, session_mode, user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .bind(&id)
        .bind(name)
        .bind(prompt)
        .bind(stype)
        .bind(&svalue)
        .bind(delete_flag)
        .bind(now)
        .bind(next_run)
        .bind(max_retries as i64)
        .bind(timeout_secs as i64)
        .bind(session_mode.as_str())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to add job: {}", e)))?;

        Ok(id)
    }

    /// Remove a job by ID.
    pub async fn remove_job(&self, id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM cron_jobs WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to remove job: {}", e)))?;

        if result.rows_affected() == 0 {
            return Err(StarpodError::Cron(format!("Job '{}' not found", id)));
        }
        Ok(())
    }

    /// Remove a job by name.
    pub async fn remove_job_by_name(&self, name: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM cron_jobs WHERE name = ?1")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to remove job: {}", e)))?;

        if result.rows_affected() == 0 {
            return Err(StarpodError::Cron(format!("Job '{}' not found", name)));
        }
        Ok(())
    }

    /// List all jobs.
    pub async fn list_jobs(&self) -> Result<Vec<CronJob>> {
        let rows = sqlx::query(
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at, retry_count, max_retries, last_error, retry_at, timeout_secs, session_mode, user_id
             FROM cron_jobs ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to list jobs: {}", e)))?;

        Ok(rows.iter().map(job_from_row).collect())
    }

    /// List jobs belonging to a specific user.
    pub async fn list_jobs_for_user(&self, user_id: &str) -> Result<Vec<CronJob>> {
        let rows = sqlx::query(
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at, retry_count, max_retries, last_error, retry_at, timeout_secs, session_mode, user_id
             FROM cron_jobs WHERE user_id = ?1 ORDER BY name",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to list jobs for user: {}", e)))?;

        Ok(rows.iter().map(job_from_row).collect())
    }

    /// Get a job by name.
    /// Get a job by its unique ID.
    pub async fn get_job(&self, id: &str) -> Result<CronJob> {
        let row = sqlx::query(
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at, retry_count, max_retries, last_error, retry_at, timeout_secs, session_mode, user_id
             FROM cron_jobs WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to get job: {}", e)))?;

        row.as_ref()
            .map(job_from_row)
            .ok_or_else(|| StarpodError::Cron(format!("Job not found: {}", id)))
    }

    pub async fn get_job_by_name(&self, name: &str) -> Result<Option<CronJob>> {
        let row = sqlx::query(
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at, retry_count, max_retries, last_error, retry_at, timeout_secs, session_mode, user_id
             FROM cron_jobs WHERE name = ?1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to get job: {}", e)))?;

        Ok(row.as_ref().map(job_from_row))
    }

    /// Get jobs that are due for execution.
    pub async fn get_due_jobs(&self) -> Result<Vec<CronJob>> {
        let now = Utc::now().timestamp();

        let rows = sqlx::query(
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at, retry_count, max_retries, last_error, retry_at, timeout_secs, session_mode, user_id
             FROM cron_jobs
             WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to query due jobs: {}", e)))?;

        Ok(rows.iter().map(job_from_row).collect())
    }

    /// Get jobs that are due for retry.
    pub async fn get_retry_jobs(&self) -> Result<Vec<CronJob>> {
        let now = Utc::now().timestamp();

        let rows = sqlx::query(
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at, retry_count, max_retries, last_error, retry_at, timeout_secs, session_mode, user_id
             FROM cron_jobs
             WHERE retry_at IS NOT NULL AND retry_at <= ?1 AND enabled = 1",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to query retry jobs: {}", e)))?;

        Ok(rows.iter().map(job_from_row).collect())
    }

    /// Record that a job started running. Returns the run ID.
    pub async fn record_run_start(&self, job_id: &str) -> Result<String> {
        let run_id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();

        sqlx::query(
            "INSERT INTO cron_runs (id, job_id, started_at, status) VALUES (?1, ?2, ?3, 'running')",
        )
        .bind(&run_id)
        .bind(job_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to record run start: {}", e)))?;

        sqlx::query("UPDATE cron_jobs SET last_run_at = ?2 WHERE id = ?1")
            .bind(job_id)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to update last_run_at: {}", e)))?;

        Ok(run_id)
    }

    /// Record that a run completed.
    pub async fn record_run_complete(
        &self,
        run_id: &str,
        status: RunStatus,
        summary: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().timestamp();
        sqlx::query(
            "UPDATE cron_runs SET completed_at = ?2, status = ?3, result_summary = ?4 WHERE id = ?1",
        )
        .bind(run_id)
        .bind(now)
        .bind(status.as_str())
        .bind(summary)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to record run complete: {}", e)))?;
        Ok(())
    }

    /// Update the next_run_at for a job.
    pub async fn update_next_run(&self, job_id: &str, next: Option<i64>) -> Result<()> {
        sqlx::query("UPDATE cron_jobs SET next_run_at = ?2 WHERE id = ?1")
            .bind(job_id)
            .bind(next)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to update next_run: {}", e)))?;
        Ok(())
    }

    /// Disable a job (set enabled = 0).
    pub async fn disable_job(&self, job_id: &str) -> Result<()> {
        sqlx::query("UPDATE cron_jobs SET enabled = 0 WHERE id = ?1")
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to disable job: {}", e)))?;
        Ok(())
    }

    /// List recent runs for a job.
    pub async fn list_runs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRun>> {
        let rows = sqlx::query(
            "SELECT id, job_id, started_at, completed_at, status, result_summary, session_id
             FROM cron_runs WHERE job_id = ?1
             ORDER BY started_at DESC LIMIT ?2",
        )
        .bind(job_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to list runs: {}", e)))?;

        Ok(rows
            .iter()
            .map(|row| CronRun {
                id: row.get("id"),
                job_id: row.get("job_id"),
                started_at: row.get("started_at"),
                completed_at: row.get("completed_at"),
                status: RunStatus::from_str(row.get::<&str, _>("status")),
                result_summary: row.get("result_summary"),
                session_id: row.try_get("session_id").unwrap_or(None),
            })
            .collect())
    }

    /// Link a run to the session it created/used.
    pub async fn record_run_session(&self, run_id: &str, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE cron_runs SET session_id = ?2 WHERE id = ?1")
            .bind(run_id)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to record run session: {}", e)))?;
        Ok(())
    }

    // ── Retry helpers (Phase 1) ───────────────────────────────────────────

    /// Schedule a retry for a failed job.
    pub async fn schedule_retry(&self, job_id: &str, retry_at: i64, error: &str) -> Result<()> {
        sqlx::query(
            "UPDATE cron_jobs SET retry_count = retry_count + 1, retry_at = ?2, last_error = ?3 WHERE id = ?1",
        )
        .bind(job_id)
        .bind(retry_at)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to schedule retry: {}", e)))?;
        Ok(())
    }

    /// Reset retry state after a successful run.
    pub async fn reset_retry(&self, job_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE cron_jobs SET retry_count = 0, retry_at = NULL, last_error = NULL WHERE id = ?1",
        )
        .bind(job_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to reset retry: {}", e)))?;
        Ok(())
    }

    /// Mark a job as permanently failed (clear retry_at so it won't be picked up again).
    pub async fn mark_permanently_failed(&self, job_id: &str, error: &str) -> Result<()> {
        sqlx::query("UPDATE cron_jobs SET retry_at = NULL, last_error = ?2 WHERE id = ?1")
            .bind(job_id)
            .bind(error)
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to mark permanently failed: {}", e)))?;
        Ok(())
    }

    // ── Concurrency / Timeout helpers (Phase 2) ───────────────────────────

    /// Count currently running runs for a specific job.
    pub async fn count_running(&self, job_id: &str) -> Result<u32> {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM cron_runs WHERE job_id = ?1 AND status = 'running'",
        )
        .bind(job_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to count running: {}", e)))?;

        Ok(row.get::<i64, _>("cnt") as u32)
    }

    /// Count total running runs across all jobs.
    pub async fn count_all_running(&self) -> Result<u32> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM cron_runs WHERE status = 'running'")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to count all running: {}", e)))?;

        Ok(row.get::<i64, _>("cnt") as u32)
    }

    /// Mark stuck runs as failed. Returns how many were reaped.
    pub async fn timeout_stuck_runs(&self) -> Result<u32> {
        let now = Utc::now().timestamp();

        // Join with cron_jobs to get per-job timeout_secs
        let result = sqlx::query(
            "UPDATE cron_runs SET status = 'failed', completed_at = ?1,
                    result_summary = 'Timed out (stuck job reaper)'
             WHERE status = 'running'
               AND id IN (
                   SELECT r.id FROM cron_runs r
                   JOIN cron_jobs j ON r.job_id = j.id
                   WHERE r.status = 'running'
                     AND r.started_at + j.timeout_secs < ?1
               )",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to timeout stuck runs: {}", e)))?;

        Ok(result.rows_affected() as u32)
    }

    // ── Update helper (Phase 4) ───────────────────────────────────────────

    /// Update a job's fields dynamically (only provided fields are changed).
    pub async fn update_job(&self, id: &str, update: &JobUpdate) -> Result<()> {
        // Build dynamic SQL with parameterized queries.
        // We reserve ?1 for the WHERE clause (id), so SET params start at ?2.
        let mut set_clauses: Vec<String> = Vec::new();
        let mut params: Vec<String> = Vec::new(); // collected as strings for binding

        if let Some(ref prompt) = update.prompt {
            params.push(prompt.clone());
            set_clauses.push(format!("prompt = ?{}", params.len() + 1));
        }
        if let Some(ref schedule) = update.schedule {
            let (stype, svalue) = schedule_to_db(schedule);
            params.push(stype.to_string());
            set_clauses.push(format!("schedule_type = ?{}", params.len() + 1));
            params.push(svalue);
            set_clauses.push(format!("schedule_value = ?{}", params.len() + 1));
        }
        if let Some(enabled) = update.enabled {
            params.push((enabled as i64).to_string());
            set_clauses.push(format!("enabled = ?{}", params.len() + 1));
        }
        if let Some(max_retries) = update.max_retries {
            params.push(max_retries.to_string());
            set_clauses.push(format!("max_retries = ?{}", params.len() + 1));
        }
        if let Some(timeout_secs) = update.timeout_secs {
            params.push(timeout_secs.to_string());
            set_clauses.push(format!("timeout_secs = ?{}", params.len() + 1));
        }
        if let Some(ref session_mode) = update.session_mode {
            params.push(session_mode.as_str().to_string());
            set_clauses.push(format!("session_mode = ?{}", params.len() + 1));
        }

        if set_clauses.is_empty() {
            return Ok(()); // nothing to update
        }

        let sql = format!(
            "UPDATE cron_jobs SET {} WHERE id = ?1",
            set_clauses.join(", ")
        );
        let mut query = sqlx::query(&sql).bind(id.to_string());
        for param in &params {
            query = query.bind(param.clone());
        }
        query
            .execute(&self.pool)
            .await
            .map_err(|e| StarpodError::Cron(format!("Failed to update job: {}", e)))?;

        Ok(())
    }
}

/// Compute exponential backoff delay in seconds for a given retry count.
///
/// The delay schedule is: 30s, 60s, 5m, 15m, 1h. Retry counts beyond the
/// table length are clamped to the maximum (1 hour).
///
/// ```
/// use starpod_cron::store::backoff_delay;
///
/// assert_eq!(backoff_delay(0), 30);   // 30 seconds
/// assert_eq!(backoff_delay(1), 60);   // 1 minute
/// assert_eq!(backoff_delay(2), 300);  // 5 minutes
/// assert_eq!(backoff_delay(3), 900);  // 15 minutes
/// assert_eq!(backoff_delay(4), 3600); // 1 hour
/// assert_eq!(backoff_delay(99), 3600); // clamped to max
/// ```
pub fn backoff_delay(retry_count: u32) -> i64 {
    const DELAYS: [i64; 5] = [30, 60, 300, 900, 3600];
    let idx = (retry_count as usize).min(DELAYS.len() - 1);
    DELAYS[idx]
}

fn job_from_row(row: &SqliteRow) -> CronJob {
    let stype: String = row.get("schedule_type");
    let svalue: String = row.get("schedule_value");
    CronJob {
        id: row.get("id"),
        name: row.get("name"),
        prompt: row.get("prompt"),
        schedule: schedule_from_db(&stype, &svalue),
        enabled: row.get::<i64, _>("enabled") != 0,
        delete_after_run: row.get::<i64, _>("delete_after_run") != 0,
        created_at: row.get("created_at"),
        last_run_at: row.get("last_run_at"),
        next_run_at: row.get("next_run_at"),
        retry_count: row.get::<i64, _>("retry_count") as u32,
        max_retries: row.get::<i64, _>("max_retries") as u32,
        last_error: row.get("last_error"),
        retry_at: row.get("retry_at"),
        timeout_secs: row.get::<i64, _>("timeout_secs") as u32,
        session_mode: SessionMode::from_str(row.get::<&str, _>("session_mode")),
        user_id: row.get("user_id"),
    }
}

fn schedule_to_db(schedule: &Schedule) -> (&'static str, String) {
    match schedule {
        Schedule::OneShot { at } => ("one_shot", at.clone()),
        Schedule::Interval { every_ms } => ("interval", every_ms.to_string()),
        Schedule::Cron { expr } => ("cron", expr.clone()),
    }
}

fn schedule_from_db(stype: &str, svalue: &str) -> Schedule {
    match stype {
        "one_shot" => Schedule::OneShot {
            at: svalue.to_string(),
        },
        "interval" => Schedule::Interval {
            every_ms: svalue.parse().unwrap_or(60000),
        },
        "cron" => Schedule::Cron {
            expr: svalue.to_string(),
        },
        _ => Schedule::Interval { every_ms: 60000 },
    }
}

/// Convert a Unix epoch timestamp to an RFC3339 string for display.
pub fn epoch_to_rfc3339(epoch: i64) -> String {
    DateTime::from_timestamp(epoch, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| epoch.to_string())
}

/// Compute the next run time for a schedule. Returns a Unix epoch timestamp.
///
/// `user_tz` is an optional IANA timezone string (e.g. "Europe/Rome").
/// For `Cron` schedules, expressions are evaluated in the user's timezone
/// so that "0 23 * * *" fires at 23:00 local time. Falls back to UTC
/// if the timezone is `None` or invalid.
///
/// Cron expressions with 5 fields (standard) are auto-expanded to 6 fields
/// by prepending `0` for the seconds field.
pub fn compute_next_run(
    schedule: &Schedule,
    last_run: Option<DateTime<Utc>>,
    user_tz: Option<&str>,
) -> Result<Option<i64>> {
    let now = Utc::now();
    match schedule {
        Schedule::OneShot { at } => {
            // Try RFC 3339 first (has explicit timezone offset)
            let dt_utc = if let Ok(dt) = DateTime::parse_from_rfc3339(at) {
                dt.with_timezone(&Utc)
            } else if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M:%S")
            {
                // Naive timestamp (no offset) — interpret in user_tz if available, else UTC
                match user_tz.and_then(|s| s.parse::<chrono_tz::Tz>().ok()) {
                    Some(tz) => match naive.and_local_timezone(tz) {
                        chrono::LocalResult::Single(local) => local.with_timezone(&Utc),
                        chrono::LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
                        chrono::LocalResult::None => {
                            return Err(StarpodError::Cron(format!(
                                "Timestamp '{}' does not exist in timezone '{}'",
                                at, tz
                            )));
                        }
                    },
                    None => {
                        warn!(
                            timestamp = %at,
                            "One-shot timestamp has no timezone offset and no user timezone configured — assuming UTC"
                        );
                        naive.and_utc()
                    }
                }
            } else {
                return Err(StarpodError::Cron(format!(
                    "Invalid timestamp '{}': expected ISO 8601 format (e.g. '2026-03-19T09:00:00Z' or '2026-03-19T09:00:00')", at
                )));
            };

            if dt_utc > now && last_run.is_none() {
                Ok(Some(dt_utc.timestamp()))
            } else {
                Ok(None)
            }
        }
        Schedule::Interval { every_ms } => {
            let base = last_run.unwrap_or(now);
            let next = base + Duration::milliseconds(*every_ms as i64);
            Ok(Some(next.timestamp()))
        }
        Schedule::Cron { expr } => {
            // Auto-prepend seconds field if expression has only 5 fields (standard cron)
            let effective_expr = if expr.split_whitespace().count() == 5 {
                format!("0 {}", expr)
            } else {
                expr.clone()
            };

            let cron_schedule = cron::Schedule::from_str(&effective_expr).map_err(|e| {
                StarpodError::Cron(format!("Invalid cron expression '{}': {}", expr, e))
            })?;

            let next_utc = match user_tz.and_then(|s| s.parse::<chrono_tz::Tz>().ok()) {
                Some(tz) => cron_schedule
                    .upcoming(tz)
                    .next()
                    .map(|dt| dt.with_timezone(&Utc)),
                None => {
                    if user_tz.is_some() {
                        warn!(tz = ?user_tz, "Invalid timezone, falling back to UTC");
                    }
                    cron_schedule.upcoming(Utc).next()
                }
            };

            Ok(next_utc.map(|dt| dt.timestamp()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> CronStore {
        let db = starpod_db::CoreDb::in_memory().await.unwrap();
        CronStore::from_pool(db.pool().clone())
    }

    #[tokio::test]
    async fn test_add_and_list_jobs() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job(
                "check-deploy",
                "Check deploy status",
                &schedule,
                false,
                None,
            )
            .await
            .unwrap();
        assert!(!id.is_empty());

        let jobs = store.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "check-deploy");
        assert_eq!(jobs[0].prompt, "Check deploy status");
        assert!(jobs[0].enabled);
        assert!(jobs[0].next_run_at.is_some());
        assert_eq!(jobs[0].max_retries, 3); // default
        assert_eq!(jobs[0].timeout_secs, 7200); // default
        assert_eq!(jobs[0].session_mode, SessionMode::Isolated); // default
    }

    #[tokio::test]
    async fn test_get_job_by_id() {
        let store = setup().await;
        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("find-by-id", "test prompt", &schedule, false, None)
            .await
            .unwrap();

        let job = store.get_job(&id).await.unwrap();
        assert_eq!(job.id, id);
        assert_eq!(job.name, "find-by-id");
        assert_eq!(job.prompt, "test prompt");
    }

    #[tokio::test]
    async fn test_get_job_not_found() {
        let store = setup().await;
        let result = store.get_job("nonexistent-id").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_add_job_full() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job_full(
                "my-job",
                "Do stuff",
                &schedule,
                false,
                None,
                5,
                3600,
                SessionMode::Main,
                None,
            )
            .await
            .unwrap();
        assert!(!id.is_empty());

        let jobs = store.list_jobs().await.unwrap();
        assert_eq!(jobs[0].max_retries, 5);
        assert_eq!(jobs[0].timeout_secs, 3600);
        assert_eq!(jobs[0].session_mode, SessionMode::Main);
    }

    #[tokio::test]
    async fn test_remove_job() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("temp", "temp job", &schedule, false, None)
            .await
            .unwrap();

        store.remove_job(&id).await.unwrap();
        assert_eq!(store.list_jobs().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_remove_by_name() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("my-job", "do stuff", &schedule, false, None)
            .await
            .unwrap();

        store.remove_job_by_name("my-job").await.unwrap();
        assert_eq!(store.list_jobs().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_get_job_by_name() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("find-me", "find this", &schedule, false, None)
            .await
            .unwrap();

        let found = store.get_job_by_name("find-me").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().prompt, "find this");

        let missing = store.get_job_by_name("nope").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_run_lifecycle() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let job_id = store
            .add_job("test-job", "test", &schedule, false, None)
            .await
            .unwrap();

        let run_id = store.record_run_start(&job_id).await.unwrap();

        let runs = store.list_runs(&job_id, 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Running);

        store
            .record_run_complete(&run_id, RunStatus::Success, Some("All good"))
            .await
            .unwrap();

        let runs = store.list_runs(&job_id, 10).await.unwrap();
        assert_eq!(runs[0].status, RunStatus::Success);
        assert_eq!(runs[0].result_summary.as_deref(), Some("All good"));
    }

    #[tokio::test]
    async fn test_retry_lifecycle() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let job_id = store
            .add_job("retry-job", "test", &schedule, false, None)
            .await
            .unwrap();

        // Schedule a retry
        let retry_at = Utc::now().timestamp() + 30;
        store
            .schedule_retry(&job_id, retry_at, "Connection refused")
            .await
            .unwrap();

        let jobs = store.list_jobs().await.unwrap();
        assert_eq!(jobs[0].retry_count, 1);
        assert_eq!(jobs[0].last_error.as_deref(), Some("Connection refused"));
        assert_eq!(jobs[0].retry_at, Some(retry_at));

        // Reset after success
        store.reset_retry(&job_id).await.unwrap();

        let jobs = store.list_jobs().await.unwrap();
        assert_eq!(jobs[0].retry_count, 0);
        assert!(jobs[0].last_error.is_none());
        assert!(jobs[0].retry_at.is_none());
    }

    #[tokio::test]
    async fn test_count_running() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let job_id = store
            .add_job("count-job", "test", &schedule, false, None)
            .await
            .unwrap();

        assert_eq!(store.count_running(&job_id).await.unwrap(), 0);
        assert_eq!(store.count_all_running().await.unwrap(), 0);

        store.record_run_start(&job_id).await.unwrap();

        assert_eq!(store.count_running(&job_id).await.unwrap(), 1);
        assert_eq!(store.count_all_running().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_update_job() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("update-me", "old prompt", &schedule, false, None)
            .await
            .unwrap();

        store
            .update_job(
                &id,
                &JobUpdate {
                    prompt: Some("new prompt".into()),
                    max_retries: Some(10),
                    session_mode: Some(SessionMode::Main),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let job = store.get_job_by_name("update-me").await.unwrap().unwrap();
        assert_eq!(job.prompt, "new prompt");
        assert_eq!(job.max_retries, 10);
        assert_eq!(job.session_mode, SessionMode::Main);
    }

    #[tokio::test]
    async fn test_backoff_delay() {
        assert_eq!(backoff_delay(0), 30);
        assert_eq!(backoff_delay(1), 60);
        assert_eq!(backoff_delay(2), 300);
        assert_eq!(backoff_delay(3), 900);
        assert_eq!(backoff_delay(4), 3600);
        assert_eq!(backoff_delay(100), 3600); // clamped
    }

    #[tokio::test]
    async fn test_compute_next_run_interval() {
        let schedule = Schedule::Interval { every_ms: 300000 };
        let next = compute_next_run(&schedule, None, None).unwrap();
        assert!(next.is_some());
        let epoch = next.unwrap();
        assert!(epoch > Utc::now().timestamp(), "Should be in the future");
    }

    #[tokio::test]
    async fn test_compute_next_run_cron_six_fields() {
        let schedule = Schedule::Cron {
            expr: "0 0 * * * *".to_string(),
        };
        let next = compute_next_run(&schedule, None, None).unwrap();
        assert!(next.is_some());
        assert!(next.unwrap() > Utc::now().timestamp());
    }

    #[tokio::test]
    async fn test_compute_next_run_cron_five_fields() {
        // Standard 5-field cron (no seconds) — should be auto-expanded
        let schedule = Schedule::Cron {
            expr: "0 9 * * *".to_string(),
        };
        let next = compute_next_run(&schedule, None, None).unwrap();
        assert!(
            next.is_some(),
            "5-field cron should be auto-expanded to 6 fields"
        );
    }

    #[tokio::test]
    async fn test_compute_next_run_cron_with_timezone() {
        // "Every day at 23:00" — should differ between UTC and Europe/Rome
        let schedule = Schedule::Cron {
            expr: "0 0 23 * * *".to_string(),
        };
        let next_utc = compute_next_run(&schedule, None, None).unwrap().unwrap();
        let next_rome = compute_next_run(&schedule, None, Some("Europe/Rome"))
            .unwrap()
            .unwrap();
        // Rome is UTC+1 or UTC+2 (DST), so next_rome should differ
        assert_ne!(
            next_utc, next_rome,
            "Timezone should affect computed next_run"
        );
    }

    #[tokio::test]
    async fn test_one_shot_with_timezone_offset() {
        // A one-shot at 23:00+01:00 should be stored as 22:00 UTC
        let schedule = Schedule::OneShot {
            at: "2030-06-15T23:00:00+01:00".to_string(),
        };
        let next = compute_next_run(&schedule, None, None).unwrap();
        assert!(next.is_some());
        let expected = DateTime::parse_from_rfc3339("2030-06-15T22:00:00+00:00")
            .unwrap()
            .timestamp();
        assert_eq!(next.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_get_due_jobs() {
        let store = setup().await;

        // Insert a job with next_run_at in the past
        let past = (Utc::now() - Duration::hours(1)).timestamp();
        sqlx::query(
            "INSERT INTO cron_jobs (id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, next_run_at)
             VALUES ('id1', 'old-job', 'do stuff', 'cron', '0 0 9 * * *', 1, 0, ?1, ?2)"
        )
        .bind(past)
        .bind(past)
        .execute(&store.pool)
        .await
        .unwrap();

        let due = store.get_due_jobs().await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].name, "old-job");
    }

    #[tokio::test]
    async fn test_get_retry_jobs() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("retry-test", "test", &schedule, false, None)
            .await
            .unwrap();

        // No retry jobs yet
        assert!(store.get_retry_jobs().await.unwrap().is_empty());

        // Schedule a retry in the past
        let past = Utc::now().timestamp() - 10;
        store.schedule_retry(&id, past, "err").await.unwrap();

        let retry_jobs = store.get_retry_jobs().await.unwrap();
        assert_eq!(retry_jobs.len(), 1);
        assert_eq!(retry_jobs[0].name, "retry-test");
    }

    #[tokio::test]
    async fn test_disable_job() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("j", "p", &schedule, false, None)
            .await
            .unwrap();

        store.disable_job(&id).await.unwrap();

        let jobs = store.list_jobs().await.unwrap();
        assert!(!jobs[0].enabled);
    }

    #[tokio::test]
    async fn test_mark_permanently_failed() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("perm-fail", "test", &schedule, false, None)
            .await
            .unwrap();

        // Schedule a retry first
        let retry_at = Utc::now().timestamp() + 30;
        store
            .schedule_retry(&id, retry_at, "transient error")
            .await
            .unwrap();
        assert!(store.list_jobs().await.unwrap()[0].retry_at.is_some());

        // Mark permanently failed — should clear retry_at but keep last_error
        store
            .mark_permanently_failed(&id, "fatal error")
            .await
            .unwrap();

        let job = store.get_job_by_name("perm-fail").await.unwrap().unwrap();
        assert!(job.retry_at.is_none(), "retry_at should be cleared");
        assert_eq!(job.last_error.as_deref(), Some("fatal error"));
    }

    #[tokio::test]
    async fn test_timeout_stuck_runs() {
        let store = setup().await;

        // Create a job with a very short timeout
        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job_full(
                "timeout-job",
                "test",
                &schedule,
                false,
                None,
                3,
                1,
                SessionMode::Isolated,
                None,
            )
            .await
            .unwrap();

        // Record a run start, then manually backdate started_at to simulate a stuck run
        let run_id = store.record_run_start(&id).await.unwrap();

        // Backdate the run to 10 seconds ago (timeout is 1 second)
        let past = Utc::now().timestamp() - 10;
        sqlx::query("UPDATE cron_runs SET started_at = ?2 WHERE id = ?1")
            .bind(&run_id)
            .bind(past)
            .execute(&store.pool)
            .await
            .unwrap();

        // The run should be running before reaping
        assert_eq!(store.count_running(&id).await.unwrap(), 1);

        // Reap stuck runs
        let reaped = store.timeout_stuck_runs().await.unwrap();
        assert_eq!(reaped, 1);

        // The run should now be marked as failed
        assert_eq!(store.count_running(&id).await.unwrap(), 0);

        let runs = store.list_runs(&id, 10).await.unwrap();
        assert_eq!(runs[0].status, RunStatus::Failed);
        assert_eq!(
            runs[0].result_summary.as_deref(),
            Some("Timed out (stuck job reaper)")
        );
    }

    #[tokio::test]
    async fn test_timeout_does_not_reap_within_limit() {
        let store = setup().await;

        // Job with a generous timeout
        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job_full(
                "safe-job",
                "test",
                &schedule,
                false,
                None,
                3,
                7200,
                SessionMode::Isolated,
                None,
            )
            .await
            .unwrap();

        store.record_run_start(&id).await.unwrap();

        // Should not reap — the run just started and timeout is 2 hours
        let reaped = store.timeout_stuck_runs().await.unwrap();
        assert_eq!(reaped, 0);
        assert_eq!(store.count_running(&id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_update_job_noop() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("noop-update", "original", &schedule, false, None)
            .await
            .unwrap();

        // Empty update should be a no-op
        store.update_job(&id, &JobUpdate::default()).await.unwrap();

        let job = store.get_job_by_name("noop-update").await.unwrap().unwrap();
        assert_eq!(job.prompt, "original");
    }

    #[tokio::test]
    async fn test_update_job_schedule() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("schedule-update", "test", &schedule, false, None)
            .await
            .unwrap();

        let new_schedule = Schedule::Cron {
            expr: "0 0 9 * * *".into(),
        };
        store
            .update_job(
                &id,
                &JobUpdate {
                    schedule: Some(new_schedule),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let job = store
            .get_job_by_name("schedule-update")
            .await
            .unwrap()
            .unwrap();
        match job.schedule {
            Schedule::Cron { ref expr } => assert_eq!(expr, "0 0 9 * * *"),
            _ => panic!("Expected cron schedule"),
        }
    }

    #[tokio::test]
    async fn test_update_job_enabled() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("toggle-job", "test", &schedule, false, None)
            .await
            .unwrap();

        // Disable via update
        store
            .update_job(
                &id,
                &JobUpdate {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let job = store.get_job_by_name("toggle-job").await.unwrap().unwrap();
        assert!(!job.enabled);

        // Re-enable
        store
            .update_job(
                &id,
                &JobUpdate {
                    enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let job = store.get_job_by_name("toggle-job").await.unwrap().unwrap();
        assert!(job.enabled);
    }

    #[tokio::test]
    async fn test_update_job_timeout() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("timeout-update", "test", &schedule, false, None)
            .await
            .unwrap();

        store
            .update_job(
                &id,
                &JobUpdate {
                    timeout_secs: Some(300),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let job = store
            .get_job_by_name("timeout-update")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.timeout_secs, 300);
    }

    #[tokio::test]
    async fn test_retry_count_increments() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("inc-retry", "test", &schedule, false, None)
            .await
            .unwrap();

        // Three successive retries
        for i in 0..3 {
            let retry_at = Utc::now().timestamp() + 30;
            store
                .schedule_retry(&id, retry_at, &format!("error {}", i))
                .await
                .unwrap();
        }

        let job = store.get_job_by_name("inc-retry").await.unwrap().unwrap();
        assert_eq!(job.retry_count, 3);
        assert_eq!(job.last_error.as_deref(), Some("error 2"));
    }

    #[tokio::test]
    async fn test_get_retry_jobs_respects_disabled() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("disabled-retry", "test", &schedule, false, None)
            .await
            .unwrap();

        // Schedule a retry in the past
        let past = Utc::now().timestamp() - 10;
        store.schedule_retry(&id, past, "err").await.unwrap();

        // Disable the job
        store.disable_job(&id).await.unwrap();

        // Should not appear in retry jobs (enabled = 0)
        let retry_jobs = store.get_retry_jobs().await.unwrap();
        assert!(retry_jobs.is_empty());
    }

    #[tokio::test]
    async fn test_set_default_max_retries_affects_add_job() {
        let mut store = setup().await;
        store.set_default_max_retries(5);

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("retry-default", "test", &schedule, false, None)
            .await
            .unwrap();

        let job = store
            .get_job_by_name("retry-default")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            job.max_retries, 5,
            "add_job should use the custom default_max_retries"
        );
    }

    #[tokio::test]
    async fn test_set_default_timeout_secs_affects_add_job() {
        let mut store = setup().await;
        store.set_default_timeout_secs(3600);

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("timeout-default", "test", &schedule, false, None)
            .await
            .unwrap();

        let job = store
            .get_job_by_name("timeout-default")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            job.timeout_secs, 3600,
            "add_job should use the custom default_timeout_secs"
        );
    }

    #[tokio::test]
    async fn test_get_retry_jobs_future_not_due() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("future-retry", "test", &schedule, false, None)
            .await
            .unwrap();

        // Schedule a retry far in the future
        let future = Utc::now().timestamp() + 99999;
        store.schedule_retry(&id, future, "err").await.unwrap();

        // Should not appear yet
        let retry_jobs = store.get_retry_jobs().await.unwrap();
        assert!(retry_jobs.is_empty());
    }

    #[tokio::test]
    async fn test_count_running_multiple_jobs() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id1 = store
            .add_job("job-a", "test", &schedule, false, None)
            .await
            .unwrap();
        let id2 = store
            .add_job("job-b", "test", &schedule, false, None)
            .await
            .unwrap();

        store.record_run_start(&id1).await.unwrap();
        store.record_run_start(&id2).await.unwrap();
        store.record_run_start(&id2).await.unwrap(); // two runs for job-b

        assert_eq!(store.count_running(&id1).await.unwrap(), 1);
        assert_eq!(store.count_running(&id2).await.unwrap(), 2);
        assert_eq!(store.count_all_running().await.unwrap(), 3);
    }

    // ── SQL injection regression tests ──────────────────────────────────

    #[tokio::test]
    async fn test_update_job_with_sql_injection_in_prompt() {
        let store = setup().await;
        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("inject-test", "original", &schedule, false, None)
            .await
            .unwrap();

        // Attempt SQL injection via prompt — this would corrupt data with string formatting
        store
            .update_job(
                &id,
                &JobUpdate {
                    prompt: Some("'); DROP TABLE cron_jobs; --".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Table should still exist and contain our job
        let jobs = store.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].prompt, "'); DROP TABLE cron_jobs; --");
    }

    #[tokio::test]
    async fn test_update_job_with_special_chars_in_prompt() {
        let store = setup().await;
        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("special-chars", "original", &schedule, false, None)
            .await
            .unwrap();

        let special = "Hello 'world' \"quotes\" \\ backslash; semicolon -- comment /* block */";
        store
            .update_job(
                &id,
                &JobUpdate {
                    prompt: Some(special.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let job = store
            .get_job_by_name("special-chars")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.prompt, special);
    }

    // ── OneShot timezone tests ──

    #[tokio::test]
    async fn test_compute_next_run_oneshot_rfc3339() {
        // RFC 3339 timestamp with Z suffix — should work
        let future = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let schedule = Schedule::OneShot { at: future };
        let next = compute_next_run(&schedule, None, None).unwrap();
        assert!(
            next.is_some(),
            "Future RFC 3339 timestamp should be scheduled"
        );
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_past_returns_none() {
        let past = (Utc::now() - Duration::hours(1)).to_rfc3339();
        let schedule = Schedule::OneShot { at: past };
        let next = compute_next_run(&schedule, None, None).unwrap();
        assert!(next.is_none(), "Past timestamp should return None");
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_naive_with_timezone() {
        // A naive timestamp 1 hour from now in Europe/Rome
        let rome_tz: chrono_tz::Tz = "Europe/Rome".parse().unwrap();
        let rome_now = Utc::now().with_timezone(&rome_tz);
        let future_rome = rome_now + Duration::hours(1);
        let naive_str = future_rome.format("%Y-%m-%dT%H:%M:%S").to_string();

        let schedule = Schedule::OneShot { at: naive_str };
        let next = compute_next_run(&schedule, None, Some("Europe/Rome")).unwrap();
        assert!(
            next.is_some(),
            "Naive timestamp with user_tz should be scheduled"
        );
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_naive_no_timezone_assumes_utc() {
        // A naive timestamp 1 hour from now — should assume UTC
        let future_utc = Utc::now() + Duration::hours(1);
        let naive_str = future_utc.format("%Y-%m-%dT%H:%M:%S").to_string();

        let schedule = Schedule::OneShot { at: naive_str };
        let next = compute_next_run(&schedule, None, None).unwrap();
        assert!(
            next.is_some(),
            "Naive timestamp without user_tz should assume UTC"
        );
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_naive_timezone_matters() {
        // Create a timestamp that is "23:00" today. Depending on timezone,
        // this could be in the future or the past relative to UTC now.
        // Use a timezone far ahead of UTC to test the difference.
        let schedule = Schedule::OneShot {
            at: "2099-01-01T00:00:00".to_string(),
        };

        let next_utc = compute_next_run(&schedule, None, None).unwrap();
        let next_tokyo = compute_next_run(&schedule, None, Some("Asia/Tokyo")).unwrap();

        // Both should be Some (far future), but the UTC timestamps should differ
        // because Tokyo is UTC+9, so 2099-01-01T00:00:00 Tokyo = 2098-12-31T15:00:00 UTC
        assert!(next_utc.is_some());
        assert!(next_tokyo.is_some());
        assert_ne!(
            next_utc.unwrap(),
            next_tokyo.unwrap(),
            "Same naive timestamp should produce different UTC epochs for different timezones"
        );
        // Tokyo interpretation should be 9 hours earlier in UTC
        assert_eq!(next_utc.unwrap() - next_tokyo.unwrap(), 9 * 3600);
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_invalid_format() {
        let schedule = Schedule::OneShot {
            at: "not-a-date".to_string(),
        };
        assert!(compute_next_run(&schedule, None, None).is_err());
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_rfc3339_with_offset() {
        // Explicit non-Z offset (e.g. India Standard Time +05:30)
        let schedule = Schedule::OneShot {
            at: "2099-06-15T10:00:00+05:30".to_string(),
        };
        let next = compute_next_run(&schedule, None, None).unwrap();
        assert!(next.is_some());
        // 10:00 IST = 04:30 UTC
        let expected = chrono::DateTime::parse_from_rfc3339("2099-06-15T04:30:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(next.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_naive_past_with_timezone() {
        // Naive timestamp well in the past — should return None even with user_tz
        let schedule = Schedule::OneShot {
            at: "2020-01-01T00:00:00".to_string(),
        };
        let next = compute_next_run(&schedule, None, Some("Europe/Rome")).unwrap();
        assert!(next.is_none(), "Past naive timestamp should return None");
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_naive_invalid_timezone_falls_to_utc() {
        // Invalid timezone string — should fall through to UTC
        let future_utc = Utc::now() + Duration::hours(1);
        let naive_str = future_utc.format("%Y-%m-%dT%H:%M:%S").to_string();

        let schedule = Schedule::OneShot {
            at: naive_str.clone(),
        };
        let next_bad_tz = compute_next_run(&schedule, None, Some("Not/A/Timezone")).unwrap();
        let next_no_tz = compute_next_run(&schedule, None, None).unwrap();

        // Both should produce the same result (UTC fallback)
        assert_eq!(next_bad_tz, next_no_tz);
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_dst_gap_returns_error() {
        // 2026-03-29 02:30:00 doesn't exist in Europe/Rome (spring DST: clocks jump 02:00 → 03:00)
        let schedule = Schedule::OneShot {
            at: "2026-03-29T02:30:00".to_string(),
        };
        let result = compute_next_run(&schedule, None, Some("Europe/Rome"));
        assert!(result.is_err(), "DST gap timestamp should return error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not exist"),
            "Error should mention the timestamp doesn't exist: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_dst_ambiguous_uses_earliest() {
        // 2026-10-25 02:30:00 is ambiguous in Europe/Rome (fall DST: clocks go 03:00 → 02:00)
        let schedule = Schedule::OneShot {
            at: "2026-10-25T02:30:00".to_string(),
        };
        let result = compute_next_run(&schedule, None, Some("Europe/Rome"));
        assert!(
            result.is_ok(),
            "Ambiguous DST timestamp should succeed (use earliest)"
        );
        assert!(result.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_compute_next_run_oneshot_already_ran() {
        // Even if the timestamp is in the future, last_run = Some means it already executed
        let future = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let schedule = Schedule::OneShot { at: future };
        let next = compute_next_run(&schedule, Some(Utc::now()), None).unwrap();
        assert!(
            next.is_none(),
            "OneShot with last_run=Some should return None"
        );
    }

    #[tokio::test]
    async fn test_list_jobs_for_user() {
        let store = setup().await;
        let schedule = Schedule::Interval { every_ms: 60000 };

        // Add jobs for different users and an agent-level job
        store
            .add_job_full(
                "alice-job",
                "Alice's job",
                &schedule,
                false,
                None,
                3,
                7200,
                SessionMode::Isolated,
                Some("alice"),
            )
            .await
            .unwrap();
        store
            .add_job_full(
                "bob-job",
                "Bob's job",
                &schedule,
                false,
                None,
                3,
                7200,
                SessionMode::Isolated,
                Some("bob"),
            )
            .await
            .unwrap();
        store
            .add_job_full(
                "agent-job",
                "Agent job",
                &schedule,
                false,
                None,
                3,
                7200,
                SessionMode::Isolated,
                None,
            )
            .await
            .unwrap();

        // list_jobs returns all
        let all = store.list_jobs().await.unwrap();
        assert_eq!(all.len(), 3);

        // list_jobs_for_user returns only that user's jobs
        let alice_jobs = store.list_jobs_for_user("alice").await.unwrap();
        assert_eq!(alice_jobs.len(), 1);
        assert_eq!(alice_jobs[0].name, "alice-job");
        assert_eq!(alice_jobs[0].user_id, Some("alice".to_string()));

        let bob_jobs = store.list_jobs_for_user("bob").await.unwrap();
        assert_eq!(bob_jobs.len(), 1);
        assert_eq!(bob_jobs[0].name, "bob-job");

        // Unknown user returns empty
        let none = store.list_jobs_for_user("nobody").await.unwrap();
        assert!(none.is_empty());
    }
}
