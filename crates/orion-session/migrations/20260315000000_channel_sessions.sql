-- Add channel-based session routing.
-- "main" = explicit sessions (web, REPL, CLI); "telegram" = time-gap sessions (6h).
ALTER TABLE session_metadata ADD COLUMN channel TEXT NOT NULL DEFAULT 'main';
ALTER TABLE session_metadata ADD COLUMN channel_session_key TEXT;

CREATE INDEX idx_session_channel_key
    ON session_metadata(channel, channel_session_key, is_closed, last_message_at DESC);
