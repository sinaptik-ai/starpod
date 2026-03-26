# agent-sdk

Rust port of the Claude Agent SDK. Provides the `query()` function which drives the agentic loop: prompt → LLM provider → tool execution → feed results → repeat.

## Usage

```rust
use agent_sdk::{query, Options, Message};
use tokio_stream::StreamExt;

let mut stream = query(
    "What files are in this directory?",
    Options::builder()
        .allowed_tools(vec!["Bash".into(), "Glob".into()])
        .build(),
);

while let Some(msg) = stream.next().await {
    let msg = msg?;
    if let Message::Result(result) = &msg {
        println!("{}", result.result.as_deref().unwrap_or(""));
    }
}
```

## Multi-Provider Support

The SDK supports multiple LLM providers via the `LlmProvider` trait. Each provider translates between the canonical API types and its own wire format.

### Available Providers

| Provider | Struct | Also Serves |
|----------|--------|-------------|
| Anthropic | `AnthropicProvider` | — |
| AWS Bedrock | `BedrockProvider` | — |
| Google Vertex AI | `VertexProvider` | — |
| OpenAI | `OpenAiProvider` | Groq, DeepSeek, OpenRouter, Ollama |
| Gemini | `GeminiProvider` | — |

### Using a Specific Provider

```rust
use agent_sdk::{Options, OpenAiProvider};

let provider = OpenAiProvider::new("sk-...");

let stream = query(
    "Hello!",
    Options::builder()
        .provider(Box::new(provider))
        .model("gpt-4.1")
        .build(),
);
```

### Using AWS Bedrock

```rust
use agent_sdk::{Options, BedrockProvider};

// Reads AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION from env
let provider = BedrockProvider::with_region("eu-west-1");

let stream = query(
    "Hello!",
    Options::builder()
        .provider(Box::new(provider))
        .model("eu.anthropic.claude-sonnet-4-6")
        .build(),
);
```

Bedrock uses AWS SigV4 authentication and the AWS Event Stream binary protocol for streaming. Model IDs use cross-region inference profiles (e.g. `eu.anthropic.claude-sonnet-4-6`, `us.anthropic.claude-opus-4-6-v1`).

### Using Google Vertex AI

```rust
use agent_sdk::{Options, VertexProvider};

// Uses Application Default Credentials (ADC) for auth
let provider = VertexProvider::new("my-gcp-project", "us-central1").await?;

let stream = query(
    "Hello!",
    Options::builder()
        .provider(Box::new(provider))
        .model("claude-sonnet-4-6")
        .build(),
);
```

Vertex AI uses Google OAuth2 authentication and standard SSE streaming (same as Anthropic). Credentials are discovered via Application Default Credentials: service account JSON (`GOOGLE_APPLICATION_CREDENTIALS`), `gcloud auth application-default login`, or GCE metadata. Model IDs are `claude-sonnet-4-6`, `claude-opus-4-6`, `claude-haiku-4-5@20251001`.

If no provider is set, defaults to `AnthropicProvider::from_env()`.

### Implementing a Custom Provider

```rust
use agent_sdk::{LlmProvider, ProviderCapabilities, CostRates};
use agent_sdk::client::{CreateMessageRequest, MessageResponse, StreamEvent};
use async_trait::async_trait;

#[async_trait]
impl LlmProvider for MyProvider {
    fn name(&self) -> &str { "my-provider" }
    fn capabilities(&self) -> ProviderCapabilities { /* ... */ }
    fn cost_rates(&self, model: &str) -> CostRates { /* ... */ }
    async fn create_message(&self, req: &CreateMessageRequest)
        -> Result<MessageResponse> { /* ... */ }
    async fn create_message_stream(&self, req: &CreateMessageRequest)
        -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> { /* ... */ }
}
```

## Options Builder

```rust
Options::builder()
    .model("claude-haiku-4-5")
    .max_turns(30)
    .system_prompt("You are a helpful assistant")
    .allowed_tools(vec!["Bash".into(), "Read".into()])
    .session_id("my-session")
    .thinking(ThinkingConfig::Enabled { budget_tokens: 10_000 })
    .permission_mode(PermissionMode::BypassPermissions)
    .external_tool_handler(handler_fn)
    .custom_tools(vec![tool_def])
    .provider(Box::new(my_provider))
    .build()
```

| Method | Description |
|--------|-------------|
| `.model()` | LLM model identifier |
| `.max_turns()` | Maximum agentic loop iterations |
| `.system_prompt()` | System prompt text |
| `.allowed_tools()` | Restrict available tools |
| `.session_id()` | Session identifier for multi-turn |
| `.thinking()` | Extended thinking configuration |
| `.permission_mode()` | Tool access control |
| `.external_tool_handler()` | Custom tool callback |
| `.custom_tools()` | Custom tool definitions |
| `.context_budget()` | Token threshold for conversation compaction |
| `.compaction_model()` | Model for generating compaction summaries |
| `.provider()` | LLM provider (default: Anthropic) |
| `.env_blocklist()` | Env var names to strip from Bash child processes |

## Message Types

```rust
enum Message {
    System(SystemMessage),
    Assistant(AssistantMessage),
    User(UserMessage),
    Result(ResultMessage),
    StreamEvent(StreamEventMessage),
}
```

## Custom Tools

Define tools with JSON schemas and handle them with an external callback:

```rust
let tool = CustomToolDefinition {
    name: "MyTool".into(),
    description: "Does something useful".into(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" }
        },
        "required": ["query"]
    }),
};

let handler: ExternalToolHandlerFn = Arc::new(|name, input| {
    Box::pin(async move {
        if name == "MyTool" {
            Some(ToolResult {
                content: "result".into(),
                is_error: false,
            })
        } else {
            None // Fall through to built-in handler
        }
    })
});
```

The handler returns `Some(ToolResult)` to handle a tool, or `None` to let the SDK's built-in handler run it.

## Tests

97 tests (including doc-tests).
