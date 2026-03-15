//! Cron scheduling system for Starpod.
//!
//! Provides persistent job scheduling with:
//! - **Three schedule types**: cron expressions, fixed intervals, and one-shot timestamps
//! - **Retry with exponential backoff**: transient failures are retried up to `max_retries`
//!   times with delays of 30s → 60s → 5m → 15m → 1h
//! - **Concurrency control**: global `max_concurrent_runs` limit prevents runaway execution
//! - **Stuck job detection**: runs exceeding `timeout_secs` are automatically reaped
//! - **Session targeting**: jobs can run in isolated sessions or the shared main session
//! - **Heartbeat system**: a reserved `__heartbeat__` job enables proactive agent behavior
//!
//! # Architecture
//!
//! ```text
//! ┌──────────┐     tick()      ┌───────────┐    executor(ctx)    ┌───────────┐
//! │ CronStore│◄───────────────►│ Scheduler │───────────────────►│   Agent   │
//! │ (SQLite) │  due/retry jobs │  (30s loop)│   JobContext       │  chat()   │
//! └──────────┘                 └───────────┘                    └───────────┘
//! ```
//!
//! The [`CronScheduler`] polls [`CronStore`] every `tick_interval_secs` for due jobs
//! and retry-eligible jobs. Each job is executed via a [`JobExecutor`] callback that
//! receives a [`JobContext`] containing the prompt, session mode, and job metadata.

mod schema;
pub mod scheduler;
pub mod store;
pub mod types;

pub use scheduler::{CronScheduler, JobExecutor, NotificationSender};
pub use store::CronStore;
pub use types::{CronJob, CronRun, JobContext, JobUpdate, RunStatus, Schedule, SessionMode};
