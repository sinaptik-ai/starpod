-- Phase 3: Session targeting
ALTER TABLE cron_jobs ADD COLUMN session_mode TEXT NOT NULL DEFAULT 'isolated';
