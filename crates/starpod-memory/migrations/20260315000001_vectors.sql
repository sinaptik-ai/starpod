CREATE TABLE IF NOT EXISTS memory_vectors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    chunk_idx INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vectors_source ON memory_vectors(source);
