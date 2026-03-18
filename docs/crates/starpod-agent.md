# starpod-agent

The orchestrator crate that wires all subsystems together. Provides `StarpodAgent` — the central type for all chat interactions.

## API

```rust
let config = load_agent_config(&paths)?;
let agent = StarpodAgent::new(config).await?;

// Non-streaming chat
let response = agent.chat(ChatMessage {
    text: "Hello!".into(),
    user_id: None,
    channel_id: Some("main".into()),
    channel_session_key: Some("session-uuid".into()),
    attachments: vec![],
}).await?;

// Streaming chat
let (stream, session_id, followup_tx) = agent.chat_stream(&message).await?;
// stream is a Query (tokio Stream of Message)
// followup_tx can inject messages into the running agent loop

// Finalize after streaming
agent.finalize_chat(&session_id, &user_text, &result_text, &result).await;
```

## Chat Pipeline

1. **Snapshot config** — take a cheap clone of the current config (supports hot reload)
2. **Resolve channel** — map `ChatMessage` → `(Channel, key)`
3. **Resolve session** — find or create session via `SessionManager`; export closed session transcript to memory if applicable
4. **Bootstrap context** — memory files + daily logs
4. **Build system prompt** — identity + context + skills + tools + time
5. **Build provider** — construct `LlmProvider` from `config.provider` (anthropic, openai, gemini, groq, deepseek, openrouter, ollama)
6. **Run agent-sdk query** — agentic loop with custom tools via the selected provider + automatic conversation compaction
7. **Drain followup messages** — at each iteration boundary, inject any queued user messages (when `followup_mode = "inject"`)
8. **Record usage** — tokens and cost to session database
9. **Append daily log** — conversation summary

## Followup Message Handling

When a user sends a message while a stream is active, behavior depends on `followup_mode`:

- **`inject`** (default) — Messages are sent through a channel and drained at the next agent loop iteration boundary (before the next API call). Multiple rapid messages are batched into a single user message.
- **`queue`** — Messages are buffered. After the current stream finishes, all queued messages are combined and dispatched as a new agent loop.

Conversation compaction is enabled by default with a 160k token context budget. The compaction model is configurable via `compaction_model` in `agent.toml` (defaults to the primary model).

## Custom Tools (20)

| Category | Tools |
|----------|-------|
| Memory | `MemorySearch`, `MemoryWrite`, `MemoryAppendDaily` |
| Environment | `EnvGet` |
| Files | `FileRead`, `FileWrite`, `FileList`, `FileDelete` |
| Skills | `SkillActivate`, `SkillCreate`, `SkillUpdate`, `SkillDelete`, `SkillList` |
| Cron | `CronAdd`, `CronList`, `CronRemove`, `CronRuns`, `CronRun`, `CronUpdate` |
| Heartbeat | `HeartbeatWake` |

## Scheduler Integration

```rust
let agent = Arc::new(agent);
let handle = agent.start_scheduler(Some(notifier));
// Runs in background, executing due cron jobs through agent.chat()
```

## Config Hot Reload

The agent's config is wrapped in `RwLock` for hot reload support. Each request snapshots the config at the start, so config changes take effect on the next request.

```rust
// Reload config (called by the gateway's file watcher)
agent.reload_config(new_config);

// Get current config snapshot
let config = agent.config(); // returns owned StarpodConfig
```

## Component Accessors

```rust
agent.memory()      // &Arc<MemoryStore>
agent.session_mgr() // &Arc<SessionManager>
agent.skills()      // &Arc<SkillStore>
agent.cron()        // &Arc<CronStore>
agent.config()      // StarpodConfig (owned snapshot)
```

## Tests

11+ unit tests covering agent construction, custom tools, attachments, config reload, and session export.
