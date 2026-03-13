use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use uuid::Uuid;

use orion_core::{OrionError, Result};

use crate::schema;
use std::str::FromStr;

use crate::types::*;

/// Manages cron jobs in SQLite.
pub struct CronStore {
    conn: Mutex<Connection>,
}

impl CronStore {
    /// Open or create the cron database.
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)
            .map_err(|e| OrionError::Database(format!("Failed to open cron db: {}", e)))?;

        schema::migrate(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("cron db mutex poisoned")
    }

    /// Add a new cron job. Returns the job ID.
    pub fn add_job(
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

        self.db()
            .execute(
                "INSERT INTO cron_jobs (id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, next_run_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8)",
                rusqlite::params![id, name, prompt, stype, svalue, delete_after_run as i64, now, next_run],
            )
            .map_err(|e| OrionError::Cron(format!("Failed to add job: {}", e)))?;

        Ok(id)
    }

    /// Remove a job by ID.
    pub fn remove_job(&self, id: &str) -> Result<()> {
        let deleted = self
            .db()
            .execute("DELETE FROM cron_jobs WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| OrionError::Cron(format!("Failed to remove job: {}", e)))?;

        if deleted == 0 {
            return Err(OrionError::Cron(format!("Job '{}' not found", id)));
        }
        Ok(())
    }

    /// Remove a job by name.
    pub fn remove_job_by_name(&self, name: &str) -> Result<()> {
        let deleted = self
            .db()
            .execute(
                "DELETE FROM cron_jobs WHERE name = ?1",
                rusqlite::params![name],
            )
            .map_err(|e| OrionError::Cron(format!("Failed to remove job: {}", e)))?;

        if deleted == 0 {
            return Err(OrionError::Cron(format!("Job '{}' not found", name)));
        }
        Ok(())
    }

    /// List all jobs.
    pub fn list_jobs(&self) -> Result<Vec<CronJob>> {
        let conn = self.db();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at
                 FROM cron_jobs ORDER BY name",
            )
            .map_err(|e| OrionError::Cron(format!("Failed to list jobs: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(job_from_row(row))
            })
            .map_err(|e| OrionError::Cron(format!("Failed to query jobs: {}", e)))?;

        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(
                row.map_err(|e| OrionError::Cron(format!("Row read failed: {}", e)))?,
            );
        }
        Ok(jobs)
    }

    /// Get jobs that are due for execution.
    pub fn get_due_jobs(&self) -> Result<Vec<CronJob>> {
        let now = Utc::now().to_rfc3339();
        let conn = self.db();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at
                 FROM cron_jobs
                 WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1",
            )
            .map_err(|e| OrionError::Cron(format!("Failed to query due jobs: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![now], |row| Ok(job_from_row(row)))
            .map_err(|e| OrionError::Cron(format!("Failed to query due jobs: {}", e)))?;

        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(
                row.map_err(|e| OrionError::Cron(format!("Row read failed: {}", e)))?,
            );
        }
        Ok(jobs)
    }

    /// Record that a job started running. Returns the run ID.
    pub fn record_run_start(&self, job_id: &str) -> Result<String> {
        let run_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        let conn = self.db();
        conn.execute(
            "INSERT INTO cron_runs (id, job_id, started_at, status) VALUES (?1, ?2, ?3, 'running')",
            rusqlite::params![run_id, job_id, now],
        )
        .map_err(|e| OrionError::Cron(format!("Failed to record run start: {}", e)))?;

        conn.execute(
            "UPDATE cron_jobs SET last_run_at = ?2 WHERE id = ?1",
            rusqlite::params![job_id, now],
        )
        .map_err(|e| OrionError::Cron(format!("Failed to update last_run_at: {}", e)))?;

        Ok(run_id)
    }

    /// Record that a run completed.
    pub fn record_run_complete(
        &self,
        run_id: &str,
        status: RunStatus,
        summary: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db()
            .execute(
                "UPDATE cron_runs SET completed_at = ?2, status = ?3, result_summary = ?4 WHERE id = ?1",
                rusqlite::params![run_id, now, status.as_str(), summary],
            )
            .map_err(|e| OrionError::Cron(format!("Failed to record run complete: {}", e)))?;
        Ok(())
    }

    /// Update the next_run_at for a job, or disable it.
    pub fn update_next_run(&self, job_id: &str, next: Option<&str>) -> Result<()> {
        self.db()
            .execute(
                "UPDATE cron_jobs SET next_run_at = ?2 WHERE id = ?1",
                rusqlite::params![job_id, next],
            )
            .map_err(|e| OrionError::Cron(format!("Failed to update next_run: {}", e)))?;
        Ok(())
    }

    /// Disable a job (set enabled = 0).
    pub fn disable_job(&self, job_id: &str) -> Result<()> {
        self.db()
            .execute(
                "UPDATE cron_jobs SET enabled = 0 WHERE id = ?1",
                rusqlite::params![job_id],
            )
            .map_err(|e| OrionError::Cron(format!("Failed to disable job: {}", e)))?;
        Ok(())
    }

    /// List recent runs for a job.
    pub fn list_runs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRun>> {
        let conn = self.db();
        let mut stmt = conn
            .prepare(
                "SELECT id, job_id, started_at, completed_at, status, result_summary
                 FROM cron_runs WHERE job_id = ?1
                 ORDER BY started_at DESC LIMIT ?2",
            )
            .map_err(|e| OrionError::Cron(format!("Failed to list runs: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![job_id, limit as i64], |row| {
                Ok(CronRun {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    started_at: row.get(2)?,
                    completed_at: row.get(3)?,
                    status: RunStatus::from_str(&row.get::<_, String>(4)?),
                    result_summary: row.get(5)?,
                })
            })
            .map_err(|e| OrionError::Cron(format!("Failed to query runs: {}", e)))?;

        let mut runs = Vec::new();
        for row in rows {
            runs.push(row.map_err(|e| OrionError::Cron(format!("Row read failed: {}", e)))?);
        }
        Ok(runs)
    }
}

fn job_from_row(row: &rusqlite::Row<'_>) -> CronJob {
    let stype: String = row.get(3).unwrap_or_default();
    let svalue: String = row.get(4).unwrap_or_default();
    CronJob {
        id: row.get(0).unwrap_or_default(),
        name: row.get(1).unwrap_or_default(),
        prompt: row.get(2).unwrap_or_default(),
        schedule: schedule_from_db(&stype, &svalue),
        enabled: row.get::<_, i64>(5).unwrap_or(1) != 0,
        delete_after_run: row.get::<_, i64>(6).unwrap_or(0) != 0,
        created_at: row.get(7).unwrap_or_default(),
        last_run_at: row.get(8).unwrap_or_default(),
        next_run_at: row.get(9).unwrap_or_default(),
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
            // If the time hasn't passed yet, schedule it
            if let Ok(dt) = DateTime::parse_from_rfc3339(at) {
                let dt = dt.with_timezone(&Utc);
                if dt > now && last_run.is_none() {
                    Ok(Some(at.clone()))
                } else {
                    Ok(None) // Already fired or past
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
    use tempfile::TempDir;

    fn setup() -> (TempDir, CronStore) {
        let tmp = TempDir::new().unwrap();
        let store = CronStore::new(&tmp.path().join("cron.db")).unwrap();
        (tmp, store)
    }

    #[test]
    fn test_add_and_list_jobs() {
        let (_tmp, store) = setup();

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("check-deploy", "Check deploy status", &schedule, false)
            .unwrap();
        assert!(!id.is_empty());

        let jobs = store.list_jobs().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "check-deploy");
        assert_eq!(jobs[0].prompt, "Check deploy status");
        assert!(jobs[0].enabled);
    }

    #[test]
    fn test_remove_job() {
        let (_tmp, store) = setup();

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store.add_job("temp", "temp job", &schedule, false).unwrap();

        store.remove_job(&id).unwrap();
        assert_eq!(store.list_jobs().unwrap().len(), 0);
    }

    #[test]
    fn test_remove_by_name() {
        let (_tmp, store) = setup();

        let schedule = Schedule::Interval { every_ms: 60000 };
        store.add_job("my-job", "do stuff", &schedule, false).unwrap();

        store.remove_job_by_name("my-job").unwrap();
        assert_eq!(store.list_jobs().unwrap().len(), 0);
    }

    #[test]
    fn test_run_lifecycle() {
        let (_tmp, store) = setup();

        let schedule = Schedule::Interval { every_ms: 60000 };
        let job_id = store.add_job("test-job", "test", &schedule, false).unwrap();

        let run_id = store.record_run_start(&job_id).unwrap();

        let runs = store.list_runs(&job_id, 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Running);

        store
            .record_run_complete(&run_id, RunStatus::Success, Some("All good"))
            .unwrap();

        let runs = store.list_runs(&job_id, 10).unwrap();
        assert_eq!(runs[0].status, RunStatus::Success);
        assert_eq!(runs[0].result_summary.as_deref(), Some("All good"));
    }

    #[test]
    fn test_compute_next_run_interval() {
        let schedule = Schedule::Interval { every_ms: 300000 };
        let next = compute_next_run(&schedule, None).unwrap();
        assert!(next.is_some());
    }

    #[test]
    fn test_compute_next_run_cron() {
        let schedule = Schedule::Cron {
            expr: "0 0 * * * *".to_string(), // every hour
        };
        let next = compute_next_run(&schedule, None).unwrap();
        assert!(next.is_some());
    }

    #[test]
    fn test_disable_job() {
        let (_tmp, store) = setup();

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store.add_job("j", "p", &schedule, false).unwrap();

        store.disable_job(&id).unwrap();

        let jobs = store.list_jobs().unwrap();
        assert!(!jobs[0].enabled);
    }
}
