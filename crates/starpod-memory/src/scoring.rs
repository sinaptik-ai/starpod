//! Scoring, validation, and re-ranking utilities for the memory search pipeline.
//!
//! This module provides:
//!
//! - **Path validation** ([`validate_path`], [`validate_content_size`]) — security
//!   checks preventing directory traversal, non-`.md` writes, and oversized content.
//! - **Temporal decay** ([`decay_factor`], [`apply_decay`]) — penalizes older daily
//!   logs in search results using exponential half-life decay while leaving evergreen
//!   files (SOUL.md, HEARTBEAT.md, etc.) unaffected.
//! - **MMR re-ranking** ([`mmr_rerank`]) — Maximal Marginal Relevance diversifies
//!   search results by balancing query relevance against redundancy with
//!   already-selected results.

use std::path::Path;

use chrono::{Local, NaiveDate};

use starpod_core::{Result, StarpodError};

/// Maximum file size for memory writes (1 MB).
pub const MAX_WRITE_SIZE: usize = 1_048_576;

/// Validate a memory file path, rejecting traversal attacks and unsafe names.
pub fn validate_path(name: &str, data_dir: &Path) -> Result<()> {
    // Reject empty names
    if name.is_empty() {
        return Err(StarpodError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "File name cannot be empty",
        )));
    }

    // Reject names that are too long
    if name.len() > 255 {
        return Err(StarpodError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "File name exceeds 255 characters",
        )));
    }

    // Reject path traversal
    if name.contains("..") {
        return Err(StarpodError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "File name must not contain '..'",
        )));
    }

    // Reject absolute paths
    if Path::new(name).is_absolute() {
        return Err(StarpodError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "File name must be a relative path",
        )));
    }

    // Require .md extension
    if !name.ends_with(".md") {
        return Err(StarpodError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Only .md files are allowed",
        )));
    }

    // Ensure the resolved path stays under data_dir
    let resolved = data_dir.join(name);
    let canonical_data = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    // Use the parent's canonical path if the file doesn't exist yet
    let canonical_resolved = if resolved.exists() {
        resolved.canonicalize().unwrap_or(resolved)
    } else if let Some(parent) = resolved.parent() {
        let canonical_parent = if parent.exists() {
            parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf())
        } else {
            parent.to_path_buf()
        };
        canonical_parent.join(resolved.file_name().unwrap_or_default())
    } else {
        resolved
    };

    if !canonical_resolved.starts_with(&canonical_data) {
        return Err(StarpodError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "File path escapes the data directory",
        )));
    }

    Ok(())
}

/// Validate content size for writes.
pub fn validate_content_size(content: &str) -> Result<()> {
    if content.len() > MAX_WRITE_SIZE {
        return Err(StarpodError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "Content size ({} bytes) exceeds maximum ({} bytes)",
                content.len(),
                MAX_WRITE_SIZE
            ),
        )));
    }
    Ok(())
}

/// Compute a temporal decay factor for a memory source.
///
/// Daily log files (`memory/YYYY-MM-DD.md`) decay with a half-life:
///   `decay = 0.5^(age_days / half_life_days)`
///
/// Evergreen files (SOUL.md, USER.md, MEMORY.md, HEARTBEAT.md, etc.) return 1.0.
pub fn decay_factor(source: &str, half_life_days: f64) -> f64 {
    // Evergreen files don't decay
    let evergreen_prefixes = ["SOUL.md", "USER.md", "MEMORY.md", "HEARTBEAT.md"];
    if evergreen_prefixes.contains(&source) {
        return 1.0;
    }

    // Try to parse date from memory/YYYY-MM-DD.md pattern
    if let Some(date_str) = source
        .strip_prefix("memory/")
        .and_then(|s| s.strip_suffix(".md"))
    {
        if let Ok(file_date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            let today = Local::now().date_naive();
            let age_days = (today - file_date).num_days().max(0) as f64;
            return 0.5_f64.powf(age_days / half_life_days);
        }
    }

    // Non-daily, non-evergreen files (e.g. memory/notes.md) — slight decay
    0.8
}

/// Apply temporal decay to an FTS5 rank score.
///
/// FTS5 ranks are negative (more negative = better match). Multiplying by a
/// decay factor < 1.0 makes the score less negative (closer to zero = worse),
/// effectively penalizing older content.
///
/// Example: rank = -10.0, decay = 0.5 → adjusted = -5.0 (worse match).
pub fn apply_decay(rank: f64, source: &str, half_life_days: f64) -> f64 {
    let factor = decay_factor(source, half_life_days);
    if factor <= 0.0 {
        return rank;
    }
    rank * factor
}

/// Maximal Marginal Relevance (MMR) re-ranking for result diversity.
///
/// Given a query embedding and candidate results with their embeddings,
/// iteratively selects candidates that maximize:
///   `lambda * sim(candidate, query) - (1 - lambda) * max_sim(candidate, selected)`
///
/// This balances relevance (similarity to query) with diversity (dissimilarity
/// from already-selected results).
///
/// `lambda`: 0.0 = maximum diversity, 1.0 = pure relevance. Default: 0.7.
pub fn mmr_rerank(
    query_embedding: &[f32],
    candidates: &[(Vec<f32>, usize)], // (embedding, index into results)
    limit: usize,
    lambda: f64,
) -> Vec<usize> {
    use crate::embedder::cosine_similarity;

    if candidates.is_empty() || limit == 0 {
        return Vec::new();
    }

    // Pre-compute similarities to query
    let query_sims: Vec<f64> = candidates
        .iter()
        .map(|(emb, _)| cosine_similarity(query_embedding, emb) as f64)
        .collect();

    let mut selected: Vec<usize> = Vec::with_capacity(limit); // indices into candidates
    let mut remaining: Vec<usize> = (0..candidates.len()).collect();

    for _ in 0..limit {
        if remaining.is_empty() {
            break;
        }

        let mut best_idx = 0;
        let mut best_score = f64::NEG_INFINITY;

        for (pos, &cand_idx) in remaining.iter().enumerate() {
            let relevance = query_sims[cand_idx];

            // Max similarity to already-selected results
            let max_selected_sim = if selected.is_empty() {
                0.0
            } else {
                selected
                    .iter()
                    .map(|&sel_idx| {
                        cosine_similarity(&candidates[sel_idx].0, &candidates[cand_idx].0) as f64
                    })
                    .fold(f64::NEG_INFINITY, f64::max)
            };

            let mmr_score = lambda * relevance - (1.0 - lambda) * max_selected_sim;

            if mmr_score > best_score {
                best_score = mmr_score;
                best_idx = pos;
            }
        }

        let chosen = remaining.swap_remove(best_idx);
        selected.push(chosen);
    }

    // Return the original result indices
    selected
        .iter()
        .map(|&cand_idx| candidates[cand_idx].1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── Path validation tests ───────────────────────────────────────────

    #[test]
    fn validate_path_accepts_simple_name() {
        let tmp = TempDir::new().unwrap();
        assert!(validate_path("notes.md", tmp.path()).is_ok());
    }

    #[test]
    fn validate_path_accepts_subdirectory() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("subdir")).unwrap();
        assert!(validate_path("subdir/notes.md", tmp.path()).is_ok());
    }

    #[test]
    fn validate_path_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        assert!(validate_path("../etc/passwd.md", tmp.path()).is_err());
        assert!(validate_path("subdir/../../secret.md", tmp.path()).is_err());
    }

    #[test]
    fn validate_path_rejects_absolute() {
        let tmp = TempDir::new().unwrap();
        assert!(validate_path("/etc/passwd.md", tmp.path()).is_err());
    }

    #[test]
    fn validate_path_rejects_non_md() {
        let tmp = TempDir::new().unwrap();
        assert!(validate_path("script.sh", tmp.path()).is_err());
        assert!(validate_path("data.json", tmp.path()).is_err());
    }

    #[test]
    fn validate_path_rejects_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(validate_path("", tmp.path()).is_err());
    }

    #[test]
    fn validate_path_rejects_long_name() {
        let tmp = TempDir::new().unwrap();
        let long_name = format!("{}.md", "a".repeat(254));
        assert!(validate_path(&long_name, tmp.path()).is_err());
    }

    // ── Content size tests ──────────────────────────────────────────────

    #[test]
    fn validate_content_accepts_normal_size() {
        assert!(validate_content_size("hello world").is_ok());
    }

    #[test]
    fn validate_content_rejects_oversized() {
        let big = "x".repeat(MAX_WRITE_SIZE + 1);
        assert!(validate_content_size(&big).is_err());
    }

    #[test]
    fn validate_content_accepts_exact_limit() {
        let exact = "x".repeat(MAX_WRITE_SIZE);
        assert!(validate_content_size(&exact).is_ok());
    }

    // ── Temporal decay tests ────────────────────────────────────────────

    #[test]
    fn decay_factor_evergreen_files() {
        assert_eq!(decay_factor("SOUL.md", 30.0), 1.0);
        assert_eq!(decay_factor("USER.md", 30.0), 1.0);
        assert_eq!(decay_factor("MEMORY.md", 30.0), 1.0);
        assert_eq!(decay_factor("HEARTBEAT.md", 30.0), 1.0);
        // Non-evergreen, non-daily files get a slight decay
        assert_eq!(decay_factor("notes.md", 30.0), 0.8);
    }

    #[test]
    fn decay_factor_today() {
        let today = Local::now().format("memory/%Y-%m-%d.md").to_string();
        let factor = decay_factor(&today, 30.0);
        assert!((factor - 1.0).abs() < 0.01, "Today's factor should be ~1.0, got {}", factor);
    }

    #[test]
    fn decay_factor_30_days_ago() {
        let date = Local::now().date_naive() - chrono::Duration::days(30);
        let source = format!("memory/{}.md", date.format("%Y-%m-%d"));
        let factor = decay_factor(&source, 30.0);
        assert!((factor - 0.5).abs() < 0.01, "30-day-old factor should be ~0.5, got {}", factor);
    }

    #[test]
    fn decay_factor_60_days_ago() {
        let date = Local::now().date_naive() - chrono::Duration::days(60);
        let source = format!("memory/{}.md", date.format("%Y-%m-%d"));
        let factor = decay_factor(&source, 30.0);
        assert!((factor - 0.25).abs() < 0.01, "60-day-old factor should be ~0.25, got {}", factor);
    }

    #[test]
    fn decay_factor_non_dated_memory() {
        assert_eq!(decay_factor("memory/notes.md", 30.0), 0.8);
    }

    #[test]
    fn apply_decay_worsens_old_results() {
        // FTS5 rank of -10.0 (a decent match)
        let rank = -10.0;
        let date = Local::now().date_naive() - chrono::Duration::days(30);
        let source = format!("memory/{}.md", date.format("%Y-%m-%d"));

        let decayed = apply_decay(rank, &source, 30.0);
        // Multiplying -10.0 by 0.5 = -5.0 (less negative = worse rank)
        assert!(decayed > rank, "Decayed rank should be less negative (worse): {} > {}", decayed, rank);
        assert!((decayed - (-5.0)).abs() < 0.1);
    }

    #[test]
    fn apply_decay_preserves_evergreen() {
        let rank = -5.0;
        let decayed = apply_decay(rank, "SOUL.md", 30.0);
        assert_eq!(decayed, rank);
    }

    // ── MMR re-ranking tests ────────────────────────────────────────────

    #[test]
    fn mmr_empty_candidates() {
        let query = vec![1.0, 0.0, 0.0];
        assert!(mmr_rerank(&query, &[], 5, 0.7).is_empty());
    }

    #[test]
    fn mmr_single_candidate() {
        let query = vec![1.0, 0.0, 0.0];
        let candidates = vec![(vec![1.0, 0.0, 0.0], 0)];
        let selected = mmr_rerank(&query, &candidates, 5, 0.7);
        assert_eq!(selected, vec![0]);
    }

    #[test]
    fn mmr_selects_most_relevant_first() {
        let query = vec![1.0, 0.0, 0.0];
        let candidates = vec![
            (vec![0.0, 1.0, 0.0], 0), // orthogonal to query
            (vec![0.9, 0.1, 0.0], 1), // very similar to query
            (vec![0.5, 0.5, 0.0], 2), // moderate similarity
        ];
        let selected = mmr_rerank(&query, &candidates, 3, 1.0); // lambda=1.0 = pure relevance
        assert_eq!(selected[0], 1, "Most relevant should be first");
    }

    #[test]
    fn mmr_promotes_diversity() {
        let query = vec![1.0, 0.0, 0.0];
        // Two near-identical candidates and one diverse one with moderate relevance
        let candidates = vec![
            (vec![1.0, 0.0, 0.0], 0),  // identical to query
            (vec![0.99, 0.01, 0.0], 1), // near-duplicate of candidate 0
            (vec![0.7, 0.7, 0.0], 2),   // different direction but still relevant
        ];
        let selected = mmr_rerank(&query, &candidates, 3, 0.3); // lambda=0.3 = diversity-heavy
        // First should be most relevant, second should be the diverse one
        assert_eq!(selected[0], 0);
        assert_eq!(selected[1], 2, "Diverse candidate should come before near-duplicate");
    }

    #[test]
    fn mmr_respects_limit() {
        let query = vec![1.0, 0.0];
        let candidates: Vec<(Vec<f32>, usize)> = (0..10)
            .map(|i| (vec![1.0, i as f32 * 0.1], i))
            .collect();
        let selected = mmr_rerank(&query, &candidates, 3, 0.7);
        assert_eq!(selected.len(), 3);
    }

    #[test]
    fn mmr_limit_zero_returns_empty() {
        let query = vec![1.0, 0.0];
        let candidates = vec![(vec![1.0, 0.0], 0)];
        assert!(mmr_rerank(&query, &candidates, 0, 0.7).is_empty());
    }

    #[test]
    fn mmr_preserves_original_indices() {
        let query = vec![1.0, 0.0, 0.0];
        // Indices 42 and 99 are the original result positions
        let candidates = vec![
            (vec![0.9, 0.1, 0.0], 42),
            (vec![0.1, 0.9, 0.0], 99),
        ];
        let selected = mmr_rerank(&query, &candidates, 2, 1.0);
        assert_eq!(selected[0], 42, "Should return original index 42");
        assert_eq!(selected[1], 99);
    }

    // ── Path validation edge cases ──────────────────────────────────────

    #[test]
    fn validate_path_accepts_exact_255_chars() {
        let tmp = TempDir::new().unwrap();
        // 255 total: 251 'a's + ".md" = 254 — exactly at the limit
        let name = format!("{}.md", "a".repeat(252));
        assert_eq!(name.len(), 255);
        assert!(validate_path(&name, tmp.path()).is_ok());
    }

    #[test]
    fn validate_path_rejects_hidden_dotdot_in_component() {
        let tmp = TempDir::new().unwrap();
        // "foo/../bar.md" contains ".." even though it looks like a subdirectory
        assert!(validate_path("foo/../bar.md", tmp.path()).is_err());
    }

    #[test]
    fn validate_path_accepts_dotfile() {
        let tmp = TempDir::new().unwrap();
        // A single dot in a filename is fine (not traversal)
        assert!(validate_path(".hidden.md", tmp.path()).is_ok());
    }

    #[test]
    fn validate_path_rejects_md_in_middle() {
        let tmp = TempDir::new().unwrap();
        // Must end with .md, not just contain it
        assert!(validate_path("notes.md.bak", tmp.path()).is_err());
    }

    // ── Temporal decay edge cases ───────────────────────────────────────

    #[test]
    fn decay_factor_future_date_returns_above_one() {
        // A future date should not be penalized (factor >= 1.0)
        let date = Local::now().date_naive() + chrono::Duration::days(5);
        let source = format!("memory/{}.md", date.format("%Y-%m-%d"));
        let factor = decay_factor(&source, 30.0);
        assert!(factor >= 1.0, "Future date factor should be >= 1.0, got {}", factor);
    }

    #[test]
    fn decay_factor_custom_half_life() {
        // With half-life of 7 days, 7 days ago should give ~0.5
        let date = Local::now().date_naive() - chrono::Duration::days(7);
        let source = format!("memory/{}.md", date.format("%Y-%m-%d"));
        let factor = decay_factor(&source, 7.0);
        assert!((factor - 0.5).abs() < 0.01, "7-day half-life, 7 days old should be ~0.5, got {}", factor);
    }

    #[test]
    fn decay_factor_very_old_approaches_zero() {
        let date = Local::now().date_naive() - chrono::Duration::days(365);
        let source = format!("memory/{}.md", date.format("%Y-%m-%d"));
        let factor = decay_factor(&source, 30.0);
        assert!(factor < 0.01, "365-day-old factor should be near 0, got {}", factor);
    }

    #[test]
    fn decay_factor_malformed_date_returns_default() {
        assert_eq!(decay_factor("memory/not-a-date.md", 30.0), 0.8);
        assert_eq!(decay_factor("memory/2026-13-45.md", 30.0), 0.8);
    }

    #[test]
    fn apply_decay_with_factor_one_is_identity() {
        // Evergreen files have factor 1.0 — rank should be unchanged
        let rank = -7.5;
        assert_eq!(apply_decay(rank, "HEARTBEAT.md", 30.0), rank);
    }
}
