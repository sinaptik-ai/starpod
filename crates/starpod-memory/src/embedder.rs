//! Embedding support for vector search.
//!
//! Provides the [`Embedder`] trait for pluggable text embedding models and a
//! concrete [`LocalEmbedder`] (behind the `embeddings` feature) that uses
//! [fastembed](https://docs.rs/fastembed) with the BGE-Small-EN v1.5 model
//! (384 dimensions, ~45 MB on disk).
//!
//! Also provides [`cosine_similarity`] for comparing embedding vectors.

use starpod_core::Result;
#[cfg(feature = "embeddings")]
use starpod_core::StarpodError;

/// Trait for text embedding models.
///
/// Implementations must be `Send + Sync` to allow sharing across async tasks
/// via `Arc<dyn Embedder>`.
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Embed one or more texts into fixed-dimensional vectors.
    ///
    /// Returns one vector per input text. All vectors have the same
    /// dimensionality (see [`dimensions`](Self::dimensions)).
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Dimensionality of the output vectors (e.g., 384 for BGE-Small-EN).
    fn dimensions(&self) -> usize;
}

/// Local embedder using fastembed (BGE-Small-EN v1.5, 384 dims).
///
/// The model is lazily initialized on the first call to [`embed`](Embedder::embed),
/// which downloads the model weights (~45 MB) if not already cached. Both the
/// init and inference are CPU-bound and internally blocking, so all calls are
/// dispatched onto Tokio's blocking thread pool via [`spawn_blocking`] to
/// avoid stalling async runtime workers during cold starts on slow networks.
///
/// Thread-safe: the inner model is protected by a `Mutex` and shared across
/// tasks via `Arc`.
#[cfg(feature = "embeddings")]
pub struct LocalEmbedder {
    model: std::sync::Arc<std::sync::Mutex<Option<fastembed::TextEmbedding>>>,
}

#[cfg(feature = "embeddings")]
impl LocalEmbedder {
    /// Create a new `LocalEmbedder`. The underlying model is loaded lazily.
    pub fn new() -> Self {
        Self {
            model: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Blocking helper: get or initialize the fastembed model and run
    /// inference on the provided texts. Intended to be called from inside
    /// `tokio::task::spawn_blocking`.
    fn embed_blocking(
        model: &std::sync::Mutex<Option<fastembed::TextEmbedding>>,
        texts: Vec<String>,
    ) -> Result<Vec<Vec<f32>>> {
        let mut guard = model
            .lock()
            .map_err(|e| StarpodError::Agent(format!("Embedder lock poisoned: {}", e)))?;
        if guard.is_none() {
            let m = fastembed::TextEmbedding::try_new(
                fastembed::InitOptions::new(fastembed::EmbeddingModel::BGESmallENV15)
                    .with_show_download_progress(false),
            )
            .map_err(|e| StarpodError::Agent(format!("Failed to init embedding model: {}", e)))?;
            *guard = Some(m);
        }
        let m = guard.as_ref().expect("initialized above");
        m.embed(texts, None)
            .map_err(|e| StarpodError::Agent(format!("Embedding failed: {}", e)))
    }
}

#[cfg(feature = "embeddings")]
impl Default for LocalEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "embeddings")]
#[async_trait::async_trait]
impl Embedder for LocalEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let model = self.model.clone();
        let texts = texts.to_vec();
        tokio::task::spawn_blocking(move || Self::embed_blocking(&model, texts))
            .await
            .map_err(|e| StarpodError::Agent(format!("Embedder task join failed: {}", e)))?
    }

    fn dimensions(&self) -> usize {
        384
    }
}

/// Compute cosine similarity between two vectors.
///
/// Returns a value in `[-1.0, 1.0]`:
/// - `1.0` = identical direction
/// - `0.0` = orthogonal (unrelated)
/// - `-1.0` = opposite direction
///
/// If either vector is zero-length, returns `0.0`.
///
/// Only the overlapping dimensions are considered (i.e., `min(a.len(), b.len())`
/// pairs are used via `zip`). In practice both vectors should have the same
/// dimensionality.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![1.0, 2.0];
        let b = vec![0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_both_zero_vectors() {
        let a = vec![0.0, 0.0];
        let b = vec![0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_high_dimensional() {
        // 384-dim vectors (same as BGE-Small-EN) — identical direction
        let a: Vec<f32> = (0..384).map(|i| (i as f32).sin()).collect();
        let b = a.clone();
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 1e-5,
            "Identical 384-dim vectors should have sim ~1.0, got {}",
            sim
        );
    }

    #[test]
    fn cosine_different_lengths_uses_shorter() {
        // zip truncates to shorter length — [1,0] . [1] = 1 / (1 * 1) = 1.0
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0];
        let sim = cosine_similarity(&a, &b);
        // dot = 1, norm_a = sqrt(1+0+0) = 1, norm_b = 1
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_scaled_vectors_are_equal() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![2.0, 4.0, 6.0]; // same direction, 2x magnitude
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "Scaled vectors should have similarity 1.0"
        );
    }
}
