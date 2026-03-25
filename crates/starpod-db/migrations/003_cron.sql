-- Cron tables: jobs and run history

CREATE TABLE IF NOT EXISTS cron_jobs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    prompt TEXT NOT NULL,
    schedule_type TEXT NOT NULL,
    schedule_value TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    delete_after_run INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_run_at INTEGER,
    next_run_at INTEGER,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 3,
    last_error TEXT,
    retry_at INTEGER,
    timeout_secs INTEGER NOT NULL DEFAULT 7200,
    session_mode TEXT NOT NULL DEFAULT 'isolated',
    user_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_cron_jobs_next
    ON cron_jobs(next_run_at);

CREATE TABLE IF NOT EXISTS cron_runs (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    completed_at INTEGER,
    status TEXT NOT NULL DEFAULT 'pending',
    result_summary TEXT,
    session_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_cron_runs_job
    ON cron_runs(job_id, started_at DESC);
