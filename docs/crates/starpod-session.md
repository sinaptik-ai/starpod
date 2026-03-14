# starpod-session

Channel-aware session lifecycle management with usage tracking and message persistence.

## API

```rust
let mgr = SessionManager::new(&db_path, &sessions_dir).await?;

// Resolve or create a session
let decision = mgr.resolve_session(&Channel::Main, "session-key").await?;
match decision {
    SessionDecision::Continue(id) => { /* use existing session */ }
    SessionDecision::New => {
        let id = mgr.create_session(&Channel::Main, "session-key").await?;
    }
}

// Session lifecycle
mgr.touch_session(&id).await?;              // Update timestamp
mgr.set_title_if_empty(&id, "Title").await?; // Auto-title
mgr.close_session(&id, "summary").await?;    // Close with summary

// Messages
mgr.save_message(&id, "user", "Hello").await?;
let messages = mgr.get_messages(&id).await?;

// Usage
mgr.record_usage(&id, &usage_record, turn).await?;
let summary = mgr.session_usage(&id).await?;

// Compaction logging
mgr.record_compaction(&id, "auto", 150_000, "Summary text", 12).await?;

// Listing
let sessions = mgr.list_sessions(20).await?;
let session = mgr.get_session(&id).await?;
```

## Channel Enum

```rust
pub enum Channel {
    Main,       // Explicit sessions (web, REPL, CLI)
    Telegram,   // Time-gap sessions (6h threshold)
}
```

## Session Resolution

- **Main**: Always continues if session exists with same key
- **Telegram**: Continues if last message within 6 hours; otherwise auto-closes old session and returns `New`

## Types

```rust
pub struct SessionMeta {
    pub id: String,
    pub created_at: String,
    pub last_message_at: String,
    pub is_closed: bool,
    pub summary: Option<String>,
    pub title: Option<String>,
    pub message_count: i64,
    pub channel: String,
    pub channel_session_key: Option<String>,
}

pub struct UsageRecord {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
    pub model: String,
}

pub struct UsageSummary {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read: u64,
    pub total_cache_write: u64,
    pub total_cost_usd: f64,
    pub total_turns: u32,
}
```

## Tests

12 unit tests.
