CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    source,
    chunk_text,
    line_start,
    line_end,
    tokenize = 'porter'
);
