-- Session tables: metadata, messages, usage stats, compaction log

CREATE TABLE IF NOT EXISTS session_metadata (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    last_message_at TEXT NOT NULL,
    is_closed INTEGER NOT NULL DEFAULT 0,
    summary TEXT,
    message_count INTEGER NOT NULL DEFAULT 0,
    channel TEXT NOT NULL DEFAULT 'main',
    channel_session_key TEXT,
    title TEXT,
    user_id TEXT NOT NULL DEFAULT 'admin',
    is_read INTEGER NOT NULL DEFAULT 1,
    triggered_by TEXT
);

CREATE INDEX IF NOT EXISTS idx_session_channel_key
    ON session_metadata(channel, channel_session_key, is_closed, last_message_at DESC);

CREATE INDEX IF NOT EXISTS idx_session_user_channel
    ON session_metadata(user_id, channel, channel_session_key);

CREATE TABLE IF NOT EXISTS session_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES session_metadata(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_session_messages_session
    ON session_messages(session_id, id);

CREATE TABLE IF NOT EXISTS usage_stats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    turn INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read INTEGER NOT NULL DEFAULT 0,
    cache_write INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL NOT NULL DEFAULT 0.0,
    model TEXT,
    timestamp TEXT NOT NULL,
    user_id TEXT NOT NULL DEFAULT 'admin'
);

CREATE INDEX IF NOT EXISTS idx_usage_stats_user
    ON usage_stats(user_id, timestamp);

CREATE TABLE IF NOT EXISTS compaction_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES session_metadata(id) ON DELETE CASCADE,
    timestamp TEXT NOT NULL,
    trigger TEXT NOT NULL DEFAULT 'auto',
    pre_tokens INTEGER NOT NULL,
    summary TEXT NOT NULL,
    messages_compacted INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_compaction_log_session
    ON compaction_log(session_id);
