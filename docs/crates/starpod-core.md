# starpod-core

Shared types, configuration loading, and error handling for all Starpod crates.

## StarpodConfig

The central configuration type, loaded from `.starpod/config.toml`:

```rust
let config = StarpodConfig::load()?;

// Accessors
config.model()          // "claude-haiku-4-5"
config.max_turns()      // 30
config.server_addr()    // "127.0.0.1:3000"
config.data_dir()       // PathBuf to .starpod/data/
config.project_root()   // PathBuf to project root
```

### Config Discovery

```rust
// Walk up to find nearest .starpod/ directory
StarpodConfig::find_project_root()   // -> Option<PathBuf>

// Load from discovered location
StarpodConfig::load()                // -> Result<StarpodConfig>

// Load from specific path
StarpodConfig::load_from(path)       // -> Result<StarpodConfig>

// Initialize a new project
StarpodConfig::init(dir, content)    // -> Result<()>
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

## AttachmentsConfig

Controls file upload handling (validated in gateway and Telegram):

```rust
pub struct AttachmentsConfig {
    pub enabled: bool,                    // default: true
    pub allowed_extensions: Vec<String>,  // default: [] (all allowed)
    pub max_file_size: usize,             // default: 20 MB
}

// Validate an attachment against the config
config.attachments.validate("photo.jpg", raw_size)?;
```

## ChannelsConfig

Container for per-channel configuration:

```rust
pub struct ChannelsConfig {
    pub telegram: TelegramChannelConfig,
}
```

## TelegramChannelConfig

Controls the Telegram channel:

```rust
pub struct TelegramChannelConfig {
    pub enabled: bool,               // default: true
    pub gap_minutes: i64,            // default: 360 (6h)
    pub bot_token: Option<String>,
    pub allowed_users: Vec<u64>,
    pub stream_mode: String,         // default: "final_only"
    pub edit_throttle_ms: u64,       // default: 300
}
```

## CompactionConfig

Controls conversation compaction (summarizing older messages):

```rust
pub struct CompactionConfig {
    pub context_budget: u64,        // default: 160_000
    pub summary_max_tokens: u32,    // default: 4096
    pub min_keep_messages: usize,   // default: 4
}
```

## CronConfig

Defaults for the cron scheduling system:

```rust
pub struct CronConfig {
    pub default_max_retries: u32,   // default: 3
    pub default_timeout_secs: u64,  // default: 7200 (2h)
    pub max_concurrent_runs: usize, // default: 1
}
```

## InstancesConfig

Remote instance management settings:

```rust
pub struct InstancesConfig {
    pub health_check_interval_secs: u64,  // default: 30
    pub heartbeat_timeout_secs: u64,      // default: 90
    pub http_timeout_secs: u64,           // default: 30
}
```

## ChatMessage

The input type for `StarpodAgent::chat()`:

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

## StarpodError

Unified error type for the workspace:

```rust
pub enum StarpodError {
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
