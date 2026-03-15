-- Phase 1: Retry with exponential backoff
ALTER TABLE cron_jobs ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cron_jobs ADD COLUMN max_retries INTEGER NOT NULL DEFAULT 3;
ALTER TABLE cron_jobs ADD COLUMN last_error TEXT;
ALTER TABLE cron_jobs ADD COLUMN retry_at INTEGER;
