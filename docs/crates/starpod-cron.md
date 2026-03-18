# starpod-cron

Scheduling system supporting intervals, cron expressions, and one-shot schedules.

## CronStore API

```rust
let store = CronStore::new(&db_path).await?;

// Create a job
let id = store.add_job(
    "morning-check",
    "Summarize overnight alerts",
    &Schedule::Cron { expr: "0 0 9 * * *".into() },
    false,                    // delete_after_run
    Some("America/New_York"), // user timezone
).await?;

// Create with full options (retry, timeout, session mode, user)
let id = store.add_job_full(
    "morning-check", "Summarize overnight alerts",
    &Schedule::Cron { expr: "0 0 9 * * *".into() },
    false, Some("America/New_York"),
    3,                               // max_retries
    7200,                            // timeout_secs
    SessionMode::Isolated,
    None,                            // user_id
).await?;

// List and manage
let jobs = store.list_jobs().await?;
store.remove_job(&id).await?;

// Execution tracking
let due = store.get_due_jobs().await?;
let run_id = store.record_run_start(&id).await?;
store.record_run_complete(&run_id, RunStatus::Success, Some("All clear")).await?;
let runs = store.list_runs(&id, 10).await?;
```

## Schedule Enum

```rust
pub enum Schedule {
    OneShot { at: String },        // ISO 8601 timestamp
    Interval { every_ms: u64 },    // Fixed interval
    Cron { expr: String },         // 5-field standard or 6-field with seconds
}
```

## CronScheduler

```rust
let scheduler = CronScheduler::new(
    store,              // Arc<CronStore>
    executor,           // Fn(JobContext) -> Future<Result<String, String>>
    30,                 // tick interval (seconds)
    Some("America/New_York".into()),
)
.with_notifier(notifier);  // Optional Fn(job_name, result, success) -> Future

scheduler.start();  // Returns JoinHandle
```

## Callback Types

```rust
// Executes a job — receives a JobContext with prompt, session mode, job name, job ID, user ID
type JobExecutor = Arc<dyn Fn(JobContext) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync>;

// Sends notification after job completion (job_name, result_text, success)
type NotificationSender = Arc<dyn Fn(String, String, bool) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
```

## Types

```rust
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
    pub retry_count: u32,
    pub max_retries: u32,
    pub last_error: Option<String>,
    pub retry_at: Option<i64>,
    pub timeout_secs: u32,
    pub session_mode: SessionMode,      // Isolated or Main
    pub user_id: Option<String>,        // None = agent-level job
}

pub struct JobContext {
    pub prompt: String,
    pub session_mode: SessionMode,
    pub job_name: String,
    pub job_id: String,
    pub user_id: Option<String>,
}

pub struct CronRun {
    pub id: String,
    pub job_id: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub status: RunStatus,
    pub result_summary: Option<String>,
}

pub enum RunStatus {
    Pending, Running, Success, Failed,
}
```

## Configuration

Default retry, timeout, and concurrency settings are configurable via `[cron]` in `agent.toml`:

| Key | Default | Description |
|-----|---------|-------------|
| `default_max_retries` | `3` | Max retries for failed jobs |
| `default_timeout_secs` | `7200` | Job timeout in seconds (2h) |
| `max_concurrent_runs` | `1` | Max concurrent job runs |

## Tests

11 unit tests.
