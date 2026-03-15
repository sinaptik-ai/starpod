use serde::{Deserialize, Serialize};

/// Schedule type for a cron job.
///
/// Supports three scheduling strategies:
/// - **OneShot**: fire once at a specific ISO 8601 timestamp
/// - **Interval**: fire at fixed millisecond intervals
/// - **Cron**: fire on a cron expression (5-field standard auto-expanded to 6-field)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Schedule {
    /// Run once at a specific timestamp (ISO 8601 with optional timezone offset; normalized to UTC).
    #[serde(rename = "one_shot")]
    OneShot { at: String },
    /// Run at fixed intervals (milliseconds).
    #[serde(rename = "interval")]
    Interval { every_ms: u64 },
    /// Cron expression (5-field standard or 6-field with seconds).
    #[serde(rename = "cron")]
    Cron { expr: String },
}

/// Session mode for cron job execution.
///
/// Controls how the scheduler routes the job's prompt through the agent:
/// - `Isolated`: each run gets its own throwaway session (`channel_id="scheduler"`)
/// - `Main`: runs share the persistent main session (`channel_id="main"`, `key="main"`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    /// Each run gets its own isolated session (default).
    Isolated,
    /// Runs execute in the main session context.
    Main,
}

impl SessionMode {
    /// Convert to a string representation for storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionMode::Isolated => "isolated",
            SessionMode::Main => "main",
        }
    }

    /// Parse from a string, defaulting to `Isolated` for unknown values.
    pub fn from_str(s: &str) -> Self {
        match s {
            "main" => SessionMode::Main,
            _ => SessionMode::Isolated,
        }
    }
}

/// A scheduled job stored in the cron database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    /// Unique job identifier (UUID).
    pub id: String,
    /// Human-readable job name (unique constraint in DB).
    pub name: String,
    /// The prompt/message sent to the agent when the job fires.
    pub prompt: String,
    /// Schedule configuration (cron expression, interval, or one-shot).
    pub schedule: Schedule,
    /// Whether the job is active and will be picked up by the scheduler.
    pub enabled: bool,
    /// If true, the job is deleted after its first execution.
    pub delete_after_run: bool,
    /// Unix epoch timestamp when the job was created.
    pub created_at: i64,
    /// Unix epoch timestamp of the most recent execution start, if any.
    pub last_run_at: Option<i64>,
    /// Unix epoch timestamp when the job should next fire. `None` means disabled/completed.
    pub next_run_at: Option<i64>,
    /// Current retry attempt count (0 = no retries pending).
    pub retry_count: u32,
    /// Maximum number of retry attempts before the job is marked permanently failed.
    pub max_retries: u32,
    /// Error message from the most recent failure, if any.
    pub last_error: Option<String>,
    /// Unix epoch timestamp for the next retry attempt. `None` when no retry is pending.
    pub retry_at: Option<i64>,
    /// Maximum seconds a run can be in "running" status before the stuck-job reaper marks it failed.
    pub timeout_secs: u32,
    /// Controls whether the job runs in an isolated or shared main session.
    pub session_mode: SessionMode,
}

/// Context passed to the [`JobExecutor`](crate::scheduler::JobExecutor) callback.
///
/// Contains everything the executor needs to route and execute the job's prompt.
#[derive(Debug, Clone)]
pub struct JobContext {
    /// The prompt/message to send to the agent.
    pub prompt: String,
    /// Session routing mode — isolated vs. main.
    pub session_mode: SessionMode,
    /// Human-readable job name (for logging and special-case handling like heartbeat).
    pub job_name: String,
    /// Unique job ID (for correlating runs).
    pub job_id: String,
}

/// Partial update for a cron job — only non-`None` fields are applied.
///
/// Used by [`CronStore::update_job`](crate::store::CronStore::update_job) to
/// dynamically build an `UPDATE` statement with only the provided fields.
#[derive(Debug, Default)]
pub struct JobUpdate {
    /// New prompt text.
    pub prompt: Option<String>,
    /// New schedule configuration.
    pub schedule: Option<Schedule>,
    /// Enable or disable the job.
    pub enabled: Option<bool>,
    /// New maximum retry count.
    pub max_retries: Option<u32>,
    /// New timeout in seconds.
    pub timeout_secs: Option<u32>,
    /// New session mode.
    pub session_mode: Option<SessionMode>,
}

/// Record of a single job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRun {
    /// Unique run identifier (UUID).
    pub id: String,
    /// ID of the job that was executed.
    pub job_id: String,
    /// Unix epoch timestamp when the run started.
    pub started_at: i64,
    /// Unix epoch timestamp when the run completed, if finished.
    pub completed_at: Option<i64>,
    /// Current status of the run.
    pub status: RunStatus,
    /// Summary text from the execution result.
    pub result_summary: Option<String>,
}

/// Status of a cron run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Pending,
    Running,
    Success,
    Failed,
}

impl RunStatus {
    /// Convert to a string representation for storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
        }
    }

    /// Parse from a string, defaulting to `Pending` for unknown values.
    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => RunStatus::Running,
            "success" => RunStatus::Success,
            "failed" => RunStatus::Failed,
            _ => RunStatus::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_mode_roundtrip() {
        assert_eq!(SessionMode::from_str("isolated"), SessionMode::Isolated);
        assert_eq!(SessionMode::from_str("main"), SessionMode::Main);
        assert_eq!(SessionMode::from_str("unknown"), SessionMode::Isolated);
        assert_eq!(SessionMode::Isolated.as_str(), "isolated");
        assert_eq!(SessionMode::Main.as_str(), "main");
    }

    #[test]
    fn test_run_status_roundtrip() {
        for status in &[RunStatus::Pending, RunStatus::Running, RunStatus::Success, RunStatus::Failed] {
            assert_eq!(&RunStatus::from_str(status.as_str()), status);
        }
        assert_eq!(RunStatus::from_str("garbage"), RunStatus::Pending);
    }

    #[test]
    fn test_job_context_construction() {
        let ctx = JobContext {
            prompt: "Do something".into(),
            session_mode: SessionMode::Main,
            job_name: "my-job".into(),
            job_id: "abc-123".into(),
        };
        assert_eq!(ctx.prompt, "Do something");
        assert_eq!(ctx.session_mode, SessionMode::Main);
    }

    #[test]
    fn test_job_update_default() {
        let update = JobUpdate::default();
        assert!(update.prompt.is_none());
        assert!(update.schedule.is_none());
        assert!(update.enabled.is_none());
        assert!(update.max_retries.is_none());
        assert!(update.timeout_secs.is_none());
        assert!(update.session_mode.is_none());
    }

    #[test]
    fn test_schedule_serde_roundtrip() {
        let cases = vec![
            serde_json::json!({"kind": "interval", "every_ms": 5000}),
            serde_json::json!({"kind": "cron", "expr": "0 0 9 * * *"}),
            serde_json::json!({"kind": "one_shot", "at": "2030-01-01T00:00:00Z"}),
        ];
        for val in cases {
            let schedule: Schedule = serde_json::from_value(val.clone()).unwrap();
            let back = serde_json::to_value(&schedule).unwrap();
            assert_eq!(val, back);
        }
    }

    #[test]
    fn test_session_mode_serde() {
        let json = serde_json::json!("main");
        let mode: SessionMode = serde_json::from_value(json).unwrap();
        assert_eq!(mode, SessionMode::Main);

        let json = serde_json::json!("isolated");
        let mode: SessionMode = serde_json::from_value(json).unwrap();
        assert_eq!(mode, SessionMode::Isolated);
    }
}
