//! Ollama model discovery.
//!
//! Queries a running Ollama instance to discover locally-available models
//! and their capabilities, so they don't need to be pre-registered in the
//! model catalog.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use tracing::{debug, warn};

use crate::models::{ModelInfo, ModelRegistry};
use crate::provider::CostRates;

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

// ── API response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagEntry>,
}

#[derive(Debug, Deserialize)]
struct TagEntry {
    name: String,
    #[serde(default)]
    details: TagDetails,
}

#[derive(Debug, Default, Deserialize)]
struct TagDetails {
    #[serde(default)]
    family: String,
    #[serde(default)]
    parameter_size: String,
    #[serde(default)]
    quantization_level: String,
}

#[derive(Debug, Deserialize)]
struct ShowResponse {
    #[serde(default)]
    model_info: HashMap<String, serde_json::Value>,
}

// ── Public types ────────────────────────────────────────────────────────

/// Summary of a locally-available Ollama model (from `/api/tags`).
#[derive(Debug, Clone)]
pub struct OllamaModelSummary {
    /// Model name as Ollama knows it (e.g. `"qwen3.5:9b"`).
    pub name: String,
    /// Model family (e.g. `"qwen35"`, `"llama"`).
    pub family: String,
    /// Human-readable parameter count (e.g. `"9.7B"`).
    pub parameter_size: String,
    /// Quantization level (e.g. `"Q4_K_M"`).
    pub quantization_level: String,
}

/// Detailed metadata for a single Ollama model (from `/api/show`).
#[derive(Debug, Clone)]
pub struct OllamaModelDetail {
    /// Context window in tokens.
    pub context_length: u64,
    /// Whether the model has a vision encoder.
    pub supports_vision: bool,
    /// Model family.
    pub family: String,
}

impl OllamaModelDetail {
    /// Convert into a [`ModelInfo`] with zero pricing (local = free).
    pub fn into_model_info(self, model_name: &str) -> ModelInfo {
        ModelInfo {
            id: model_name.to_string(),
            provider: "ollama".to_string(),
            pricing: CostRates {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_multiplier: None,
                cache_creation_multiplier: None,
            },
            context_window: Some(self.context_length),
            supports_tool_use: true,
            supports_vision: self.supports_vision,
        }
    }
}

// ── Discovery client ────────────────────────────────────────────────────

/// Client for discovering models from a running Ollama instance.
pub struct OllamaDiscovery {
    base_url: String,
    http: reqwest::Client,
}

impl OllamaDiscovery {
    /// Create a discovery client pointing at the given Ollama base URL.
    ///
    /// The URL should be the root (e.g. `http://localhost:11434`), not
    /// the OpenAI-compatible chat endpoint.
    pub fn new(base_url: &str) -> Self {
        // Strip the /v1/chat/completions suffix if the caller passed the
        // OpenAI-compatible endpoint URL.
        let base = base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1/chat/completions")
            .to_string();

        let http = reqwest::Client::builder()
            .timeout(DISCOVERY_TIMEOUT)
            .build()
            .unwrap_or_default();

        Self { base_url: base, http }
    }

    /// Create a discovery client with the default Ollama URL (`localhost:11434`).
    pub fn default_url() -> Self {
        Self::new(DEFAULT_OLLAMA_URL)
    }

    /// List all locally-available models via `GET /api/tags`.
    pub async fn list_models(&self) -> Result<Vec<OllamaModelSummary>, String> {
        let url = format!("{}/api/tags", self.base_url);
        let resp: TagsResponse = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("ollama /api/tags request failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("ollama /api/tags parse failed: {e}"))?;

        Ok(resp
            .models
            .into_iter()
            .map(|t| OllamaModelSummary {
                name: t.name,
                family: t.details.family,
                parameter_size: t.details.parameter_size,
                quantization_level: t.details.quantization_level,
            })
            .collect())
    }

    /// Get detailed metadata for a single model via `POST /api/show`.
    pub async fn show_model(&self, name: &str) -> Result<OllamaModelDetail, String> {
        let url = format!("{}/api/show", self.base_url);
        let body = serde_json::json!({ "model": name });
        let resp: ShowResponse = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("ollama /api/show request failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("ollama /api/show parse failed: {e}"))?;

        // Extract context_length: look for any key ending with ".context_length".
        let context_length = resp
            .model_info
            .iter()
            .find(|(k, _)| k.ends_with(".context_length"))
            .and_then(|(_, v)| v.as_u64())
            .unwrap_or(131_072); // sensible default

        // Detect vision support: any key matching "*.vision.block_count" with value > 0.
        let supports_vision = resp
            .model_info
            .iter()
            .find(|(k, _)| k.contains(".vision.block_count"))
            .and_then(|(_, v)| v.as_u64())
            .map(|n| n > 0)
            .unwrap_or(false);

        // Family from "general.architecture".
        let family = resp
            .model_info
            .get("general.architecture")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(OllamaModelDetail {
            context_length,
            supports_vision,
            family,
        })
    }

    /// Discover all local models and return a [`ModelRegistry`] containing them.
    ///
    /// Calls `/api/tags` to list models, then `/api/show` for each to get
    /// context window and capabilities. Models that fail `/api/show` are
    /// registered with sensible defaults.
    pub async fn discover_all(&self) -> Result<ModelRegistry, String> {
        let summaries = self.list_models().await?;
        let mut registry = ModelRegistry::new();

        // Deduplicate by digest — models like "qwen3.5:9b" and "qwen3.5:latest"
        // are often the same blob. We keep all names but only call /api/show once
        // per unique model.
        for summary in &summaries {
            let detail = match self.show_model(&summary.name).await {
                Ok(d) => d,
                Err(e) => {
                    warn!(model = %summary.name, error = %e, "failed to query ollama model details, using defaults");
                    OllamaModelDetail {
                        context_length: 131_072,
                        supports_vision: false,
                        family: summary.family.clone(),
                    }
                }
            };
            debug!(
                model = %summary.name,
                context_length = detail.context_length,
                vision = detail.supports_vision,
                "discovered ollama model"
            );
            registry.register("ollama", &summary.name, detail.into_model_info(&summary.name));
        }

        Ok(registry)
    }

    /// Discover a single model and return its [`ModelInfo`].
    ///
    /// Useful for on-demand enrichment when a model is referenced but not
    /// yet in the registry.
    pub async fn discover_one(&self, model_name: &str) -> Result<ModelInfo, String> {
        let detail = self.show_model(model_name).await?;
        Ok(detail.into_model_info(model_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_openai_suffix_from_base_url() {
        let d = OllamaDiscovery::new("http://localhost:11434/v1/chat/completions");
        assert_eq!(d.base_url, "http://localhost:11434");
    }

    #[test]
    fn strips_trailing_slash() {
        let d = OllamaDiscovery::new("http://localhost:11434/");
        assert_eq!(d.base_url, "http://localhost:11434");
    }

    #[test]
    fn default_url_is_localhost() {
        let d = OllamaDiscovery::default_url();
        assert_eq!(d.base_url, "http://localhost:11434");
    }

    #[test]
    fn detail_into_model_info() {
        let detail = OllamaModelDetail {
            context_length: 262_144,
            supports_vision: true,
            family: "qwen35".into(),
        };
        let info = detail.into_model_info("qwen3.5:9b");
        assert_eq!(info.id, "qwen3.5:9b");
        assert_eq!(info.provider, "ollama");
        assert_eq!(info.context_window, Some(262_144));
        assert!(info.supports_vision);
        assert!(info.supports_tool_use);
        assert!((info.pricing.input_per_million - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_tags_response() {
        let json = r#"{
            "models": [
                {
                    "name": "llama3:8b",
                    "model": "llama3:8b",
                    "size": 4000000000,
                    "details": {
                        "family": "llama",
                        "parameter_size": "8B",
                        "quantization_level": "Q4_0"
                    }
                }
            ]
        }"#;
        let resp: TagsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.models.len(), 1);
        assert_eq!(resp.models[0].name, "llama3:8b");
        assert_eq!(resp.models[0].details.family, "llama");
    }

    #[test]
    fn parse_show_response_extracts_context_and_vision() {
        let json = r#"{
            "model_info": {
                "general.architecture": "qwen35",
                "qwen35.context_length": 262144,
                "qwen35.vision.block_count": 27
            }
        }"#;
        let resp: ShowResponse = serde_json::from_str(json).unwrap();

        let ctx = resp
            .model_info
            .iter()
            .find(|(k, _)| k.ends_with(".context_length"))
            .and_then(|(_, v)| v.as_u64())
            .unwrap();
        assert_eq!(ctx, 262_144);

        let vision = resp
            .model_info
            .iter()
            .find(|(k, _)| k.contains(".vision.block_count"))
            .and_then(|(_, v)| v.as_u64())
            .map(|n| n > 0)
            .unwrap_or(false);
        assert!(vision);
    }

    #[test]
    fn parse_show_response_no_vision() {
        let json = r#"{
            "model_info": {
                "general.architecture": "llama",
                "llama.context_length": 131072
            }
        }"#;
        let resp: ShowResponse = serde_json::from_str(json).unwrap();

        let vision = resp
            .model_info
            .iter()
            .find(|(k, _)| k.contains(".vision.block_count"))
            .and_then(|(_, v)| v.as_u64())
            .map(|n| n > 0)
            .unwrap_or(false);
        assert!(!vision);
    }

    #[test]
    fn parse_show_response_vision_block_count_zero() {
        let json = r#"{
            "model_info": {
                "general.architecture": "llama",
                "llama.context_length": 131072,
                "llama.vision.block_count": 0
            }
        }"#;
        let resp: ShowResponse = serde_json::from_str(json).unwrap();

        let vision = resp
            .model_info
            .iter()
            .find(|(k, _)| k.contains(".vision.block_count"))
            .and_then(|(_, v)| v.as_u64())
            .map(|n| n > 0)
            .unwrap_or(false);
        assert!(!vision);
    }

    #[test]
    fn parse_empty_tags_response() {
        let json = r#"{ "models": [] }"#;
        let resp: TagsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.models.is_empty());
    }

    #[test]
    fn parse_tags_missing_models_field() {
        let json = r#"{}"#;
        let resp: TagsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.models.is_empty());
    }

    #[test]
    fn detail_no_vision_model_info() {
        let detail = OllamaModelDetail {
            context_length: 8_192,
            supports_vision: false,
            family: "llama".into(),
        };
        let info = detail.into_model_info("llama3:8b");
        assert_eq!(info.context_window, Some(8_192));
        assert!(!info.supports_vision);
        assert!(info.supports_tool_use);
    }

    #[test]
    fn custom_base_url_preserved() {
        let d = OllamaDiscovery::new("http://192.168.1.100:11434");
        assert_eq!(d.base_url, "http://192.168.1.100:11434");
    }

    #[test]
    fn parse_tags_with_multiple_models() {
        let json = r#"{
            "models": [
                {
                    "name": "llama3:8b",
                    "details": { "family": "llama", "parameter_size": "8B", "quantization_level": "Q4_0" }
                },
                {
                    "name": "qwen3.5:9b",
                    "details": { "family": "qwen35", "parameter_size": "9.7B", "quantization_level": "Q4_K_M" }
                },
                {
                    "name": "mistral:latest",
                    "details": { "family": "mistral", "parameter_size": "7B", "quantization_level": "Q4_0" }
                }
            ]
        }"#;
        let resp: TagsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.models.len(), 3);
        assert_eq!(resp.models[1].details.family, "qwen35");
        assert_eq!(resp.models[1].details.quantization_level, "Q4_K_M");
    }
}
