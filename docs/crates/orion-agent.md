# orion-agent

The orchestrator crate that wires all subsystems together. Provides `OrionAgent` — the central type for all chat interactions.

## API

```rust
let config = OrionConfig::load()?;
let agent = OrionAgent::new(config).await?;

// Non-streaming chat
let response = agent.chat(ChatMessage {
    text: "Hello!".into(),
    user_id: None,
    channel_id: Some("main".into()),
    channel_session_key: Some("session-uuid".into()),
    attachments: vec![],
}).await?;

// Streaming chat
let (stream, session_id) = agent.chat_stream(&message).await?;
// stream is a Query (tokio Stream of Message)

// Finalize after streaming
agent.finalize_chat(&session_id, &user_text, &result_text, &result).await;
```

## Chat Pipeline

1. **Resolve channel** — map `ChatMessage` → `(Channel, key)`
2. **Resolve session** — find or create session via `SessionManager`
3. **Bootstrap context** — memory files + daily logs
4. **Build system prompt** — identity + context + skills + tools + time
5. **Run agent-sdk query** — agentic loop with custom tools + automatic conversation compaction
6. **Record usage** — tokens and cost to session database
7. **Append daily log** — conversation summary

Conversation compaction is enabled by default with a 160k token context budget. The compaction model is configurable via `compaction_model` in `.orion/config.toml` (defaults to the primary model).

## Custom Tools (13)

| Category | Tools |
|----------|-------|
| Memory | `MemorySearch`, `MemoryWrite`, `MemoryAppendDaily` |
| Vault | `VaultGet`, `VaultSet` |
| Skills | `SkillCreate`, `SkillUpdate`, `SkillDelete`, `SkillList` |
| Cron | `CronAdd`, `CronList`, `CronRemove`, `CronRuns` |

## Scheduler Integration

```rust
let agent = Arc::new(agent);
let handle = agent.start_scheduler(Some(notifier));
// Runs in background, executing due cron jobs through agent.chat()
```

## Component Accessors

```rust
agent.memory()      // &Arc<MemoryStore>
agent.session_mgr() // &Arc<SessionManager>
agent.vault()       // &Arc<Vault>
agent.skills()      // &Arc<SkillStore>
agent.cron()        // &Arc<CronStore>
agent.config()      // &OrionConfig
```

## Tests

3 unit tests.
