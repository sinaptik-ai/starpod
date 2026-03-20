-- Add is_read flag to session_metadata (default true — existing sessions are "read")
ALTER TABLE session_metadata ADD COLUMN is_read INTEGER NOT NULL DEFAULT 1;
