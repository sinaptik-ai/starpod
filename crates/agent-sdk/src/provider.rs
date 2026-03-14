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
}

impl CostRates {
    /// Compute cost for a given number of input/output tokens.
    pub fn compute(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        (input_tokens as f64 * self.input_per_million
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

    #[test]
    fn cost_rates_compute() {
        let rates = CostRates {
            input_per_million: 2.0,
            output_per_million: 8.0,
        };
        // 1M input tokens at $2/M + 500K output tokens at $8/M = $2 + $4 = $6
        let cost = rates.compute(1_000_000, 500_000);
        assert!((cost - 6.0).abs() < 1e-9, "expected 6.0, got {}", cost);
    }

    #[test]
    fn cost_rates_compute_zero_tokens() {
        let rates = CostRates {
            input_per_million: 10.0,
            output_per_million: 40.0,
        };
        let cost = rates.compute(0, 0);
        assert!((cost - 0.0).abs() < 1e-9, "expected 0.0, got {}", cost);
    }

    #[test]
    fn cost_rates_compute_small_usage() {
        let rates = CostRates {
            input_per_million: 2.5,
            output_per_million: 10.0,
        };
        // 100 input + 50 output => (100 * 2.5 + 50 * 10.0) / 1_000_000 = 750 / 1_000_000
        let cost = rates.compute(100, 50);
        let expected = 750.0 / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-12, "expected {}, got {}", expected, cost);
    }
}
