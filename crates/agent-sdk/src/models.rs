//! Centralized model catalog with pricing, capabilities, and provider metadata.
//!
//! The [`ModelRegistry`] is the single source of truth for every model the
//! system knows about. It ships with embedded defaults (from `defaults/models.toml`)
//! and supports overlaying a user-provided TOML file at runtime — so updates
//! don't require recompilation.
//!
//! # Example
//!
//! ```rust
//! use agent_sdk::models::ModelRegistry;
//!
//! let registry = ModelRegistry::with_defaults();
//! let info = registry.get("anthropic", "claude-sonnet-4-5").unwrap();
//! assert!(info.pricing.input_per_million > 0.0);
//! assert_eq!(info.context_window, Some(200_000));
//! ```

use std::collections::HashMap;

use serde::Deserialize;
use tracing::debug;

use crate::provider::CostRates;

/// Embedded default catalog (compiled into the binary).
const DEFAULTS_TOML: &str = include_str!("defaults/models.toml");

// ── TOML serde types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CatalogFile {
    #[serde(flatten)]
    providers: HashMap<String, ProviderEntry>,
}

#[derive(Debug, Deserialize)]
struct ProviderEntry {
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    cache_read_multiplier: Option<f64>,
    #[serde(default)]
    cache_creation_multiplier: Option<f64>,
    #[serde(default)]
    models: HashMap<String, ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    input: f64,
    output: f64,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default = "default_true")]
    supports_tool_use: bool,
    #[serde(default)]
    supports_vision: bool,
    #[serde(default)]
    cache_read_multiplier: Option<f64>,
    #[serde(default)]
    cache_creation_multiplier: Option<f64>,
}

fn default_true() -> bool {
    true
}

// ── Public types ──────────────────────────────────────────────────────

/// Full information about a model: pricing + capabilities.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Model ID as registered in the catalog.
    pub id: String,
    /// Provider name (e.g. "anthropic", "openai").
    pub provider: String,
    /// Cost rates for this model.
    pub pricing: CostRates,
    /// Maximum context window in tokens.
    pub context_window: Option<u64>,
    /// Whether this model supports tool use.
    pub supports_tool_use: bool,
    /// Whether this model supports vision/images.
    pub supports_vision: bool,
}

/// Provider-level metadata.
#[derive(Debug, Clone)]
pub struct ProviderInfo {
    /// Provider name (e.g. "anthropic").
    pub name: String,
    /// Default model for this provider.
    pub default_model: Option<String>,
    /// Environment variable for the API key.
    pub api_key_env: Option<String>,
    /// Provider-level cache read multiplier.
    pub cache_read_multiplier: Option<f64>,
    /// Provider-level cache creation multiplier.
    pub cache_creation_multiplier: Option<f64>,
}

// ── Registry ──────────────────────────────────────────────────────────

/// Composite key: `"provider::model"`.
type ModelKey = String;

fn make_key(provider: &str, model: &str) -> ModelKey {
    format!("{provider}::{model}")
}

/// Centralized model catalog with pricing, capabilities, and provider metadata.
///
/// Lookup order for pricing/model queries:
/// 1. Exact match on `"provider::model"`
/// 2. Fuzzy match — any registered model whose name is a substring of the
///    query (or vice-versa), scoped to the same provider
/// 3. Provider-level default entry (cache multipliers only, via `get_pricing`)
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    models: HashMap<ModelKey, ModelInfo>,
    providers: HashMap<String, ProviderInfo>,
}

impl ModelRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            providers: HashMap::new(),
        }
    }

    /// Create a registry pre-loaded with the embedded defaults.
    pub fn with_defaults() -> Self {
        Self::from_toml(DEFAULTS_TOML).expect("embedded models.toml must be valid")
    }

    /// Parse a TOML string into a registry.
    pub fn from_toml(toml_str: &str) -> Result<Self, String> {
        let file: CatalogFile =
            toml::from_str(toml_str).map_err(|e| format!("models TOML parse error: {e}"))?;

        let mut models = HashMap::new();
        let mut providers = HashMap::new();

        for (prov_name, pe) in &file.providers {
            providers.insert(
                prov_name.clone(),
                ProviderInfo {
                    name: prov_name.clone(),
                    default_model: pe.default_model.clone(),
                    api_key_env: pe.api_key_env.clone(),
                    cache_read_multiplier: pe.cache_read_multiplier,
                    cache_creation_multiplier: pe.cache_creation_multiplier,
                },
            );

            for (model_id, me) in &pe.models {
                let info = ModelInfo {
                    id: model_id.clone(),
                    provider: prov_name.clone(),
                    pricing: CostRates {
                        input_per_million: me.input,
                        output_per_million: me.output,
                        cache_read_multiplier: me.cache_read_multiplier.or(pe.cache_read_multiplier),
                        cache_creation_multiplier: me
                            .cache_creation_multiplier
                            .or(pe.cache_creation_multiplier),
                    },
                    context_window: me.context_window,
                    supports_tool_use: me.supports_tool_use,
                    supports_vision: me.supports_vision,
                };
                models.insert(make_key(prov_name, model_id), info);
            }
        }

        Ok(Self { models, providers })
    }

    /// Merge another registry on top (overrides win).
    pub fn merge(&mut self, other: Self) {
        for (key, info) in other.models {
            self.models.insert(key, info);
        }
        for (key, info) in other.providers {
            if let Some(existing) = self.providers.get_mut(&key) {
                if info.default_model.is_some() {
                    existing.default_model = info.default_model;
                }
                if info.api_key_env.is_some() {
                    existing.api_key_env = info.api_key_env;
                }
                if info.cache_read_multiplier.is_some() {
                    existing.cache_read_multiplier = info.cache_read_multiplier;
                }
                if info.cache_creation_multiplier.is_some() {
                    existing.cache_creation_multiplier = info.cache_creation_multiplier;
                }
            } else {
                self.providers.insert(key, info);
            }
        }
    }

    // ── Model lookups ─────────────────────────────────────────────────

    /// Exact-match lookup.
    pub fn get(&self, provider: &str, model: &str) -> Option<&ModelInfo> {
        self.models.get(&make_key(provider, model))
    }

    /// Fuzzy lookup: tries exact match first, then substring matching
    /// against all models for the given provider.
    pub fn get_fuzzy(&self, provider: &str, model: &str) -> Option<&ModelInfo> {
        if let Some(info) = self.get(provider, model) {
            return Some(info);
        }

        let prefix = format!("{provider}::");

        let mut best: Option<(&str, &ModelInfo)> = None;
        for (key, info) in &self.models {
            if let Some(registered) = key.strip_prefix(&prefix) {
                if model.contains(registered) || registered.contains(model) {
                    let dominated = best
                        .map(|(prev, _)| registered.len() > prev.len())
                        .unwrap_or(true);
                    if dominated {
                        best = Some((registered, info));
                    }
                }
            }
        }
        if let Some((matched, info)) = best {
            debug!(provider, model, matched, "fuzzy model match");
            return Some(info);
        }

        None
    }

    /// Get pricing for a model (convenience wrapper returning just `CostRates`).
    /// Falls back to provider-level cache multipliers for unknown models.
    pub fn get_pricing(&self, provider: &str, model: &str) -> Option<CostRates> {
        if let Some(info) = self.get_fuzzy(provider, model) {
            return Some(info.pricing.clone());
        }

        self.providers.get(provider).and_then(|p| {
            if p.cache_read_multiplier.is_some() || p.cache_creation_multiplier.is_some() {
                Some(CostRates {
                    input_per_million: 0.0,
                    output_per_million: 0.0,
                    cache_read_multiplier: p.cache_read_multiplier,
                    cache_creation_multiplier: p.cache_creation_multiplier,
                })
            } else {
                None
            }
        })
    }

    // ── Provider lookups ──────────────────────────────────────────────

    /// Get provider metadata.
    pub fn provider(&self, name: &str) -> Option<&ProviderInfo> {
        self.providers.get(name)
    }

    /// List all known provider names, sorted alphabetically.
    pub fn provider_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.providers.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    /// Get the default model for a provider.
    pub fn default_model(&self, provider: &str) -> Option<&str> {
        self.providers
            .get(provider)
            .and_then(|p| p.default_model.as_deref())
    }

    /// Get the API key env var for a provider.
    pub fn api_key_env(&self, provider: &str) -> Option<&str> {
        self.providers
            .get(provider)
            .and_then(|p| p.api_key_env.as_deref())
    }

    /// List all model IDs for a provider, sorted alphabetically.
    pub fn models_for_provider(&self, provider: &str) -> Vec<&str> {
        let prefix = format!("{provider}::");
        let mut out: Vec<&str> = self
            .models
            .iter()
            .filter_map(|(key, info)| {
                if key.starts_with(&prefix) {
                    Some(info.id.as_str())
                } else {
                    None
                }
            })
            .collect();
        out.sort();
        out
    }

    /// Get a map of provider → model list, suitable for the settings API.
    pub fn models_by_provider(&self) -> HashMap<String, Vec<String>> {
        let mut result: HashMap<String, Vec<String>> = HashMap::new();
        for prov in self.providers.keys() {
            result.insert(
                prov.clone(),
                self.models_for_provider(prov)
                    .into_iter()
                    .map(String::from)
                    .collect(),
            );
        }
        result
    }

    /// Number of models in the registry.
    pub fn len(&self) -> usize {
        self.models.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Backward-compatible alias.
pub type PricingRegistry = ModelRegistry;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_load_successfully() {
        let reg = ModelRegistry::with_defaults();
        assert!(!reg.is_empty());
    }

    #[test]
    fn exact_match() {
        let reg = ModelRegistry::with_defaults();
        let info = reg.get("anthropic", "claude-sonnet-4-5").unwrap();
        assert!((info.pricing.input_per_million - 3.0).abs() < 1e-9);
        assert!((info.pricing.output_per_million - 15.0).abs() < 1e-9);
        assert!((info.pricing.cache_read_multiplier.unwrap() - 0.1).abs() < 1e-9);
        assert!((info.pricing.cache_creation_multiplier.unwrap() - 1.25).abs() < 1e-9);
        assert_eq!(info.context_window, Some(200_000));
        assert!(info.supports_tool_use);
        assert!(info.supports_vision);
    }

    #[test]
    fn fuzzy_match_longer_model_id() {
        let reg = ModelRegistry::with_defaults();
        let info = reg.get_fuzzy("anthropic", "claude-sonnet-4-5-20250514").unwrap();
        assert!((info.pricing.input_per_million - 3.0).abs() < 1e-9);
    }

    #[test]
    fn fuzzy_match_picks_most_specific() {
        let mut reg = ModelRegistry::new();
        let short_key = make_key("test", "claude-sonnet");
        reg.models.insert(short_key, ModelInfo {
            id: "claude-sonnet".into(),
            provider: "test".into(),
            pricing: CostRates {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_multiplier: None,
                cache_creation_multiplier: None,
            },
            context_window: None,
            supports_tool_use: true,
            supports_vision: false,
        });
        let long_key = make_key("test", "claude-sonnet-4-5");
        reg.models.insert(long_key, ModelInfo {
            id: "claude-sonnet-4-5".into(),
            provider: "test".into(),
            pricing: CostRates {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_multiplier: None,
                cache_creation_multiplier: None,
            },
            context_window: None,
            supports_tool_use: true,
            supports_vision: false,
        });
        let info = reg.get_fuzzy("test", "claude-sonnet-4-5-20250514").unwrap();
        assert!((info.pricing.input_per_million - 3.0).abs() < 1e-9);
    }

    #[test]
    fn provider_default_cache_multipliers() {
        let reg = ModelRegistry::with_defaults();
        let pricing = reg.get_pricing("anthropic", "claude-unknown-99").unwrap();
        assert!((pricing.cache_read_multiplier.unwrap() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn merge_overrides() {
        let mut base = ModelRegistry::with_defaults();
        let overrides = ModelRegistry::from_toml(r#"
[anthropic.models.claude-sonnet-4-5]
input = 99.0
output = 99.0
"#).unwrap();
        base.merge(overrides);
        let info = base.get("anthropic", "claude-sonnet-4-5").unwrap();
        assert!((info.pricing.input_per_million - 99.0).abs() < 1e-9);
    }

    #[test]
    fn openai_cache_rates() {
        let reg = ModelRegistry::with_defaults();
        let info = reg.get("openai", "gpt-4o").unwrap();
        assert!((info.pricing.cache_read_multiplier.unwrap() - 0.1).abs() < 1e-9);
        assert!((info.pricing.cache_creation_multiplier.unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn gemini_cache_rates() {
        let reg = ModelRegistry::with_defaults();
        let info = reg.get_fuzzy("gemini", "gemini-2-5-flash").unwrap();
        assert!((info.pricing.cache_read_multiplier.unwrap() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn from_toml_custom() {
        let toml = r#"
[custom]
cache_read_multiplier = 0.3

[custom.models.my-model]
input = 5.0
output = 20.0
"#;
        let reg = ModelRegistry::from_toml(toml).unwrap();
        let info = reg.get("custom", "my-model").unwrap();
        assert!((info.pricing.input_per_million - 5.0).abs() < 1e-9);
        assert!((info.pricing.cache_read_multiplier.unwrap() - 0.3).abs() < 1e-9);
        assert!(info.pricing.cache_creation_multiplier.is_none());
    }

    #[test]
    fn per_model_cache_override() {
        let toml = r#"
[prov]
cache_read_multiplier = 0.1
cache_creation_multiplier = 1.25

[prov.models.special]
input = 10.0
output = 50.0
cache_read_multiplier = 0.05
"#;
        let reg = ModelRegistry::from_toml(toml).unwrap();
        let info = reg.get("prov", "special").unwrap();
        assert!((info.pricing.cache_read_multiplier.unwrap() - 0.05).abs() < 1e-9);
        assert!((info.pricing.cache_creation_multiplier.unwrap() - 1.25).abs() < 1e-9);
    }

    #[test]
    fn empty_provider_no_panic() {
        let toml = r#"
[empty]
"#;
        let reg = ModelRegistry::from_toml(toml).unwrap();
        assert!(reg.get("empty", "anything").is_none());
        assert!(reg.get_fuzzy("empty", "anything").is_none());
    }

    // ── Provider metadata ─────────────────────────────────────────────

    #[test]
    fn default_model_per_provider() {
        let reg = ModelRegistry::with_defaults();
        assert_eq!(reg.default_model("anthropic"), Some("claude-haiku-4-5"));
        assert_eq!(reg.default_model("openai"), Some("gpt-4o"));
        assert_eq!(reg.default_model("gemini"), Some("gemini-2.5-pro"));
        assert_eq!(reg.default_model("groq"), Some("llama-3.3-70b-versatile"));
        assert_eq!(reg.default_model("deepseek"), Some("deepseek-chat"));
        assert_eq!(reg.default_model("ollama"), Some("llama3.3"));
    }

    #[test]
    fn api_key_env_per_provider() {
        let reg = ModelRegistry::with_defaults();
        assert_eq!(reg.api_key_env("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(reg.api_key_env("openai"), Some("OPENAI_API_KEY"));
        assert_eq!(reg.api_key_env("ollama"), None);
    }

    #[test]
    fn models_for_provider_lists_all() {
        let reg = ModelRegistry::with_defaults();
        let anthropic = reg.models_for_provider("anthropic");
        assert!(anthropic.contains(&"claude-haiku-4-5"));
        assert!(anthropic.contains(&"claude-sonnet-4-6"));
        assert!(anthropic.contains(&"claude-opus-4-6"));
        assert!(anthropic.len() >= 4);
    }

    #[test]
    fn models_by_provider_for_settings_api() {
        let reg = ModelRegistry::with_defaults();
        let map = reg.models_by_provider();
        assert!(map.contains_key("anthropic"));
        assert!(map.contains_key("openai"));
        assert!(map.contains_key("ollama"));
        assert!(map["ollama"].is_empty());
    }

    #[test]
    fn provider_names_returns_all() {
        let reg = ModelRegistry::with_defaults();
        let names = reg.provider_names();
        assert!(names.contains(&"anthropic"));
        assert!(names.contains(&"openai"));
        assert!(names.contains(&"gemini"));
        assert!(names.contains(&"groq"));
        assert!(names.contains(&"deepseek"));
        assert!(names.contains(&"openrouter"));
        assert!(names.contains(&"ollama"));
    }

    #[test]
    fn model_capabilities() {
        let reg = ModelRegistry::with_defaults();
        let haiku = reg.get("anthropic", "claude-haiku-4-5").unwrap();
        assert!(haiku.supports_tool_use);
        assert!(haiku.supports_vision);

        let gpt41 = reg.get("openai", "gpt-4.1").unwrap();
        assert!(gpt41.supports_tool_use);
        assert!(!gpt41.supports_vision);
    }
}
