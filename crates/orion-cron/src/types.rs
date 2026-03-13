use serde::{Deserialize, Serialize};

/// Schedule type for a cron job.
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

/// A scheduled job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub schedule: Schedule,
    pub enabled: bool,
    pub delete_after_run: bool,
    pub created_at: i64,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
}

/// Record of a single job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRun {
    pub id: String,
    pub job_id: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub status: RunStatus,
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
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => RunStatus::Running,
            "success" => RunStatus::Success,
            "failed" => RunStatus::Failed,
            _ => RunStatus::Pending,
        }
    }
}
