-- Phase 2: Concurrency guard + stuck job timeout
ALTER TABLE cron_jobs ADD COLUMN timeout_secs INTEGER NOT NULL DEFAULT 7200;
