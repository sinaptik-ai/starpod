//! Reciprocal Rank Fusion (RRF) for combining FTS5 and vector search results.
//!
//! RRF is a simple, parameter-free method for merging multiple ranked lists.
//! Each document's score is `sum(1 / (K + rank_in_list))` across all lists
//! it appears in. Documents appearing in multiple lists receive a natural
//! boost. The constant `K = 60` (from Cormack, Clarke & Büttcher 2009)
//! controls how much top-ranked results dominate.
//!
//! Used by [`MemoryStore::hybrid_search`](crate::store::MemoryStore::hybrid_search)
//! to combine FTS5 keyword results with vector similarity results.

use std::collections::HashMap;

use crate::store::SearchResult;

/// RRF constant (standard value from the literature: Cormack et al. 2009).
const RRF_K: f64 = 60.0;

/// A candidate for fusion, identified by (source, line_start, line_end).
#[derive(Hash, Eq, PartialEq, Clone)]
struct ChunkKey {
    source: String,
    line_start: usize,
    line_end: usize,
}

/// Fuse two ranked lists using Reciprocal Rank Fusion.
///
/// Each result's score is `sum(1 / (K + rank_in_list))` across the lists
/// it appears in. Higher RRF score = better match.
///
/// The returned results have `rank` set to the negative RRF score so that
/// the existing "more negative = better" convention is preserved.
pub fn reciprocal_rank_fusion(
    fts_results: &[SearchResult],
    vector_results: &[SearchResult],
    limit: usize,
) -> Vec<SearchResult> {
    let mut scores: HashMap<ChunkKey, (f64, SearchResult)> = HashMap::new();

    // Score FTS5 results by their rank position
    for (rank_pos, result) in fts_results.iter().enumerate() {
        let key = ChunkKey {
            source: result.source.clone(),
            line_start: result.line_start,
            line_end: result.line_end,
        };
        let rrf_score = 1.0 / (RRF_K + rank_pos as f64);
        scores
            .entry(key)
            .and_modify(|(score, _)| *score += rrf_score)
            .or_insert((rrf_score, result.clone()));
    }

    // Score vector results by their rank position
    for (rank_pos, result) in vector_results.iter().enumerate() {
        let key = ChunkKey {
            source: result.source.clone(),
            line_start: result.line_start,
            line_end: result.line_end,
        };
        let rrf_score = 1.0 / (RRF_K + rank_pos as f64);
        scores
            .entry(key)
            .and_modify(|(score, _)| *score += rrf_score)
            .or_insert((rrf_score, result.clone()));
    }

    // Sort by RRF score descending, then convert to negative rank
    let mut fused: Vec<(f64, SearchResult)> = scores.into_values().collect();
    fused.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    fused.truncate(limit);

    fused
        .into_iter()
        .map(|(rrf_score, mut result)| {
            result.rank = -rrf_score; // negative so more negative = better
            result
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(source: &str, line_start: usize, rank: f64) -> SearchResult {
        SearchResult {
            source: source.to_string(),
            text: format!("chunk from {} at {}", source, line_start),
            line_start,
            line_end: line_start + 5,
            rank,
        }
    }

    #[test]
    fn rrf_fts_only() {
        let fts = vec![
            make_result("a.md", 1, -10.0),
            make_result("b.md", 1, -5.0),
        ];
        let results = reciprocal_rank_fusion(&fts, &[], 10);
        assert_eq!(results.len(), 2);
        // First result should have better (more negative) rank
        assert!(results[0].rank < results[1].rank);
        assert_eq!(results[0].source, "a.md");
    }

    #[test]
    fn rrf_vector_only() {
        let vec_results = vec![
            make_result("c.md", 1, -0.9),
            make_result("d.md", 1, -0.5),
        ];
        let results = reciprocal_rank_fusion(&[], &vec_results, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source, "c.md");
    }

    #[test]
    fn rrf_overlap_boosts_shared_results() {
        // "a.md" appears in both lists — should get boosted
        let fts = vec![
            make_result("a.md", 1, -10.0),
            make_result("b.md", 1, -5.0),
        ];
        let vec_results = vec![
            make_result("a.md", 1, -0.9),
            make_result("c.md", 1, -0.5),
        ];
        let results = reciprocal_rank_fusion(&fts, &vec_results, 10);
        assert_eq!(results[0].source, "a.md", "Shared result should rank first");
        assert_eq!(results.len(), 3); // a.md, b.md, c.md (deduplicated)
    }

    #[test]
    fn rrf_respects_limit() {
        let fts: Vec<SearchResult> = (0..20)
            .map(|i| make_result(&format!("f{}.md", i), 1, -(20 - i) as f64))
            .collect();
        let results = reciprocal_rank_fusion(&fts, &[], 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn rrf_both_empty() {
        let results = reciprocal_rank_fusion(&[], &[], 10);
        assert!(results.is_empty());
    }

    #[test]
    fn rrf_deduplicates_by_source_and_lines() {
        // Same chunk in both lists — should appear only once, with boosted score
        let fts = vec![make_result("a.md", 1, -10.0)];
        let vec_results = vec![make_result("a.md", 1, -0.9)];
        let results = reciprocal_rank_fusion(&fts, &vec_results, 10);
        assert_eq!(results.len(), 1, "Duplicate chunk should be merged");
    }

    #[test]
    fn rrf_different_line_ranges_are_distinct() {
        // Same source but different line ranges — should NOT be merged
        let fts = vec![make_result("a.md", 1, -10.0)];
        let vec_results = vec![make_result("a.md", 20, -0.9)];
        let results = reciprocal_rank_fusion(&fts, &vec_results, 10);
        assert_eq!(results.len(), 2, "Different line ranges are distinct chunks");
    }

    #[test]
    fn rrf_scores_are_negative() {
        // All output ranks should be negative (convention: more negative = better)
        let fts = vec![
            make_result("a.md", 1, -10.0),
            make_result("b.md", 1, -5.0),
        ];
        let results = reciprocal_rank_fusion(&fts, &[], 10);
        for r in &results {
            assert!(r.rank < 0.0, "RRF rank should be negative, got {}", r.rank);
        }
    }

    #[test]
    fn rrf_shared_result_has_higher_score_than_single() {
        // A result appearing in both lists should score better than one in only one list
        let fts = vec![
            make_result("shared.md", 1, -10.0),
            make_result("fts_only.md", 1, -5.0),
        ];
        let vec_results = vec![
            make_result("shared.md", 1, -0.9),
            make_result("vec_only.md", 1, -0.5),
        ];
        let results = reciprocal_rank_fusion(&fts, &vec_results, 10);

        let shared_rank = results.iter().find(|r| r.source == "shared.md").unwrap().rank;
        let fts_only_rank = results.iter().find(|r| r.source == "fts_only.md").unwrap().rank;
        let vec_only_rank = results.iter().find(|r| r.source == "vec_only.md").unwrap().rank;

        // shared should have a more negative (better) rank
        assert!(
            shared_rank < fts_only_rank && shared_rank < vec_only_rank,
            "Shared result rank ({}) should be better than fts_only ({}) and vec_only ({})",
            shared_rank, fts_only_rank, vec_only_rank
        );
    }
}
