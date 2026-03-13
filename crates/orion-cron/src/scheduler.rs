use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::store::{compute_next_run, CronStore};
use crate::types::RunStatus;

/// Callback type for executing a job prompt. Returns a result summary string.
pub type JobExecutor =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync>;

/// Background scheduler that polls for due jobs and executes them.
pub struct CronScheduler {
    store: Arc<CronStore>,
    executor: JobExecutor,
    tick_interval_secs: u64,
}

impl CronScheduler {
    /// Create a new scheduler.
    ///
    /// `executor` is called with the job's prompt string when a job fires.
    /// `tick_interval_secs` controls how often the scheduler checks for due jobs.
    pub fn new(store: Arc<CronStore>, executor: JobExecutor, tick_interval_secs: u64) -> Self {
        Self {
            store,
            executor,
            tick_interval_secs,
        }
    }

    /// Start the scheduler background loop. Returns a JoinHandle.
    pub fn start(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(tick_secs = self.tick_interval_secs, "Cron scheduler started");

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(self.tick_interval_secs))
                    .await;

                self.tick().await;
            }
        })
    }

    /// Single tick: check for due jobs and execute them.
    async fn tick(&self) {
        let due_jobs = match self.store.get_due_jobs() {
            Ok(jobs) => jobs,
            Err(e) => {
                error!(error = %e, "Failed to query due jobs");
                return;
            }
        };

        if due_jobs.is_empty() {
            debug!("No due cron jobs");
            return;
        }

        for job in due_jobs {
            debug!(job = %job.name, "Executing cron job");

            // Record run start
            let run_id = match self.store.record_run_start(&job.id) {
                Ok(id) => id,
                Err(e) => {
                    error!(job = %job.name, error = %e, "Failed to record run start");
                    continue;
                }
            };

            // Execute the job
            let executor = Arc::clone(&self.executor);
            let prompt = job.prompt.clone();
            let result = (executor)(prompt).await;

            // Record completion
            let (status, summary) = match result {
                Ok(s) => (RunStatus::Success, s),
                Err(e) => {
                    warn!(job = %job.name, error = %e, "Cron job failed");
                    (RunStatus::Failed, e)
                }
            };

            let _ = self
                .store
                .record_run_complete(&run_id, status, Some(&summary));

            // Handle one-shot / delete-after-run
            if job.delete_after_run {
                debug!(job = %job.name, "Removing one-shot job after execution");
                let _ = self.store.remove_job(&job.id);
                continue;
            }

            // Compute and set next run time
            let last_run = Some(Utc::now());
            match compute_next_run(&job.schedule, last_run) {
                Ok(Some(next)) => {
                    let _ = self.store.update_next_run(&job.id, Some(&next));
                }
                Ok(None) => {
                    // No more runs (e.g., one-shot already fired)
                    let _ = self.store.disable_job(&job.id);
                }
                Err(e) => {
                    error!(job = %job.name, error = %e, "Failed to compute next run");
                    let _ = self.store.disable_job(&job.id);
                }
            }
        }
    }
}
