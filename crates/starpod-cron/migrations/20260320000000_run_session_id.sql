-- Link each cron run to the session it used/created.
ALTER TABLE cron_runs ADD COLUMN session_id TEXT;
