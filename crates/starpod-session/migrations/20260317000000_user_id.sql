-- Add user_id column to session_metadata for per-user session scoping.
ALTER TABLE session_metadata ADD COLUMN user_id TEXT NOT NULL DEFAULT 'admin';

-- Create composite index for efficient per-user session lookups.
CREATE INDEX IF NOT EXISTS idx_session_user_channel ON session_metadata(user_id, channel, channel_session_key);
