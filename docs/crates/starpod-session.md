# starpod-session

Channel-aware session lifecycle management with usage tracking and message persistence.

## API

```rust
let mgr = SessionManager::new(&db_path, &sessions_dir).await?;

// Resolve or create a session
let decision = mgr.resolve_session(&Channel::Main, "session-key", None).await?;
match decision {
    SessionDecision::Continue(id) => { /* use existing session */ }
    SessionDecision::New { closed_session_id } => {
        // closed_session_id is Some(id) when a previous session was auto-closed
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
- **Telegram**: Continues if last message within the gap threshold; otherwise auto-closes old session and returns `New { closed_session_id: Some(id) }`

The Telegram inactivity threshold defaults to 6 hours (360 minutes) and is configurable via `[channels.telegram] gap_minutes` in `agent.toml`.

### Session Export on Close

When a session is auto-closed, `closed_session_id` is returned so the caller (typically `StarpodAgent`) can export the transcript to memory. See [Memory — Session Transcript Export](/concepts/memory#session-transcript-export).

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
    pub user_id: String,
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

15+ unit tests covering channel resolution, time-gap auto-close, session isolation, usage tracking, compaction logging, and closed session ID propagation.
