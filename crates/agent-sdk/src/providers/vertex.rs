//! Google Cloud Vertex AI provider for Claude models.
//!
//! Uses Vertex AI's rawPredict / streamRawPredict endpoints which accept the
//! same Messages API format as direct Anthropic, with Google OAuth2 Bearer
//! token authentication and a different URL structure.

use std::env;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use gcp_auth::TokenProvider;
use reqwest::StatusCode;
use tokio::time::sleep;
use tracing::warn;

use crate::client::{CreateMessageRequest, MessageResponse, RetryConfig, StreamEvent};
use crate::error::{AgentError, Result};
use crate::models::ModelRegistry;
use crate::provider::{CostRates, LlmProvider, ProviderCapabilities};

use super::anthropic::{sse_stream, strip_tool_result_names, AnthropicProvider};

const VERTEX_API_VERSION: &str = "vertex-2023-10-16";

/// Google Cloud Vertex AI provider for Claude models.
///
/// Routes Claude API calls through Vertex AI using Google OAuth2 authentication.
/// The request/response format is identical to the Anthropic Messages API —
/// only authentication and URL structure differ. Streaming uses standard SSE,
/// the same as the direct Anthropic API.
pub struct VertexProvider {
    http: reqwest::Client,
    project_id: String,
    region: String,
    auth: Arc<dyn TokenProvider>,
    retry_config: RetryConfig,
    pricing: Option<Arc<ModelRegistry>>,
}

impl VertexProvider {
    /// Create from environment / Application Default Credentials.
    ///
    /// Reads `GOOGLE_CLOUD_PROJECT` (or `GCP_PROJECT_ID`) for the project ID,
    /// and `GOOGLE_CLOUD_LOCATION` (or `GCP_REGION`, default `us-central1`)
    /// for the region. Credentials are discovered automatically via ADC.
    pub async fn from_env() -> Result<Self> {
        let project_id = env::var("GOOGLE_CLOUD_PROJECT")
            .or_else(|_| env::var("GCP_PROJECT_ID"))
            .map_err(|_| {
                AgentError::AuthenticationFailed(
                    "GOOGLE_CLOUD_PROJECT or GCP_PROJECT_ID environment variable is not set".into(),
                )
            })?;
        let region = env::var("GOOGLE_CLOUD_LOCATION")
            .or_else(|_| env::var("GCP_REGION"))
            .unwrap_or_else(|_| "us-central1".to_string());

        Self::new(project_id, region).await
    }

    /// Create with explicit project ID and region. Credentials are discovered
    /// automatically via Application Default Credentials (ADC).
    pub async fn new(project_id: impl Into<String>, region: impl Into<String>) -> Result<Self> {
        let auth = gcp_auth::provider().await.map_err(|e| {
            AgentError::AuthenticationFailed(format!("Google Cloud auth error: {e}"))
        })?;

        Ok(Self {
            http: reqwest::Client::new(),
            project_id: project_id.into(),
            region: region.into(),
            auth,
            retry_config: RetryConfig::default(),
            pricing: None,
        })
    }

    /// Create with a pre-existing token provider (useful for testing or
    /// sharing auth across multiple providers).
    pub fn with_auth(
        project_id: impl Into<String>,
        region: impl Into<String>,
        auth: Arc<dyn TokenProvider>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            project_id: project_id.into(),
            region: region.into(),
            auth,
            retry_config: RetryConfig::default(),
            pricing: None,
        }
    }

    /// Attach a pricing registry for cost lookups.
    pub fn with_pricing(mut self, registry: Arc<ModelRegistry>) -> Self {
        self.pricing = Some(registry);
        self
    }

    /// Override the default retry configuration.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Build the rawPredict URL for a given model ID.
    fn invoke_url(&self, model_id: &str) -> String {
        format!(
            "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/anthropic/models/{model}:rawPredict",
            region = self.region,
            project = self.project_id,
            model = model_id,
        )
    }

    /// Build the streamRawPredict URL for a given model ID.
    fn invoke_stream_url(&self, model_id: &str) -> String {
        format!(
            "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/anthropic/models/{model}:streamRawPredict",
            region = self.region,
            project = self.project_id,
            model = model_id,
        )
    }

    /// Build the Vertex AI request body.
    ///
    /// Same as Anthropic Messages API but with `anthropic_version` in the body
    /// instead of as a header, and without the `model` field (model is in the URL).
    fn build_body(
        &self,
        request: &CreateMessageRequest,
        streaming: bool,
    ) -> Result<serde_json::Value> {
        let mut req = request.clone();
        req.stream = streaming;
        strip_tool_result_names(&mut req);

        let mut body = serde_json::to_value(&req)?;

        if let Some(obj) = body.as_object_mut() {
            // Model is specified in the URL path, not in the body
            obj.remove("model");
            // Add Vertex-specific anthropic_version
            obj.insert(
                "anthropic_version".to_string(),
                serde_json::Value::String(VERTEX_API_VERSION.to_string()),
            );
        }

        Ok(body)
    }

    /// Get a fresh OAuth2 Bearer token.
    async fn get_token(&self) -> Result<String> {
        let scopes = &["https://www.googleapis.com/auth/cloud-platform"];
        let token = self.auth.token(scopes).await.map_err(|e| {
            AgentError::AuthenticationFailed(format!("Google Cloud token error: {e}"))
        })?;
        Ok(token.as_str().to_string())
    }

    /// Send a POST request with OAuth2 Bearer auth and retry logic.
    async fn send_with_auth(&self, url: &str, body: &[u8]) -> Result<reqwest::Response> {
        let mut attempt: u32 = 0;

        loop {
            let token = self.get_token().await?;

            let response = self
                .http
                .post(url)
                .header("content-type", "application/json")
                .bearer_auth(&token)
                .body(body.to_vec())
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                return Ok(response);
            }

            let error_body = response.bytes().await.unwrap_or_default();
            let error_text = String::from_utf8_lossy(&error_body);

            if AnthropicProvider::is_retryable(status) && attempt < self.retry_config.max_retries {
                let wait = self.backoff_duration(attempt);
                warn!(
                    status = status.as_u16(),
                    attempt,
                    wait_secs = wait.as_secs_f64(),
                    "Retryable Vertex AI API error, backing off"
                );
                sleep(wait).await;
                attempt += 1;
                continue;
            }

            return Err(Self::status_to_error(status, &error_text));
        }
    }

    fn backoff_duration(&self, attempt: u32) -> Duration {
        let secs = self.retry_config.initial_backoff.as_secs_f64()
            * self.retry_config.backoff_multiplier.powi(attempt as i32);
        let max_secs = self.retry_config.max_backoff.as_secs_f64();
        Duration::from_secs_f64(secs.min(max_secs))
    }

    fn status_to_error(status: StatusCode, body: &str) -> AgentError {
        AnthropicProvider::status_to_error(status, body)
    }
}

#[async_trait]
impl LlmProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_use: true,
            thinking: true,
            prompt_caching: true,
        }
    }

    fn cost_rates(&self, model: &str) -> CostRates {
        if let Some(ref registry) = self.pricing {
            if let Some(rates) = registry.get_pricing("vertex", model) {
                return rates;
            }
            // Fall back to anthropic pricing since Vertex uses same models
            if let Some(rates) = registry.get_pricing("anthropic", model) {
                return rates;
            }
        }
        // Hardcoded fallback — same as Anthropic (Vertex pricing is identical)
        let cache = (Some(0.1), Some(1.25));
        match model {
            m if m.contains("opus") => CostRates {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            m if m.contains("sonnet") => CostRates {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            m if m.contains("haiku") => CostRates {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
            _ => CostRates {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_multiplier: cache.0,
                cache_creation_multiplier: cache.1,
            },
        }
    }

    async fn create_message(&self, request: &CreateMessageRequest) -> Result<MessageResponse> {
        let body = self.build_body(request, false)?;
        let body_bytes = serde_json::to_vec(&body)?;
        let url = self.invoke_url(&request.model);

        let response = self.send_with_auth(&url, &body_bytes).await?;
        let msg: MessageResponse = response.json().await?;
        Ok(msg)
    }

    async fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        let body = self.build_body(request, true)?;
        let body_bytes = serde_json::to_vec(&body)?;
        let url = self.invoke_stream_url(&request.model);

        let response = self.send_with_auth(&url, &body_bytes).await?;

        // Vertex AI uses standard SSE — same format as direct Anthropic
        let byte_stream = response.bytes_stream();
        let event_stream = sse_stream(byte_stream);

        Ok(Box::pin(event_stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ApiContentBlock, ApiMessage, CreateMessageRequest};

    /// Dummy token provider for unit tests (never actually called).
    struct DummyTokenProvider;

    #[async_trait]
    impl TokenProvider for DummyTokenProvider {
        async fn token(
            &self,
            _scopes: &[&str],
        ) -> std::result::Result<Arc<gcp_auth::Token>, gcp_auth::Error> {
            unimplemented!("DummyTokenProvider should not be called in unit tests")
        }
        async fn project_id(&self) -> std::result::Result<Arc<str>, gcp_auth::Error> {
            Ok(Arc::from("test-project-123"))
        }
    }

    /// Helper to create a VertexProvider without real GCP credentials (for unit tests).
    fn test_provider() -> VertexProvider {
        VertexProvider::with_auth(
            "test-project-123",
            "us-central1",
            Arc::new(DummyTokenProvider),
        )
    }

    fn test_request() -> CreateMessageRequest {
        CreateMessageRequest {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 1024,
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::Text {
                    text: "Hello, world!".to_string(),
                    cache_control: None,
                }],
            }],
            system: None,
            tools: None,
            stream: false,
            metadata: None,
            thinking: None,
        }
    }

    #[test]
    fn provider_name_is_vertex() {
        let provider = test_provider();
        assert_eq!(provider.name(), "vertex");
    }

    #[test]
    fn capabilities_all_enabled() {
        let provider = test_provider();
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(caps.tool_use);
        assert!(caps.thinking);
        assert!(caps.prompt_caching);
    }

    #[test]
    fn invoke_url_format() {
        let provider = test_provider();
        let url = provider.invoke_url("claude-sonnet-4-6");
        assert_eq!(
            url,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/test-project-123/locations/us-central1/publishers/anthropic/models/claude-sonnet-4-6:rawPredict"
        );
    }

    #[test]
    fn invoke_stream_url_format() {
        let provider = test_provider();
        let url = provider.invoke_stream_url("claude-sonnet-4-6");
        assert_eq!(
            url,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/test-project-123/locations/us-central1/publishers/anthropic/models/claude-sonnet-4-6:streamRawPredict"
        );
    }

    #[test]
    fn invoke_url_global_region() {
        let mut provider = test_provider();
        provider.region = "global".to_string();
        let url = provider.invoke_url("claude-sonnet-4-6");
        assert!(url.starts_with("https://global-aiplatform.googleapis.com/"));
        assert!(url.contains("/projects/test-project-123/"));
        assert!(url.ends_with(":rawPredict"));
    }

    #[test]
    fn invoke_url_different_regions() {
        for region in &["us-east1", "europe-west1", "asia-southeast1", "global"] {
            let mut provider = test_provider();
            provider.region = region.to_string();
            let url = provider.invoke_url("claude-opus-4-6");
            assert!(url.contains(&format!("https://{region}-aiplatform.googleapis.com/")));
            assert!(url.contains("/publishers/anthropic/models/claude-opus-4-6:rawPredict"));
        }
    }

    #[test]
    fn invoke_url_with_versioned_model() {
        let provider = test_provider();
        let url = provider.invoke_url("claude-haiku-4-5@20251001");
        assert!(url.contains("/models/claude-haiku-4-5@20251001:rawPredict"));
    }

    #[test]
    fn build_body_removes_model_and_adds_version() {
        let provider = test_provider();
        let request = test_request();
        let body = provider.build_body(&request, false).unwrap();

        // model should be removed (it's in the URL)
        assert!(body.get("model").is_none());
        // anthropic_version should be added
        assert_eq!(
            body["anthropic_version"].as_str().unwrap(),
            "vertex-2023-10-16"
        );
    }

    #[test]
    fn build_body_preserves_stream_field() {
        let provider = test_provider();
        let request = test_request();

        // Non-streaming
        let body = provider.build_body(&request, false).unwrap();
        assert_eq!(body["stream"], false);

        // Streaming
        let body = provider.build_body(&request, true).unwrap();
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn build_body_preserves_messages_and_max_tokens() {
        let provider = test_provider();
        let request = test_request();
        let body = provider.build_body(&request, false).unwrap();

        assert_eq!(body["max_tokens"], 1024);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn build_body_strips_tool_result_names() {
        let provider = test_provider();
        let request = CreateMessageRequest {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 1024,
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: vec![ApiContentBlock::ToolResult {
                    tool_use_id: "id_123".to_string(),
                    content: serde_json::json!(""),
                    is_error: Some(false),
                    cache_control: None,
                    name: Some("my_tool".to_string()),
                }],
            }],
            system: None,
            tools: None,
            stream: false,
            metadata: None,
            thinking: None,
        };

        let body = provider.build_body(&request, false).unwrap();
        let messages = body["messages"].as_array().unwrap();
        let content = messages[0]["content"].as_array().unwrap();
        // The "name" field should be stripped from tool_result blocks
        assert!(content[0].get("name").is_none());
    }

    #[test]
    fn cost_rates_sonnet() {
        let provider = test_provider();
        let rates = provider.cost_rates("claude-sonnet-4-6");
        assert_eq!(rates.input_per_million, 3.0);
        assert_eq!(rates.output_per_million, 15.0);
    }

    #[test]
    fn cost_rates_opus() {
        let provider = test_provider();
        let rates = provider.cost_rates("claude-opus-4-6");
        assert_eq!(rates.input_per_million, 5.0);
        assert_eq!(rates.output_per_million, 25.0);
    }

    #[test]
    fn cost_rates_haiku() {
        let provider = test_provider();
        let rates = provider.cost_rates("claude-haiku-4-5@20251001");
        assert_eq!(rates.input_per_million, 1.0);
        assert_eq!(rates.output_per_million, 5.0);
    }

    #[test]
    fn cost_rates_unknown_defaults_to_sonnet() {
        let provider = test_provider();
        let rates = provider.cost_rates("unknown-model-xyz");
        assert_eq!(rates.input_per_million, 3.0);
        assert_eq!(rates.output_per_million, 15.0);
    }

    #[test]
    fn backoff_duration_increases() {
        let provider = test_provider();
        let d0 = provider.backoff_duration(0);
        let d1 = provider.backoff_duration(1);
        let d2 = provider.backoff_duration(2);
        assert!(d1 > d0);
        assert!(d2 > d1);
    }
}
