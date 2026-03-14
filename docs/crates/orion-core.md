# orion-core

Shared types, configuration loading, and error handling for all Orion crates.

## OrionConfig

The central configuration type, loaded from `.orion/config.toml`:

```rust
let config = OrionConfig::load()?;

// Accessors
config.model()          // "claude-haiku-4-5"
config.max_turns()      // 30
config.server_addr()    // "127.0.0.1:3000"
config.data_dir()       // PathBuf to .orion/data/
config.project_root()   // PathBuf to project root
```

### Config Discovery

```rust
// Walk up to find nearest .orion/ directory
OrionConfig::find_project_root()   // -> Option<PathBuf>

// Load from discovered location
OrionConfig::load()                // -> Result<OrionConfig>

// Load from specific path
OrionConfig::load_from(path)       // -> Result<OrionConfig>

// Initialize a new project
OrionConfig::init(dir, content)    // -> Result<()>
```

### Resolved Values

Methods that check both config and environment variables:

```rust
config.resolved_api_key()                     // config || ANTHROPIC_API_KEY
config.resolved_telegram_token()              // config || TELEGRAM_BOT_TOKEN
config.resolved_telegram_allowed_users()      // &[u64]
config.resolved_db_path()                     // data_dir/memory.db

// Multi-provider resolution
config.resolved_provider_api_key("openai")    // config || OPENAI_API_KEY
config.resolved_provider_base_url("openai")   // config || default endpoint
```

Provider API key env vars: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GROQ_API_KEY`, `DEEPSEEK_API_KEY`, `OPENROUTER_API_KEY`. Ollama requires no key.

## ChatMessage

The input type for `OrionAgent::chat()`:

```rust
pub struct ChatMessage {
    pub text: String,
    pub user_id: Option<String>,
    pub channel_id: Option<String>,
    pub channel_session_key: Option<String>,
    pub attachments: Vec<String>,
}
```

## ChatResponse

```rust
pub struct ChatResponse {
    pub text: String,
    pub session_id: String,
    pub usage: Option<ChatUsage>,
}
```

## ChatUsage

```rust
pub struct ChatUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
}
```

## OrionError

Unified error type for the workspace:

```rust
pub enum OrionError {
    Config(String),
    Database(String),
    IO(std::io::Error),
    Vault(String),
    Session(String),
    Agent(String),
    Channel(String),
    Serialization(String),
}
```
