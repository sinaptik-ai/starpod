use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;
use uuid::Uuid;

use orion_core::{OrionError, Result};

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
        .map_err(|e| OrionError::Database(format!("Invalid DB path: {}", e)))?;

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(|e| OrionError::Database(format!("Failed to open cron db: {}", e)))?;

        // Enable foreign keys
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .map_err(|e| OrionError::Database(format!("Failed to enable foreign keys: {}", e)))?;

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
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let (stype, svalue) = schedule_to_db(schedule);
        let next_run = compute_next_run(schedule, None)?;
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
        .bind(&now)
        .bind(&next_run)
        .execute(&self.pool)
        .await
        .map_err(|e| OrionError::Cron(format!("Failed to add job: {}", e)))?;

        Ok(id)
    }

    /// Remove a job by ID.
    pub async fn remove_job(&self, id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM cron_jobs WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| OrionError::Cron(format!("Failed to remove job: {}", e)))?;

        if result.rows_affected() == 0 {
            return Err(OrionError::Cron(format!("Job '{}' not found", id)));
        }
        Ok(())
    }

    /// Remove a job by name.
    pub async fn remove_job_by_name(&self, name: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM cron_jobs WHERE name = ?1")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| OrionError::Cron(format!("Failed to remove job: {}", e)))?;

        if result.rows_affected() == 0 {
            return Err(OrionError::Cron(format!("Job '{}' not found", name)));
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
        .map_err(|e| OrionError::Cron(format!("Failed to list jobs: {}", e)))?;

        Ok(rows.iter().map(job_from_row).collect())
    }

    /// Get jobs that are due for execution.
    pub async fn get_due_jobs(&self) -> Result<Vec<CronJob>> {
        let now = Utc::now().to_rfc3339();

        let rows = sqlx::query(
            "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at
             FROM cron_jobs
             WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1",
        )
        .bind(&now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| OrionError::Cron(format!("Failed to query due jobs: {}", e)))?;

        Ok(rows.iter().map(job_from_row).collect())
    }

    /// Record that a job started running. Returns the run ID.
    pub async fn record_run_start(&self, job_id: &str) -> Result<String> {
        let run_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO cron_runs (id, job_id, started_at, status) VALUES (?1, ?2, ?3, 'running')",
        )
        .bind(&run_id)
        .bind(job_id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| OrionError::Cron(format!("Failed to record run start: {}", e)))?;

        sqlx::query("UPDATE cron_jobs SET last_run_at = ?2 WHERE id = ?1")
            .bind(job_id)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(|e| OrionError::Cron(format!("Failed to update last_run_at: {}", e)))?;

        Ok(run_id)
    }

    /// Record that a run completed.
    pub async fn record_run_complete(
        &self,
        run_id: &str,
        status: RunStatus,
        summary: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE cron_runs SET completed_at = ?2, status = ?3, result_summary = ?4 WHERE id = ?1",
        )
        .bind(run_id)
        .bind(&now)
        .bind(status.as_str())
        .bind(summary)
        .execute(&self.pool)
        .await
        .map_err(|e| OrionError::Cron(format!("Failed to record run complete: {}", e)))?;
        Ok(())
    }

    /// Update the next_run_at for a job, or disable it.
    pub async fn update_next_run(&self, job_id: &str, next: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE cron_jobs SET next_run_at = ?2 WHERE id = ?1")
            .bind(job_id)
            .bind(next)
            .execute(&self.pool)
            .await
            .map_err(|e| OrionError::Cron(format!("Failed to update next_run: {}", e)))?;
        Ok(())
    }

    /// Disable a job (set enabled = 0).
    pub async fn disable_job(&self, job_id: &str) -> Result<()> {
        sqlx::query("UPDATE cron_jobs SET enabled = 0 WHERE id = ?1")
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(|e| OrionError::Cron(format!("Failed to disable job: {}", e)))?;
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
        .map_err(|e| OrionError::Cron(format!("Failed to list runs: {}", e)))?;

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

/// Compute the next run time for a schedule.
pub fn compute_next_run(
    schedule: &Schedule,
    last_run: Option<DateTime<Utc>>,
) -> Result<Option<String>> {
    let now = Utc::now();
    match schedule {
        Schedule::OneShot { at } => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(at) {
                let dt = dt.with_timezone(&Utc);
                if dt > now && last_run.is_none() {
                    Ok(Some(at.clone()))
                } else {
                    Ok(None)
                }
            } else {
                Err(OrionError::Cron(format!("Invalid timestamp: {}", at)))
            }
        }
        Schedule::Interval { every_ms } => {
            let base = last_run.unwrap_or(now);
            let next = base + Duration::milliseconds(*every_ms as i64);
            Ok(Some(next.to_rfc3339()))
        }
        Schedule::Cron { expr } => {
            let schedule = cron::Schedule::from_str(expr)
                .map_err(|e| OrionError::Cron(format!("Invalid cron expression '{}': {}", expr, e)))?;
            match schedule.upcoming(Utc).next() {
                Some(next) => Ok(Some(next.to_rfc3339())),
                None => Ok(None),
            }
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
            .add_job("check-deploy", "Check deploy status", &schedule, false)
            .await
            .unwrap();
        assert!(!id.is_empty());

        let jobs = store.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "check-deploy");
        assert_eq!(jobs[0].prompt, "Check deploy status");
        assert!(jobs[0].enabled);
    }

    #[tokio::test]
    async fn test_remove_job() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store.add_job("temp", "temp job", &schedule, false).await.unwrap();

        store.remove_job(&id).await.unwrap();
        assert_eq!(store.list_jobs().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_remove_by_name() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        store.add_job("my-job", "do stuff", &schedule, false).await.unwrap();

        store.remove_job_by_name("my-job").await.unwrap();
        assert_eq!(store.list_jobs().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_run_lifecycle() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let job_id = store.add_job("test-job", "test", &schedule, false).await.unwrap();

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
        let next = compute_next_run(&schedule, None).unwrap();
        assert!(next.is_some());
    }

    #[tokio::test]
    async fn test_compute_next_run_cron() {
        let schedule = Schedule::Cron {
            expr: "0 0 * * * *".to_string(),
        };
        let next = compute_next_run(&schedule, None).unwrap();
        assert!(next.is_some());
    }

    #[tokio::test]
    async fn test_disable_job() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store.add_job("j", "p", &schedule, false).await.unwrap();

        store.disable_job(&id).await.unwrap();

        let jobs = store.list_jobs().await.unwrap();
        assert!(!jobs[0].enabled);
    }
}
