-- Add user_id to cron_jobs for per-user scheduling.
-- NULL = agent-level job, non-NULL = user-specific job.
ALTER TABLE cron_jobs ADD COLUMN user_id TEXT;
