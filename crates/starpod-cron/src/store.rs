use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;
use tracing::warn;
use uuid::Uuid;

use starpod_core::{StarpodError, Result};

use crate::schema;
use crate::types::*;

/// Manages cron jobs in SQLite.
pub struct CronStore {
    pool: SqlitePool,
}

impl CronStore {
    /// Open or create the cron database.
    pub async fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let opts = SqliteConnectOptions::from_str(&format!(
            "sqlite://{}?mode=rwc",
            db_path.display()
        ))
        .map_err(|e| StarpodError::Database(format!("Invalid DB path: {}", e)))?;

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to open cron db: {}", e)))?;

        // Enable foreign keys
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to enable foreign keys: {}", e)))?;

        schema::run_migrations(&pool).await?;

        Ok(Self { pool })
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
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let (stype, svalue) = schedule_to_db(schedule);
        let next_run = compute_next_run(schedule, None, user_tz)?;
        let delete_flag = delete_after_run as i64;

        sqlx::query(
            "INSERT INTO cron_jobs (id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, next_run_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8)",
        )
        .bind(&id)
        .bind(name)
        .bind(prompt)
        .bind(stype)
        .bind(&svalue)
        .bind(delete_flag)
        .bind(now)
        .bind(next_run)
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
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at
             FROM cron_jobs ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to list jobs: {}", e)))?;

        Ok(rows.iter().map(job_from_row).collect())
    }

    /// Get jobs that are due for execution.
    pub async fn get_due_jobs(&self) -> Result<Vec<CronJob>> {
        let now = Utc::now().timestamp();

        let rows = sqlx::query(
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at
             FROM cron_jobs
             WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StarpodError::Cron(format!("Failed to query due jobs: {}", e)))?;

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
            "SELECT id, job_id, started_at, completed_at, status, result_summary
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
            })
            .collect())
    }
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
            if let Ok(dt) = DateTime::parse_from_rfc3339(at) {
                let dt_utc = dt.with_timezone(&Utc);
                if dt_utc > now && last_run.is_none() {
                    Ok(Some(dt_utc.timestamp()))
                } else {
                    Ok(None)
                }
            } else {
                Err(StarpodError::Cron(format!("Invalid timestamp: {}", at)))
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

            let cron_schedule = cron::Schedule::from_str(&effective_expr)
                .map_err(|e| StarpodError::Cron(format!("Invalid cron expression '{}': {}", expr, e)))?;

            let next_utc = match user_tz.and_then(|s| s.parse::<chrono_tz::Tz>().ok()) {
                Some(tz) => {
                    cron_schedule.upcoming(tz).next().map(|dt| dt.with_timezone(&Utc))
                }
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
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        schema::run_migrations(&pool).await.unwrap();
        CronStore { pool }
    }

    #[tokio::test]
    async fn test_add_and_list_jobs() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("check-deploy", "Check deploy status", &schedule, false, None)
            .await
            .unwrap();
        assert!(!id.is_empty());

        let jobs = store.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "check-deploy");
        assert_eq!(jobs[0].prompt, "Check deploy status");
        assert!(jobs[0].enabled);
        assert!(jobs[0].next_run_at.is_some());
    }

    #[tokio::test]
    async fn test_remove_job() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store.add_job("temp", "temp job", &schedule, false, None).await.unwrap();

        store.remove_job(&id).await.unwrap();
        assert_eq!(store.list_jobs().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_remove_by_name() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        store.add_job("my-job", "do stuff", &schedule, false, None).await.unwrap();

        store.remove_job_by_name("my-job").await.unwrap();
        assert_eq!(store.list_jobs().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_run_lifecycle() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let job_id = store.add_job("test-job", "test", &schedule, false, None).await.unwrap();

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
        assert!(next.is_some(), "5-field cron should be auto-expanded to 6 fields");
    }

    #[tokio::test]
    async fn test_compute_next_run_cron_with_timezone() {
        // "Every day at 23:00" — should differ between UTC and Europe/Rome
        let schedule = Schedule::Cron {
            expr: "0 0 23 * * *".to_string(),
        };
        let next_utc = compute_next_run(&schedule, None, None).unwrap().unwrap();
        let next_rome = compute_next_run(&schedule, None, Some("Europe/Rome")).unwrap().unwrap();
        // Rome is UTC+1 or UTC+2 (DST), so next_rome should differ
        assert_ne!(next_utc, next_rome, "Timezone should affect computed next_run");
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
    async fn test_disable_job() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store.add_job("j", "p", &schedule, false, None).await.unwrap();

        store.disable_job(&id).await.unwrap();

        let jobs = store.list_jobs().await.unwrap();
        assert!(!jobs[0].enabled);
    }
}
