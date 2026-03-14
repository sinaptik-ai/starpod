-- Convert all timestamp columns from RFC3339 strings to Unix epoch integers.
-- SQLite doesn't support ALTER COLUMN, so we recreate both tables.

PRAGMA foreign_keys = OFF;

-- Recreate cron_jobs with INTEGER timestamps
CREATE TABLE cron_jobs_new (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    prompt TEXT NOT NULL,
    schedule_type TEXT NOT NULL,
    schedule_value TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    delete_after_run INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_run_at INTEGER,
    next_run_at INTEGER
);

INSERT INTO cron_jobs_new (id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run, created_at, last_run_at, next_run_at)
SELECT id, name, prompt, schedule_type, schedule_value, enabled, delete_after_run,
    CAST(strftime('%s', REPLACE(created_at, 'Z', '+00:00')) AS INTEGER),
    CASE WHEN last_run_at IS NOT NULL THEN CAST(strftime('%s', REPLACE(last_run_at, 'Z', '+00:00')) AS INTEGER) END,
    CASE WHEN next_run_at IS NOT NULL THEN CAST(strftime('%s', REPLACE(next_run_at, 'Z', '+00:00')) AS INTEGER) END
FROM cron_jobs;

DROP TABLE cron_jobs;
ALTER TABLE cron_jobs_new RENAME TO cron_jobs;

-- Recreate cron_runs with INTEGER timestamps
CREATE TABLE cron_runs_new (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    completed_at INTEGER,
    status TEXT NOT NULL DEFAULT 'pending',
    result_summary TEXT
);

INSERT INTO cron_runs_new (id, job_id, started_at, completed_at, status, result_summary)
SELECT id, job_id,
    CAST(strftime('%s', REPLACE(started_at, 'Z', '+00:00')) AS INTEGER),
    CASE WHEN completed_at IS NOT NULL THEN CAST(strftime('%s', REPLACE(completed_at, 'Z', '+00:00')) AS INTEGER) END,
    status, result_summary
FROM cron_runs;

DROP TABLE cron_runs;
ALTER TABLE cron_runs_new RENAME TO cron_runs;

-- Recreate indexes
CREATE INDEX IF NOT EXISTS idx_cron_runs_job ON cron_runs(job_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_next ON cron_jobs(next_run_at);

PRAGMA foreign_keys = ON;
