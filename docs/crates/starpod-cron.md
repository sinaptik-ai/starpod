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
    Cron { expr: String },         // 6-field with seconds
}
```

## CronScheduler

```rust
let scheduler = CronScheduler::new(
    store,
    executor,           // Fn(prompt) -> Future<Result<String, String>>
    30,                 // tick interval (seconds)
    Some("America/New_York".into()),
)
.with_notifier(notifier);  // Optional Fn(job_name, result, success) -> Future

scheduler.start();  // Returns JoinHandle
```

## Callback Types

```rust
// Executes a job prompt, returns result text
type JobExecutor = Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync>;

// Sends notification after job completion
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

## Tests

11 unit tests.
