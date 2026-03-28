# starpod-core

Shared types, configuration loading, and error handling for all Starpod crates.

## StarpodConfig

The central configuration type. Config is loaded via workspace-aware functions that operate on `ResolvedPaths`:

```rust
use starpod_core::{load_agent_config, reload_agent_config, ResolvedPaths};

// Load config from resolved paths (workspace or single-agent)
let agent_config = load_agent_config(&paths)?;
let starpod_config = agent_config.into_starpod_config(&paths);

// Reload for hot-reload (called by file watcher)
let agent_config = reload_agent_config(&paths)?;
```

### Public Fields

Config fields are public (not getter methods):

```rust
config.model          // String — "claude-haiku-4-5"
config.max_turns      // u32 — 30
config.server_addr    // String — "127.0.0.1:3000"
config.agent_name     // String — "Aster"
config.provider       // String — "anthropic"
config.timezone       // Option<String>
config.max_tokens     // Option<u32>
config.db_dir         // PathBuf to .starpod/db/
```

### Resolved Values

Methods that check both config and environment variables:

```rust
config.resolved_api_key()                     // config || ANTHROPIC_API_KEY
config.resolved_telegram_token()              // config || TELEGRAM_BOT_TOKEN
config.resolved_telegram_allowed_user_ids()   // Vec<u64>
config.resolved_telegram_allowed_usernames()  // Vec<String>

// Multi-provider resolution
config.resolved_provider_api_key("openai")    // config || OPENAI_API_KEY
config.resolved_provider_base_url("openai")   // config || default endpoint
```

Provider API key env vars: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GROQ_API_KEY`, `DEEPSEEK_API_KEY`, `OPENROUTER_API_KEY`. Ollama requires no key.

## Blueprint Functions

### `apply_blueprint`

Copy a workspace blueprint into an instance directory (used by `starpod dev`):

```rust
use starpod_core::{apply_blueprint, EnvSource};

apply_blueprint(&blueprint_dir, &instance_dir, &workspace_dir, EnvSource::Dev)?;
```

### `build_standalone`

Build a self-contained `.starpod/` from an agent blueprint without a workspace (used by `starpod build`):

```rust
use starpod_core::build_standalone;

build_standalone(
    &blueprint_dir,           // must contain agent.toml
    &output_dir,              // .starpod/ created here
    Some(&skills_dir),        // optional skills to include
    Some(&env_file),          // optional .env to include
    false,                    // force: overwrite existing .starpod/
)?;
```

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
    pub telegram: Option<TelegramChannelConfig>,
}
```

## TelegramChannelConfig

Controls the Telegram channel:

```rust
pub struct TelegramChannelConfig {
    pub enabled: bool,                      // default: true
    pub gap_minutes: Option<i64>,           // default: Some(360) (6h)
    pub allowed_users: Vec<AllowedUser>,    // numeric IDs or usernames
    pub stream_mode: String,                // default: "final_only"
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

## ChatMessage

The input type for `StarpodAgent::chat()`:

```rust
pub struct ChatMessage {
    pub text: String,
    pub user_id: Option<String>,
    pub channel_id: Option<String>,
    pub channel_session_key: Option<String>,
    pub attachments: Vec<Attachment>,
}
```

## ChatResponse

```rust
pub struct ChatResponse {
    pub text: String,
    pub session_id: String,
    pub usage: Option<ChatUsage>,
    /// Files the agent attached for delivery to the user (via the `Attach` tool).
    /// Empty when no files were attached.
    pub attachments: Vec<Attachment>,
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
    Io(std::io::Error),
    Vault(String),
    Session(String),
    Agent(String),
    Skill(String),
    Cron(String),
    Instance(String),
    Channel(String),
    Serialization(serde_json::Error),
}
```
