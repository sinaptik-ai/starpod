use sqlx::SqlitePool;

use starpod_core::StarpodError;

/// A chunk of text extracted from a markdown file for FTS indexing.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub source: String,
    pub text: String,
    pub line_start: usize,
    pub line_end: usize,
}

/// Target chunk size in characters (~400 tokens ≈ 1600 chars).
pub const CHUNK_SIZE: usize = 1600;
/// Overlap in characters (~80 tokens ≈ 320 chars).
pub const CHUNK_OVERLAP: usize = 320;

/// Split text into chunks with overlap, splitting at line boundaries.
///
/// `chunk_size` and `chunk_overlap` control the target chunk size and overlap
/// in characters. Pass [`CHUNK_SIZE`] and [`CHUNK_OVERLAP`] for the defaults.
pub fn chunk_text(source: &str, text: &str, chunk_size: usize, chunk_overlap: usize) -> Vec<Chunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start_line = 0;

    while start_line < lines.len() {
        let mut char_count = 0;
        let mut end_line = start_line;

        // Accumulate lines until we reach the chunk size
        while end_line < lines.len() && char_count < chunk_size {
            char_count += lines[end_line].len() + 1; // +1 for newline
            end_line += 1;
        }

        let chunk_text: String = lines[start_line..end_line].join("\n");
        if !chunk_text.trim().is_empty() {
            chunks.push(Chunk {
                source: source.to_string(),
                text: chunk_text,
                line_start: start_line + 1, // 1-indexed
                line_end: end_line,
            });
        }

        // Advance past the chunk, minus overlap
        let mut overlap_chars = 0;
        let mut overlap_lines = 0;
        for i in (start_line..end_line).rev() {
            overlap_chars += lines[i].len() + 1;
            overlap_lines += 1;
            if overlap_chars >= chunk_overlap {
                break;
            }
        }

        let advance = end_line - start_line;
        if advance <= overlap_lines {
            // Can't make progress — move forward by at least 1 line
            start_line = end_line;
        } else {
            start_line = end_line - overlap_lines;
        }
    }

    chunks
}

/// Delete all FTS entries for a given source, then insert new chunks.
pub async fn reindex_source(
    pool: &SqlitePool,
    source: &str,
    text: &str,
    chunk_size: usize,
    chunk_overlap: usize,
) -> Result<(), StarpodError> {
    // Delete old entries for this source
    sqlx::query("DELETE FROM memory_fts WHERE source = ?1")
        .bind(source)
        .execute(pool)
        .await
        .map_err(|e| StarpodError::Database(format!("Failed to delete old chunks: {}", e)))?;

    // Chunk and insert
    let chunks = chunk_text(source, text, chunk_size, chunk_overlap);
    for chunk in &chunks {
        sqlx::query("INSERT INTO memory_fts (source, chunk_text, line_start, line_end) VALUES (?1, ?2, ?3, ?4)")
            .bind(&chunk.source)
            .bind(&chunk.text)
            .bind(chunk.line_start as i64)
            .bind(chunk.line_end as i64)
            .execute(pool)
            .await
            .map_err(|e| StarpodError::Database(format!("Failed to insert chunk: {}", e)))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_text_small() {
        let text = "line one\nline two\nline three";
        let chunks = chunk_text("test.md", text, CHUNK_SIZE, CHUNK_OVERLAP);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source, "test.md");
        assert_eq!(chunks[0].line_start, 1);
        assert_eq!(chunks[0].line_end, 3);
    }

    #[test]
    fn test_chunk_text_empty() {
        let chunks = chunk_text("test.md", "", CHUNK_SIZE, CHUNK_OVERLAP);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_text_large() {
        // Create text larger than CHUNK_SIZE
        let line = "x".repeat(200);
        let text: String = (0..20)
            .map(|_| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_text("big.md", &text, CHUNK_SIZE, CHUNK_OVERLAP);
        assert!(chunks.len() > 1, "Should produce multiple chunks");
        // Every chunk should have content
        for chunk in &chunks {
            assert!(!chunk.text.trim().is_empty());
        }
    }

    #[test]
    fn test_chunk_text_custom_sizes() {
        // Build a long text (~4000 chars): 20 lines of 200 chars each
        let line = "a".repeat(200);
        let long_text: String = (0..20)
            .map(|_| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let chunks_default = chunk_text("test.md", &long_text, CHUNK_SIZE, CHUNK_OVERLAP);
        let chunks_small = chunk_text("test.md", &long_text, 200, 50);

        // A smaller chunk_size must produce MORE chunks
        assert!(
            chunks_small.len() > chunks_default.len(),
            "Small chunk_size ({} chunks) should produce more chunks than default ({} chunks)",
            chunks_small.len(),
            chunks_default.len(),
        );

        // Verify overlap is respected: consecutive chunks should share some text.
        // With overlap=50, the tail of chunk N should appear at the start of chunk N+1.
        if chunks_small.len() >= 2 {
            for i in 0..chunks_small.len() - 1 {
                let current_lines: Vec<&str> = chunks_small[i].text.lines().collect();
                let next_lines: Vec<&str> = chunks_small[i + 1].text.lines().collect();
                // The first line of the next chunk should be present somewhere in the current chunk
                let first_next_line = next_lines[0];
                assert!(
                    current_lines.contains(&first_next_line),
                    "Overlap not respected between chunk {} and chunk {}: first line of next chunk not found in current chunk",
                    i,
                    i + 1,
                );
            }
        }
    }
}
