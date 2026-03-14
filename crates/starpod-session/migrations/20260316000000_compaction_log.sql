CREATE TABLE IF NOT EXISTS compaction_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    trigger TEXT NOT NULL DEFAULT 'auto',
    pre_tokens INTEGER NOT NULL,
    summary TEXT NOT NULL,
    messages_compacted INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (session_id) REFERENCES session_metadata(id)
);

CREATE INDEX idx_compaction_log_session ON compaction_log(session_id);
