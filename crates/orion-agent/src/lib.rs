pub mod tools;

use std::sync::Arc;

use chrono::Local;
use tokio_stream::StreamExt;
use tracing::{debug, error};

use agent_sdk::{ExternalToolHandlerFn, Message, Options, PermissionMode};
use agent_sdk::options::SystemPrompt;

use orion_core::{ChatMessage, ChatResponse, ChatUsage, OrionConfig, OrionError, Result};
use orion_memory::MemoryStore;
use orion_session::{SessionDecision, SessionManager, UsageRecord};
use orion_vault::Vault;

use crate::tools::{custom_tool_definitions, handle_custom_tool};

/// The Orion agent orchestrator.
///
/// Wires together memory, sessions, vault, and the agent-sdk to provide
/// a high-level `chat()` interface.
pub struct OrionAgent {
    memory: Arc<MemoryStore>,
    session_mgr: Arc<SessionManager>,
    vault: Arc<Vault>,
    config: OrionConfig,
}

impl OrionAgent {
    /// Create a new OrionAgent from config.
    ///
    /// Initializes memory store, session manager, and vault.
    pub fn new(config: OrionConfig) -> Result<Self> {
        // Initialize memory store
        let memory = MemoryStore::new(&config.data_dir)?;

        // Initialize session manager
        let db_path = config.resolved_db_path();
        let sessions_dir = config.data_dir.join("sessions");
        let session_mgr = SessionManager::new(&db_path, &sessions_dir)?;

        // Initialize vault — derive master key from API key or use a default
        // In production, this should come from a secure key derivation
        let vault_db = config.data_dir.join("vault.db");
        let master_key = derive_vault_key(&config);
        let vault = Vault::new(&vault_db, &master_key)?;

        Ok(Self {
            memory: Arc::new(memory),
            session_mgr: Arc::new(session_mgr),
            vault: Arc::new(vault),
            config,
        })
    }

    /// Process a chat message through the full Orion pipeline.
    ///
    /// 1. Resolve or create session
    /// 2. Build system prompt from bootstrap context
    /// 3. Configure agent-sdk with custom tools
    /// 4. Run agent loop
    /// 5. Record usage and append daily log
    pub async fn chat(&self, message: ChatMessage) -> Result<ChatResponse> {
        // Step 1: Resolve session
        let session_id = match self.session_mgr.resolve_session()? {
            SessionDecision::Continue(id) => {
                debug!(session_id = %id, "Continuing existing session");
                id
            }
            SessionDecision::New => {
                let id = self.session_mgr.create_session()?;
                debug!(session_id = %id, "Created new session");
                id
            }
        };
        self.session_mgr.touch_session(&session_id)?;

        // Step 2: Build system prompt
        let bootstrap = self.memory.bootstrap_context()?;
        let date_str = Local::now().format("%A, %B %d, %Y at %H:%M").to_string();
        let system_prompt = format!(
            "{}\n\n---\nCurrent date/time: {}\nSession ID: {}\n\
             You have access to memory tools (MemorySearch, MemoryWrite, MemoryAppendDaily) \
             and vault tools (VaultGet, VaultSet). Use MemorySearch to recall past conversations \
             and knowledge. Use MemoryWrite to persist important information. Use MemoryAppendDaily \
             to log notable events.",
            bootstrap, date_str, session_id,
        );

        // Step 3: Build allowed tools list
        let allowed_tools: Vec<String> = vec![
            "Read".into(),
            "Bash".into(),
            "Glob".into(),
            "Grep".into(),
            "MemorySearch".into(),
            "MemoryWrite".into(),
            "MemoryAppendDaily".into(),
            "VaultGet".into(),
            "VaultSet".into(),
        ];

        // Step 4: Build external tool handler
        let memory_clone = Arc::clone(&self.memory);
        let vault_clone = Arc::clone(&self.vault);

        let handler: ExternalToolHandlerFn = Box::new(move |tool_name, input| {
            let mem = Arc::clone(&memory_clone);
            let vlt = Arc::clone(&vault_clone);
            Box::pin(async move {
                handle_custom_tool(&mem, &vlt, &tool_name, &input).await
            })
        });

        // Step 5: Build options and run query
        let options = Options::builder()
            .allowed_tools(allowed_tools)
            .system_prompt(SystemPrompt::Custom(system_prompt))
            .permission_mode(PermissionMode::BypassPermissions)
            .model(&self.config.model)
            .max_turns(self.config.max_turns)
            .session_id(session_id.clone())
            .external_tool_handler(handler)
            .custom_tools(custom_tool_definitions())
            .build();

        let mut stream = agent_sdk::query(&message.text, options);

        // Step 6: Collect result
        let mut result_text = String::new();
        let mut usage = ChatUsage::default();

        while let Some(msg_result) = stream.next().await {
            match msg_result {
                Ok(Message::Assistant(assistant)) => {
                    // Extract text from content blocks
                    for block in &assistant.content {
                        if let agent_sdk::ContentBlock::Text { text } = block {
                            if !result_text.is_empty() {
                                result_text.push('\n');
                            }
                            result_text.push_str(text);
                        }
                    }
                }
                Ok(Message::Result(result)) => {
                    // Capture final result text if we don't have one yet
                    if result_text.is_empty() {
                        if let Some(text) = &result.result {
                            result_text = text.clone();
                        }
                    }

                    // Record usage
                    if let Some(u) = &result.usage {
                        usage = ChatUsage {
                            input_tokens: u.input_tokens,
                            output_tokens: u.output_tokens,
                            cache_read_tokens: u.cache_read_input_tokens,
                            cache_write_tokens: u.cache_creation_input_tokens,
                            cost_usd: result.total_cost_usd,
                        };

                        // Persist usage to session
                        let _ = self.session_mgr.record_usage(
                            &session_id,
                            &UsageRecord {
                                input_tokens: u.input_tokens,
                                output_tokens: u.output_tokens,
                                cache_read: u.cache_read_input_tokens,
                                cache_write: u.cache_creation_input_tokens,
                                cost_usd: result.total_cost_usd,
                                model: self.config.model.clone(),
                            },
                            result.num_turns,
                        );
                    }

                    if result.is_error {
                        if let Some(err) = result.errors.first() {
                            error!(error = %err, "Agent error");
                        }
                    }
                }
                Ok(_) => {} // System, User, StreamEvent — skip
                Err(e) => {
                    error!(error = %e, "Stream error");
                    return Err(OrionError::Agent(e.to_string()));
                }
            }
        }

        // Step 7: Append summary to daily log
        let summary = if result_text.len() > 200 {
            format!("{}...", &result_text[..200])
        } else {
            result_text.clone()
        };
        let _ = self.memory.append_daily(&format!(
            "**User**: {}\n**Orion**: {}",
            truncate(&message.text, 200),
            summary,
        ));

        Ok(ChatResponse {
            text: result_text,
            session_id,
            usage: Some(usage),
        })
    }

    /// Get a reference to the memory store.
    pub fn memory(&self) -> &Arc<MemoryStore> {
        &self.memory
    }

    /// Get a reference to the session manager.
    pub fn session_mgr(&self) -> &Arc<SessionManager> {
        &self.session_mgr
    }

    /// Get a reference to the vault.
    pub fn vault(&self) -> &Arc<Vault> {
        &self.vault
    }
}

/// Derive a 32-byte vault key. Uses the API key if available, otherwise a fixed default.
/// In production, use a proper KDF (Argon2, HKDF).
fn derive_vault_key(config: &OrionConfig) -> [u8; 32] {
    let env_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let seed = config
        .api_key
        .as_deref()
        .or(env_key.as_deref())
        .unwrap_or("orion-default-vault-key-change-me!");

    let mut key = [0u8; 32];
    let bytes = seed.as_bytes();
    for (i, byte) in bytes.iter().enumerate().take(32) {
        key[i] = *byte;
    }
    // If seed is shorter than 32 bytes, pad with hash-like mixing
    if bytes.len() < 32 {
        for i in bytes.len()..32 {
            key[i] = key[i % bytes.len()].wrapping_add(i as u8);
        }
    }
    key
}

/// Truncate a string to a maximum length, adding "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> OrionConfig {
        OrionConfig {
            data_dir: tmp.path().to_path_buf(),
            db_path: Some(tmp.path().join("memory.db")),
            ..OrionConfig::default()
        }
    }

    #[test]
    fn test_agent_construction() {
        let tmp = TempDir::new().unwrap();
        let agent = OrionAgent::new(test_config(&tmp)).unwrap();

        // Memory should be initialized
        let ctx = agent.memory().bootstrap_context().unwrap();
        assert!(ctx.contains("Orion"));

        // Vault should work
        agent.vault().set("test", "value").unwrap();
        assert_eq!(agent.vault().get("test").unwrap().as_deref(), Some("value"));
    }

    #[test]
    fn test_custom_tool_definitions() {
        let defs = custom_tool_definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"MemorySearch"));
        assert!(names.contains(&"MemoryWrite"));
        assert!(names.contains(&"MemoryAppendDaily"));
        assert!(names.contains(&"VaultGet"));
        assert!(names.contains(&"VaultSet"));
        assert_eq!(defs.len(), 5);
    }

    #[tokio::test]
    async fn test_custom_tool_handler() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let agent = OrionAgent::new(config).unwrap();

        // Test MemorySearch
        let result = handle_custom_tool(
            agent.memory(),
            agent.vault(),
            "MemorySearch",
            &serde_json::json!({"query": "Orion", "limit": 3}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);

        // Test MemoryWrite
        let result = handle_custom_tool(
            agent.memory(),
            agent.vault(),
            "MemoryWrite",
            &serde_json::json!({"file": "knowledge/test.md", "content": "Test content"}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Test MemoryAppendDaily
        let result = handle_custom_tool(
            agent.memory(),
            agent.vault(),
            "MemoryAppendDaily",
            &serde_json::json!({"text": "Something happened"}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Test VaultSet + VaultGet
        let result = handle_custom_tool(
            agent.memory(),
            agent.vault(),
            "VaultSet",
            &serde_json::json!({"key": "api_key", "value": "sk-123"}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        let result = handle_custom_tool(
            agent.memory(),
            agent.vault(),
            "VaultGet",
            &serde_json::json!({"key": "api_key"}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert_eq!(r.content, "sk-123");

        // Test unknown tool
        let result = handle_custom_tool(
            agent.memory(),
            agent.vault(),
            "UnknownTool",
            &serde_json::json!({}),
        )
        .await;
        assert!(result.is_none());
    }
}
