use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::store::{backoff_delay, compute_next_run, CronStore};
use crate::types::{JobContext, RunStatus};

/// Result returned by a [`JobExecutor`] on success.
///
/// Contains both the session ID (for notification routing to the web UI)
/// and a short result summary (forwarded to Telegram and shown in toasts).
///
/// ```
/// use starpod_cron::JobResult;
///
/// let result = JobResult {
///     session_id: "sess-abc-123".into(),
///     summary: "Digest sent successfully".into(),
/// };
/// assert_eq!(result.session_id, "sess-abc-123");
/// ```
#[derive(Debug, Clone)]
pub struct JobResult {
    /// The session ID created/used by this job execution.
    ///
    /// Empty string when the job failed before a session was created.
    pub session_id: String,
    /// A short summary of the execution result (typically truncated to 500 chars).
    pub summary: String,
}

/// Callback type for executing a job.
///
/// Receives a [`JobContext`] with the prompt, session mode, and job metadata.
/// Returns [`JobResult`] on success (with session ID for notification routing)
/// or an error string on failure.
pub type JobExecutor = Arc<
    dyn Fn(JobContext) -> Pin<Box<dyn Future<Output = Result<JobResult, String>> + Send>>
        + Send
        + Sync,
>;

/// Callback type for sending notifications after a cron job completes.
///
/// Called with four arguments:
/// - `job_name`: the human-readable name of the job that ran
/// - `session_id`: the session created by the job (empty string on failure)
/// - `result_text`: a summary of the result (truncated output or error message)
/// - `success`: `true` if the job succeeded, `false` if it failed
///
/// The gateway composes this to broadcast to WebSocket clients and optionally
/// forward to Telegram.
pub type NotificationSender = Arc<
    dyn Fn(String, String, String, bool) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

/// Background scheduler that polls for due jobs and executes them.
///
/// Each `tick()` cycle performs these steps in order:
/// 1. **Reap stuck runs**: marks any run exceeding its job's `timeout_secs` as failed
/// 2. **Check concurrency**: counts total running jobs against `max_concurrent_runs`
/// 3. **Process due jobs**: fires jobs where `next_run_at <= now`, handling success/failure
/// 4. **Process retry jobs**: fires jobs where `retry_at <= now`
///
/// On failure, if `retry_count < max_retries`, the job is scheduled for retry with
/// exponential backoff. Otherwise it is marked permanently failed.
pub struct CronScheduler {
    store: Arc<CronStore>,
    executor: JobExecutor,
    notifier: Option<NotificationSender>,
    tick_interval_secs: u64,
    user_tz: Option<String>,
    max_concurrent_runs: u32,
}

impl CronScheduler {
    /// Create a new scheduler.
    ///
    /// `executor` is called with a `JobContext` when a job fires.
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
            max_concurrent_runs: 1,
        }
    }

    /// Set a notification callback that fires after each job completes.
    pub fn with_notifier(mut self, notifier: NotificationSender) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// Set the maximum number of concurrent job runs (builder style).
    pub fn with_max_concurrent_runs(mut self, max: u32) -> Self {
        self.max_concurrent_runs = max;
        self
    }

    /// Set the maximum number of concurrent job runs (setter style).
    pub fn set_max_concurrent_runs(&mut self, max: u32) {
        self.max_concurrent_runs = max;
    }

    /// Start the scheduler background loop. Returns a JoinHandle.
    pub fn start(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(
                tick_secs = self.tick_interval_secs,
                "Cron scheduler started"
            );

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(self.tick_interval_secs)).await;

                self.tick().await;
            }
        })
    }

    /// Single tick: reap stuck jobs, check concurrency, process due jobs + retries.
    async fn tick(&self) {
        // Phase 2: Reap stuck jobs first
        match self.store.timeout_stuck_runs().await {
            Ok(reaped) if reaped > 0 => {
                warn!(count = reaped, "Reaped stuck cron runs");
            }
            Err(e) => {
                error!(error = %e, "Failed to reap stuck runs");
            }
            _ => {}
        }

        // Phase 2: Check global concurrency limit
        let running = match self.store.count_all_running().await {
            Ok(n) => n,
            Err(e) => {
                error!(error = %e, "Failed to count running jobs");
                return;
            }
        };

        let available = self.max_concurrent_runs.saturating_sub(running);
        if available == 0 {
            return; // All slots occupied
        }

        // Process due jobs
        let due_jobs = match self.store.get_due_jobs().await {
            Ok(jobs) => jobs,
            Err(e) => {
                error!(error = %e, "Failed to query due jobs");
                return;
            }
        };

        // Process retry jobs
        let retry_jobs = match self.store.get_retry_jobs().await {
            Ok(jobs) => jobs,
            Err(e) => {
                error!(error = %e, "Failed to query retry jobs");
                Vec::new()
            }
        };

        if due_jobs.is_empty() && retry_jobs.is_empty() {
            return;
        }

        if !due_jobs.is_empty() {
            info!(count = due_jobs.len(), "Found due cron jobs");
        }
        if !retry_jobs.is_empty() {
            info!(count = retry_jobs.len(), "Found retry cron jobs");
        }

        let mut slots_used = 0u32;

        // Process due jobs first
        for job in &due_jobs {
            if slots_used >= available {
                break;
            }

            info!(job = %job.name, "Executing cron job");

            // Record run start BEFORE clearing next_run_at (so we don't lose the job if recording fails)
            let run_id = match self.store.record_run_start(&job.id).await {
                Ok(id) => id,
                Err(e) => {
                    error!(job = %job.name, error = %e, "Failed to record run start");
                    continue;
                }
            };

            // Dedup guard: clear next_run_at so the next tick won't pick this job up again
            if let Err(e) = self.store.update_next_run(&job.id, None).await {
                error!(job = %job.name, error = %e, "Failed to clear next_run_at");
                continue;
            }

            slots_used += 1;

            // Execute the job
            let ctx = JobContext {
                prompt: job.prompt.clone(),
                session_mode: job.session_mode.clone(),
                job_name: job.name.clone(),
                job_id: job.id.clone(),
                user_id: job.user_id.clone(),
            };
            let executor = Arc::clone(&self.executor);
            let result = (executor)(ctx).await;

            // Record completion and handle retry logic
            let (status, session_id, summary) = match result {
                Ok(jr) => {
                    info!(job = %job.name, "Cron job completed successfully");
                    // Phase 1: Reset retry state on success
                    let _ = self.store.reset_retry(&job.id).await;
                    (RunStatus::Success, jr.session_id, jr.summary)
                }
                Err(e) => {
                    warn!(job = %job.name, error = %e, "Cron job failed");
                    // Phase 1: Handle retry
                    if job.retry_count < job.max_retries {
                        let delay = backoff_delay(job.retry_count);
                        let retry_at = Utc::now().timestamp() + delay;
                        let _ = self.store.schedule_retry(&job.id, retry_at, &e).await;
                        info!(
                            job = %job.name,
                            retry = job.retry_count + 1,
                            max = job.max_retries,
                            delay_secs = delay,
                            "Scheduled retry"
                        );
                    } else {
                        let _ = self.store.mark_permanently_failed(&job.id, &e).await;
                        warn!(job = %job.name, "Max retries exceeded, permanently failed");
                    }
                    (RunStatus::Failed, String::new(), e)
                }
            };

            let _ = self
                .store
                .record_run_complete(&run_id, status.clone(), Some(&summary))
                .await;

            // Link run to session
            if !session_id.is_empty() {
                let _ = self.store.record_run_session(&run_id, &session_id).await;
            }

            // Send notification if configured
            if let Some(ref notifier) = self.notifier {
                let success = status == RunStatus::Success;
                (notifier)(
                    job.name.clone(),
                    session_id.clone(),
                    summary.clone(),
                    success,
                )
                .await;
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

        // Process retry jobs
        for job in &retry_jobs {
            if slots_used >= available {
                break;
            }

            info!(job = %job.name, retry = job.retry_count, "Retrying cron job");

            let run_id = match self.store.record_run_start(&job.id).await {
                Ok(id) => id,
                Err(e) => {
                    error!(job = %job.name, error = %e, "Failed to record retry run start");
                    continue;
                }
            };

            slots_used += 1;

            let ctx = JobContext {
                prompt: job.prompt.clone(),
                session_mode: job.session_mode.clone(),
                job_name: job.name.clone(),
                job_id: job.id.clone(),
                user_id: job.user_id.clone(),
            };
            let executor = Arc::clone(&self.executor);
            let result = (executor)(ctx).await;

            let (status, session_id, summary) = match result {
                Ok(jr) => {
                    info!(job = %job.name, "Retry succeeded");
                    let _ = self.store.reset_retry(&job.id).await;
                    (RunStatus::Success, jr.session_id, jr.summary)
                }
                Err(e) => {
                    warn!(job = %job.name, error = %e, "Retry failed");
                    if job.retry_count < job.max_retries {
                        let delay = backoff_delay(job.retry_count);
                        let retry_at = Utc::now().timestamp() + delay;
                        let _ = self.store.schedule_retry(&job.id, retry_at, &e).await;
                        info!(
                            job = %job.name,
                            retry = job.retry_count + 1,
                            max = job.max_retries,
                            delay_secs = delay,
                            "Scheduled next retry"
                        );
                    } else {
                        let _ = self.store.mark_permanently_failed(&job.id, &e).await;
                        warn!(job = %job.name, "Max retries exceeded, permanently failed");
                    }
                    (RunStatus::Failed, String::new(), e)
                }
            };

            let _ = self
                .store
                .record_run_complete(&run_id, status.clone(), Some(&summary))
                .await;

            // Link run to session
            if !session_id.is_empty() {
                let _ = self.store.record_run_session(&run_id, &session_id).await;
            }

            if let Some(ref notifier) = self.notifier {
                let success = status == RunStatus::Success;
                (notifier)(
                    job.name.clone(),
                    session_id.clone(),
                    summary.clone(),
                    success,
                )
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Schedule, SessionMode};
    use std::sync::atomic::{AtomicU32, Ordering};

    async fn setup() -> Arc<CronStore> {
        let db = starpod_db::CoreDb::in_memory().await.unwrap();
        Arc::new(CronStore::from_pool(db.pool().clone()))
    }

    fn success_executor() -> JobExecutor {
        Arc::new(|_ctx| {
            Box::pin(async {
                Ok(JobResult {
                    session_id: "test-session".into(),
                    summary: "done".into(),
                })
            })
        })
    }

    fn failing_executor() -> JobExecutor {
        Arc::new(|_ctx| Box::pin(async { Err("connection refused".to_string()) }))
    }

    fn counting_executor(counter: Arc<AtomicU32>) -> JobExecutor {
        Arc::new(move |_ctx| {
            let counter = Arc::clone(&counter);
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(JobResult {
                    session_id: "test-session".into(),
                    summary: "done".into(),
                })
            })
        })
    }

    #[tokio::test]
    async fn test_tick_executes_due_job() {
        let store = setup().await;
        let counter = Arc::new(AtomicU32::new(0));

        // Add a job due in the past
        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("tick-job", "test", &schedule, false, None)
            .await
            .unwrap();

        // Backdate next_run_at
        let jobs = store.list_jobs().await.unwrap();
        let past = Utc::now().timestamp() - 10;
        store
            .update_next_run(&jobs[0].id, Some(past))
            .await
            .unwrap();

        let scheduler = CronScheduler::new(
            Arc::clone(&store),
            counting_executor(Arc::clone(&counter)),
            60,
            None,
        );

        scheduler.tick().await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Run should be recorded
        let runs = store.list_runs(&jobs[0].id, 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Success);
    }

    #[tokio::test]
    async fn test_tick_skips_not_due() {
        let store = setup().await;
        let counter = Arc::new(AtomicU32::new(0));

        // Add a job with next_run in the future
        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("future-job", "test", &schedule, false, None)
            .await
            .unwrap();

        let scheduler = CronScheduler::new(
            Arc::clone(&store),
            counting_executor(Arc::clone(&counter)),
            60,
            None,
        );

        scheduler.tick().await;

        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_tick_failure_schedules_retry() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("fail-job", "test", &schedule, false, None)
            .await
            .unwrap();

        // Backdate next_run_at
        let jobs = store.list_jobs().await.unwrap();
        let past = Utc::now().timestamp() - 10;
        store
            .update_next_run(&jobs[0].id, Some(past))
            .await
            .unwrap();

        let scheduler = CronScheduler::new(Arc::clone(&store), failing_executor(), 60, None);

        scheduler.tick().await;

        // Job should have retry scheduled
        let job = store.get_job_by_name("fail-job").await.unwrap().unwrap();
        assert_eq!(job.retry_count, 1);
        assert!(job.retry_at.is_some());
        assert_eq!(job.last_error.as_deref(), Some("connection refused"));
    }

    #[tokio::test]
    async fn test_tick_processes_retry_jobs() {
        let store = setup().await;
        let counter = Arc::new(AtomicU32::new(0));

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("retry-exec", "test", &schedule, false, None)
            .await
            .unwrap();

        // Schedule a retry in the past
        let past = Utc::now().timestamp() - 10;
        store.schedule_retry(&id, past, "old error").await.unwrap();

        let scheduler = CronScheduler::new(
            Arc::clone(&store),
            counting_executor(Arc::clone(&counter)),
            60,
            None,
        );

        scheduler.tick().await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Retry state should be reset after success
        let job = store.get_job_by_name("retry-exec").await.unwrap().unwrap();
        assert_eq!(job.retry_count, 0);
        assert!(job.retry_at.is_none());
        assert!(job.last_error.is_none());
    }

    #[tokio::test]
    async fn test_tick_concurrency_limit() {
        let store = setup().await;
        let counter = Arc::new(AtomicU32::new(0));

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("job-1", "test1", &schedule, false, None)
            .await
            .unwrap();
        store
            .add_job("job-2", "test2", &schedule, false, None)
            .await
            .unwrap();

        // Backdate both to be due
        let past = Utc::now().timestamp() - 10;
        let jobs = store.list_jobs().await.unwrap();
        for j in &jobs {
            store.update_next_run(&j.id, Some(past)).await.unwrap();
        }

        // max_concurrent_runs = 1, so only 1 should execute
        let scheduler = CronScheduler::new(
            Arc::clone(&store),
            counting_executor(Arc::clone(&counter)),
            60,
            None,
        )
        .with_max_concurrent_runs(1);

        scheduler.tick().await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_tick_concurrency_allows_multiple() {
        let store = setup().await;
        let counter = Arc::new(AtomicU32::new(0));

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("multi-1", "test1", &schedule, false, None)
            .await
            .unwrap();
        store
            .add_job("multi-2", "test2", &schedule, false, None)
            .await
            .unwrap();

        let past = Utc::now().timestamp() - 10;
        let jobs = store.list_jobs().await.unwrap();
        for j in &jobs {
            store.update_next_run(&j.id, Some(past)).await.unwrap();
        }

        let scheduler = CronScheduler::new(
            Arc::clone(&store),
            counting_executor(Arc::clone(&counter)),
            60,
            None,
        )
        .with_max_concurrent_runs(5);

        scheduler.tick().await;

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_tick_max_retries_exhausted() {
        let store = setup().await;

        // Job with max_retries = 1
        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job_full(
                "exhaust-retry",
                "test",
                &schedule,
                false,
                None,
                1,
                7200,
                SessionMode::Isolated,
                None,
            )
            .await
            .unwrap();

        // First failure — should schedule retry
        let past = Utc::now().timestamp() - 10;
        store.update_next_run(&id, Some(past)).await.unwrap();

        let scheduler = CronScheduler::new(Arc::clone(&store), failing_executor(), 60, None);

        scheduler.tick().await;

        let job = store
            .get_job_by_name("exhaust-retry")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.retry_count, 1);
        assert!(job.retry_at.is_some());

        // Now process the retry — retry_count == max_retries, should be permanently failed
        let past = Utc::now().timestamp() - 10;
        store.schedule_retry(&id, past, "err").await.unwrap(); // re-backdate

        // Manually set retry_at to past (schedule_retry incremented count to 2, > max_retries=1)
        // Actually, let's just let the scheduler process the retry
        // The job already has retry_count=1 from the first failure + schedule_retry
        // Then we called schedule_retry again, making retry_count=2
        // Actually the flow is: first tick failed -> schedule_retry (retry_count=1)
        // Now we manually called schedule_retry again, retry_count=2
        // When the scheduler sees retry_count(2) >= max_retries(1), it marks permanently failed

        // Let's simplify: reset and do it cleanly
        store.reset_retry(&id).await.unwrap();

        // Simulate retry_count = max_retries
        let past = Utc::now().timestamp() - 10;
        store.schedule_retry(&id, past, "err").await.unwrap(); // retry_count = 1 == max_retries

        scheduler.tick().await;

        let job = store
            .get_job_by_name("exhaust-retry")
            .await
            .unwrap()
            .unwrap();
        // retry_count is now 1 (from the schedule_retry above), but since
        // retry_count (1) >= max_retries (1), the scheduler should mark permanently failed
        // The retry tick increments retry_count via schedule_retry in the failure path
        // But wait: the scheduler checks job.retry_count < job.max_retries BEFORE executing
        // The job has retry_count=1, max_retries=1 — so 1 < 1 is false, mark_permanently_failed is called
        assert!(
            job.retry_at.is_none(),
            "retry_at should be cleared after max retries exhausted"
        );
    }

    #[tokio::test]
    async fn test_tick_notification_sent() {
        let store = setup().await;
        let notified = Arc::new(AtomicU32::new(0));

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("notify-job", "test", &schedule, false, None)
            .await
            .unwrap();

        let past = Utc::now().timestamp() - 10;
        let jobs = store.list_jobs().await.unwrap();
        store
            .update_next_run(&jobs[0].id, Some(past))
            .await
            .unwrap();

        let notified_clone = Arc::clone(&notified);
        let notifier: crate::NotificationSender =
            Arc::new(move |_name, _session_id, _result, _success| {
                let n = Arc::clone(&notified_clone);
                Box::pin(async move {
                    n.fetch_add(1, Ordering::SeqCst);
                })
            });

        let scheduler = CronScheduler::new(Arc::clone(&store), success_executor(), 60, None)
            .with_notifier(notifier);

        scheduler.tick().await;

        assert_eq!(notified.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_tick_delete_after_run() {
        let store = setup().await;

        // Add a delete-after-run job
        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job_full(
                "one-shot",
                "test",
                &schedule,
                true,
                None,
                3,
                7200,
                SessionMode::Isolated,
                None,
            )
            .await
            .unwrap();

        let past = Utc::now().timestamp() - 10;
        let jobs = store.list_jobs().await.unwrap();
        store
            .update_next_run(&jobs[0].id, Some(past))
            .await
            .unwrap();

        let scheduler = CronScheduler::new(Arc::clone(&store), success_executor(), 60, None);

        scheduler.tick().await;

        // Job should be deleted
        assert!(store.list_jobs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_tick_passes_job_context() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job_full(
                "ctx-job",
                "hello world",
                &schedule,
                false,
                None,
                3,
                7200,
                SessionMode::Main,
                None,
            )
            .await
            .unwrap();

        let past = Utc::now().timestamp() - 10;
        let jobs = store.list_jobs().await.unwrap();
        store
            .update_next_run(&jobs[0].id, Some(past))
            .await
            .unwrap();

        let received_ctx = Arc::new(tokio::sync::Mutex::new(None::<JobContext>));
        let ctx_clone = Arc::clone(&received_ctx);

        let executor: JobExecutor = Arc::new(move |ctx| {
            let ctx_clone = Arc::clone(&ctx_clone);
            Box::pin(async move {
                *ctx_clone.lock().await = Some(ctx);
                Ok(JobResult {
                    session_id: "test-session".into(),
                    summary: "done".into(),
                })
            })
        });

        let scheduler = CronScheduler::new(Arc::clone(&store), executor, 60, None);

        scheduler.tick().await;

        let ctx = received_ctx.lock().await;
        let ctx = ctx.as_ref().expect("Executor should have been called");
        assert_eq!(ctx.prompt, "hello world");
        assert_eq!(ctx.session_mode, SessionMode::Main);
        assert_eq!(ctx.job_name, "ctx-job");
    }

    #[tokio::test]
    async fn test_notification_receives_session_id() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("session-notify", "test", &schedule, false, None)
            .await
            .unwrap();

        let past = Utc::now().timestamp() - 10;
        let jobs = store.list_jobs().await.unwrap();
        store
            .update_next_run(&jobs[0].id, Some(past))
            .await
            .unwrap();

        let received = Arc::new(tokio::sync::Mutex::new(
            None::<(String, String, String, bool)>,
        ));
        let received_clone = Arc::clone(&received);
        let notifier: crate::NotificationSender =
            Arc::new(move |name, session_id, result, success| {
                let r = Arc::clone(&received_clone);
                Box::pin(async move {
                    *r.lock().await = Some((name, session_id, result, success));
                })
            });

        // Executor returns a specific session_id
        let executor: JobExecutor = Arc::new(|_ctx| {
            Box::pin(async {
                Ok(JobResult {
                    session_id: "cron-session-42".into(),
                    summary: "all good".into(),
                })
            })
        });

        let scheduler =
            CronScheduler::new(Arc::clone(&store), executor, 60, None).with_notifier(notifier);

        scheduler.tick().await;

        let r = received.lock().await;
        let (name, session_id, result, success) =
            r.as_ref().expect("Notifier should have been called");
        assert_eq!(name, "session-notify");
        assert_eq!(session_id, "cron-session-42");
        assert_eq!(result, "all good");
        assert!(*success);
    }

    #[tokio::test]
    async fn test_notification_failure_has_empty_session_id() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        store
            .add_job("fail-notify", "test", &schedule, false, None)
            .await
            .unwrap();

        let past = Utc::now().timestamp() - 10;
        let jobs = store.list_jobs().await.unwrap();
        store
            .update_next_run(&jobs[0].id, Some(past))
            .await
            .unwrap();

        let received = Arc::new(tokio::sync::Mutex::new(None::<(String, String, bool)>));
        let received_clone = Arc::clone(&received);
        let notifier: crate::NotificationSender =
            Arc::new(move |_name, session_id, _result, success| {
                let r = Arc::clone(&received_clone);
                Box::pin(async move {
                    *r.lock().await = Some((session_id, _result, success));
                })
            });

        let scheduler = CronScheduler::new(Arc::clone(&store), failing_executor(), 60, None)
            .with_notifier(notifier);

        scheduler.tick().await;

        let r = received.lock().await;
        let (session_id, result, success) = r
            .as_ref()
            .expect("Notifier should have been called on failure");
        assert!(
            session_id.is_empty(),
            "Failed jobs should have empty session_id"
        );
        assert_eq!(result, "connection refused");
        assert!(!success);
    }

    #[tokio::test]
    async fn test_notification_on_retry_success() {
        let store = setup().await;

        let schedule = Schedule::Interval { every_ms: 60000 };
        let id = store
            .add_job("retry-notify", "test", &schedule, false, None)
            .await
            .unwrap();

        // Schedule a retry in the past
        let past = Utc::now().timestamp() - 10;
        store.schedule_retry(&id, past, "old error").await.unwrap();

        let received = Arc::new(tokio::sync::Mutex::new(None::<(String, String, bool)>));
        let received_clone = Arc::clone(&received);
        let notifier: crate::NotificationSender =
            Arc::new(move |_name, session_id, _result, success| {
                let r = Arc::clone(&received_clone);
                Box::pin(async move {
                    *r.lock().await = Some((session_id, _result, success));
                })
            });

        let scheduler = CronScheduler::new(Arc::clone(&store), success_executor(), 60, None)
            .with_notifier(notifier);

        scheduler.tick().await;

        let r = received.lock().await;
        let (session_id, _, success) = r.as_ref().expect("Notifier should fire on retry success");
        assert_eq!(session_id, "test-session");
        assert!(*success);
    }
}
