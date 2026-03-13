use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::store::{compute_next_run, CronStore};
use crate::types::RunStatus;

/// Callback type for executing a job prompt. Returns a result summary string.
pub type JobExecutor =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync>;

/// Callback type for sending notifications after a cron job completes.
/// Receives (job_name, result_text, success).
pub type NotificationSender =
    Arc<dyn Fn(String, String, bool) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Background scheduler that polls for due jobs and executes them.
pub struct CronScheduler {
    store: Arc<CronStore>,
    executor: JobExecutor,
    notifier: Option<NotificationSender>,
    tick_interval_secs: u64,
    user_tz: Option<String>,
}

impl CronScheduler {
    /// Create a new scheduler.
    ///
    /// `executor` is called with the job's prompt string when a job fires.
    /// `tick_interval_secs` controls how often the scheduler checks for due jobs.
    /// `user_tz` is an optional IANA timezone for evaluating cron expressions.
    pub fn new(
        store: Arc<CronStore>,
        executor: JobExecutor,
        tick_interval_secs: u64,
        user_tz: Option<String>,
    ) -> Self {
        Self {
            store,
            executor,
            notifier: None,
            tick_interval_secs,
            user_tz,
        }
    }

    /// Set a notification callback that fires after each job completes.
    pub fn with_notifier(mut self, notifier: NotificationSender) -> Self {
        self.notifier = Some(notifier);
        self
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
        let due_jobs = match self.store.get_due_jobs().await {
            Ok(jobs) => jobs,
            Err(e) => {
                error!(error = %e, "Failed to query due jobs");
                return;
            }
        };

        if due_jobs.is_empty() {
            return;
        }

        info!(count = due_jobs.len(), "Found due cron jobs");

        for job in due_jobs {
            info!(job = %job.name, "Executing cron job");

            // Dedup guard: clear next_run_at so the next tick won't pick this job up again
            if let Err(e) = self.store.update_next_run(&job.id, None).await {
                error!(job = %job.name, error = %e, "Failed to clear next_run_at");
                continue;
            }

            // Record run start
            let run_id = match self.store.record_run_start(&job.id).await {
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
                Ok(s) => {
                    info!(job = %job.name, "Cron job completed successfully");
                    (RunStatus::Success, s)
                }
                Err(e) => {
                    warn!(job = %job.name, error = %e, "Cron job failed");
                    (RunStatus::Failed, e)
                }
            };

            let _ = self
                .store
                .record_run_complete(&run_id, status.clone(), Some(&summary))
                .await;

            // Send notification if configured
            if let Some(ref notifier) = self.notifier {
                let success = status == RunStatus::Success;
                (notifier)(job.name.clone(), summary.clone(), success).await;
            }

            // Handle one-shot / delete-after-run
            if job.delete_after_run {
                info!(job = %job.name, "Removing one-shot job after execution");
                let _ = self.store.remove_job(&job.id).await;
                continue;
            }

            // Compute and set next run time
            let last_run = Some(Utc::now());
            match compute_next_run(&job.schedule, last_run, self.user_tz.as_deref()) {
                Ok(Some(next)) => {
                    let _ = self.store.update_next_run(&job.id, Some(next)).await;
                }
                Ok(None) => {
                    let _ = self.store.disable_job(&job.id).await;
                }
                Err(e) => {
                    error!(job = %job.name, error = %e, "Failed to compute next run");
                    let _ = self.store.disable_job(&job.id).await;
                }
            }
        }
    }
}
