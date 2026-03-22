//! LLM provider trait and shared types.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

use crate::client::{CreateMessageRequest, MessageResponse, StreamEvent};
use crate::error::Result;

/// Capabilities advertised by a provider.
#[derive(Debug, Clone)]
pub struct ProviderCapabilities {
    /// Supports streaming responses.
    pub streaming: bool,
    /// Supports tool/function calling.
    pub tool_use: bool,
    /// Supports extended thinking / chain-of-thought.
    pub thinking: bool,
    /// Supports prompt caching (cache_control blocks).
    pub prompt_caching: bool,
}

/// Per-model cost rates (USD per million tokens).
#[derive(Debug, Clone)]
pub struct CostRates {
    pub input_per_million: f64,
    pub output_per_million: f64,
    /// Multiplier for cache-read tokens relative to input rate (e.g. 0.1 = 10%).
    /// `None` means cache tokens are billed at the standard input rate.
    pub cache_read_multiplier: Option<f64>,
    /// Multiplier for cache-creation tokens relative to input rate (e.g. 1.25 = 125%).
    /// `None` means cache tokens are billed at the standard input rate.
    pub cache_creation_multiplier: Option<f64>,
}

impl CostRates {
    /// Compute cost for a given number of input/output tokens (ignoring cache).
    pub fn compute(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        self.compute_with_cache(input_tokens, output_tokens, 0, 0)
    }

    /// Compute cost accounting for cached token pricing.
    ///
    /// `input_tokens` here is only the uncached portion (as returned by the API).
    /// `cache_read` and `cache_creation` are billed at their respective multiplied rates.
    pub fn compute_with_cache(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
    ) -> f64 {
        let read_rate = self.input_per_million * self.cache_read_multiplier.unwrap_or(1.0);
        let create_rate = self.input_per_million * self.cache_creation_multiplier.unwrap_or(1.0);
        (input_tokens as f64 * self.input_per_million
            + cache_read_tokens as f64 * read_rate
            + cache_creation_tokens as f64 * create_rate
            + output_tokens as f64 * self.output_per_million)
            / 1_000_000.0
    }
}

/// Trait that all LLM providers implement.
///
/// Each provider translates between the canonical API types
/// (`CreateMessageRequest`, `MessageResponse`, `StreamEvent`) and
/// its own wire format internally.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Human-readable provider name (e.g. "anthropic", "openai").
    fn name(&self) -> &str;

    /// Capabilities this provider supports.
    fn capabilities(&self) -> ProviderCapabilities;

    /// Cost rates for a given model.
    fn cost_rates(&self, model: &str) -> CostRates;

    /// Send a non-streaming request and return the complete response.
    async fn create_message(&self, request: &CreateMessageRequest) -> Result<MessageResponse>;

    /// Send a streaming request and return a stream of events.
    async fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_rates(input: f64, output: f64) -> CostRates {
        CostRates {
            input_per_million: input,
            output_per_million: output,
            cache_read_multiplier: None,
            cache_creation_multiplier: None,
        }
    }

    #[test]
    fn cost_rates_compute() {
        let rates = simple_rates(2.0, 8.0);
        // 1M input tokens at $2/M + 500K output tokens at $8/M = $2 + $4 = $6
        let cost = rates.compute(1_000_000, 500_000);
        assert!((cost - 6.0).abs() < 1e-9, "expected 6.0, got {}", cost);
    }

    #[test]
    fn cost_rates_compute_zero_tokens() {
        let rates = simple_rates(10.0, 40.0);
        let cost = rates.compute(0, 0);
        assert!((cost - 0.0).abs() < 1e-9, "expected 0.0, got {}", cost);
    }

    #[test]
    fn cost_rates_compute_small_usage() {
        let rates = simple_rates(2.5, 10.0);
        // 100 input + 50 output => (100 * 2.5 + 50 * 10.0) / 1_000_000 = 750 / 1_000_000
        let cost = rates.compute(100, 50);
        let expected = 750.0 / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-12, "expected {}, got {}", expected, cost);
    }

    #[test]
    fn cost_rates_with_cache() {
        let rates = CostRates {
            input_per_million: 3.0,  // Sonnet pricing
            output_per_million: 15.0,
            cache_read_multiplier: Some(0.1),
            cache_creation_multiplier: Some(1.25),
        };
        // 1000 uncached at $3/M + 10000 cache_read at $0.30/M + 2000 cache_creation at $3.75/M + 500 output at $15/M
        let cost = rates.compute_with_cache(1000, 500, 10_000, 2000);
        let expected = (1000.0 * 3.0 + 10_000.0 * 0.3 + 2000.0 * 3.75 + 500.0 * 15.0) / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-12, "expected {}, got {}", expected, cost);
    }
}
