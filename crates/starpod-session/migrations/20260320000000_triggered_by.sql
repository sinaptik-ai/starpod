-- Add triggered_by column to track which cron job or heartbeat created the session.
-- NULL = regular user session, non-NULL = cron job name (e.g. "daily-digest", "__heartbeat__").
ALTER TABLE session_metadata ADD COLUMN triggered_by TEXT;
