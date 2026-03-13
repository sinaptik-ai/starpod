pub mod tools;

use std::sync::Arc;

use chrono::Local;
use tokio_stream::StreamExt;
use tracing::{debug, error};

use agent_sdk::{ExternalToolHandlerFn, Message, Options, PermissionMode, Query};
use agent_sdk::options::SystemPrompt;

use orion_core::{ChatMessage, ChatResponse, ChatUsage, OrionConfig, OrionError, Result};
use orion_cron::CronStore;
use orion_memory::MemoryStore;
use orion_session::{SessionDecision, SessionManager, UsageRecord};
use orion_skills::SkillStore;
use orion_vault::Vault;

use crate::tools::{custom_tool_definitions, handle_custom_tool, ToolContext};

/// All custom tool names.
const CUSTOM_TOOLS: &[&str] = &[
    "MemorySearch", "MemoryWrite", "MemoryAppendDaily",
    "VaultGet", "VaultSet",
    "SkillCreate", "SkillUpdate", "SkillDelete", "SkillList",
    "CronAdd", "CronList", "CronRemove", "CronRuns",
];

/// The Orion agent orchestrator.
///
/// Wires together memory, sessions, vault, skills, cron, and the agent-sdk
/// to provide a high-level `chat()` interface.
pub struct OrionAgent {
    memory: Arc<MemoryStore>,
    session_mgr: Arc<SessionManager>,
    vault: Arc<Vault>,
    skills: Arc<SkillStore>,
    cron: Arc<CronStore>,
    config: OrionConfig,
}

impl OrionAgent {
    /// Create a new OrionAgent from config.
    pub fn new(config: OrionConfig) -> Result<Self> {
        let memory = MemoryStore::new(&config.data_dir)?;

        let db_path = config.resolved_db_path();
        let sessions_dir = config.data_dir.join("sessions");
        let session_mgr = SessionManager::new(&db_path, &sessions_dir)?;

        let vault_db = config.data_dir.join("vault.db");
        let master_key = derive_vault_key(&config);
        let vault = Vault::new(&vault_db, &master_key)?;

        let skills = SkillStore::new(&config.data_dir)?;

        let cron_db = config.data_dir.join("cron.db");
        let cron = CronStore::new(&cron_db)?;

        Ok(Self {
            memory: Arc::new(memory),
            session_mgr: Arc::new(session_mgr),
            vault: Arc::new(vault),
            skills: Arc::new(skills),
            cron: Arc::new(cron),
            config,
        })
    }

    /// Build the system prompt from bootstrap context + skills.
    fn build_system_prompt(&self, session_id: &str) -> Result<String> {
        let bootstrap = self.memory.bootstrap_context()?;
        let skills_ctx = self.skills.bootstrap_skills()?;
        let date_str = Local::now().format("%A, %B %d, %Y at %H:%M").to_string();

        let mut prompt = format!(
            "{}\n\n---\nCurrent date/time: {}\nSession ID: {}\n\
             You have access to memory tools (MemorySearch, MemoryWrite, MemoryAppendDaily), \
             vault tools (VaultGet, VaultSet), skill tools (SkillCreate, SkillUpdate, SkillDelete, SkillList), \
             and scheduling tools (CronAdd, CronList, CronRemove, CronRuns).",
            bootstrap, date_str, session_id,
        );

        if !skills_ctx.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&skills_ctx);
        }

        Ok(prompt)
    }

    /// Build the allowed tools list (built-in + custom).
    fn allowed_tools() -> Vec<String> {
        let mut tools: Vec<String> = vec![
            "Read".into(), "Bash".into(), "Glob".into(), "Grep".into(),
        ];
        tools.extend(CUSTOM_TOOLS.iter().map(|s| s.to_string()));
        tools
    }

    /// Build the external tool handler closure.
    fn build_tool_handler(&self) -> ExternalToolHandlerFn {
        let ctx = Arc::new(ToolContext {
            memory: Arc::clone(&self.memory),
            vault: Arc::clone(&self.vault),
            skills: Arc::clone(&self.skills),
            cron: Arc::clone(&self.cron),
        });

        Box::new(move |tool_name, input| {
            let ctx = Arc::clone(&ctx);
            Box::pin(async move {
                handle_custom_tool(&ctx, &tool_name, &input).await
            })
        })
    }

    /// Process a chat message through the full Orion pipeline.
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
        let system_prompt = self.build_system_prompt(&session_id)?;

        // Step 3: Build options and run query
        let options = Options::builder()
            .allowed_tools(Self::allowed_tools())
            .system_prompt(SystemPrompt::Custom(system_prompt))
            .permission_mode(PermissionMode::BypassPermissions)
            .model(&self.config.model)
            .max_turns(self.config.max_turns)
            .session_id(session_id.clone())
            .external_tool_handler(self.build_tool_handler())
            .custom_tools(custom_tool_definitions())
            .build();

        let mut stream = agent_sdk::query(&message.text, options);

        // Step 4: Collect result
        let mut result_text = String::new();
        let mut usage = ChatUsage::default();

        while let Some(msg_result) = stream.next().await {
            match msg_result {
                Ok(Message::Assistant(assistant)) => {
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
                    if result_text.is_empty() {
                        if let Some(text) = &result.result {
                            result_text = text.clone();
                        }
                    }

                    if let Some(u) = &result.usage {
                        usage = ChatUsage {
                            input_tokens: u.input_tokens,
                            output_tokens: u.output_tokens,
                            cache_read_tokens: u.cache_read_input_tokens,
                            cache_write_tokens: u.cache_creation_input_tokens,
                            cost_usd: result.total_cost_usd,
                        };

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
                Ok(_) => {}
                Err(e) => {
                    error!(error = %e, "Stream error");
                    return Err(OrionError::Agent(e.to_string()));
                }
            }
        }

        // Step 5: Append summary to daily log
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

    /// Start a streaming chat that yields raw agent-sdk messages.
    ///
    /// Returns (Query stream, session_id). The caller should consume the stream
    /// for real-time display, then call `finalize_chat()` with the collected results.
    pub fn chat_stream(&self, message: &str) -> Result<(Query, String)> {
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

        let system_prompt = self.build_system_prompt(&session_id)?;

        let options = Options::builder()
            .allowed_tools(Self::allowed_tools())
            .system_prompt(SystemPrompt::Custom(system_prompt))
            .permission_mode(PermissionMode::BypassPermissions)
            .model(&self.config.model)
            .max_turns(self.config.max_turns)
            .session_id(session_id.clone())
            .external_tool_handler(self.build_tool_handler())
            .custom_tools(custom_tool_definitions())
            .build();

        let stream = agent_sdk::query(message, options);
        Ok((stream, session_id))
    }

    /// Finalize a streaming chat — record usage and append daily log.
    pub fn finalize_chat(
        &self,
        session_id: &str,
        user_text: &str,
        result_text: &str,
        result: &agent_sdk::ResultMessage,
    ) {
        if let Some(u) = &result.usage {
            let _ = self.session_mgr.record_usage(
                session_id,
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

        let summary = if result_text.len() > 200 {
            format!("{}...", &result_text[..200])
        } else {
            result_text.to_string()
        };
        let _ = self.memory.append_daily(&format!(
            "**User**: {}\n**Orion**: {}",
            truncate(user_text, 200),
            summary,
        ));
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

    /// Get a reference to the skill store.
    pub fn skills(&self) -> &Arc<SkillStore> {
        &self.skills
    }

    /// Get a reference to the cron store.
    pub fn cron(&self) -> &Arc<CronStore> {
        &self.cron
    }

    /// Start the cron scheduler as a background task.
    ///
    /// The executor callback sends the job prompt through `chat()`.
    /// Returns a JoinHandle for the background task.
    pub fn start_scheduler(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let cron_store = Arc::clone(&self.cron);
        let agent = Arc::clone(self);

        let executor: orion_cron::JobExecutor = Arc::new(move |prompt| {
            let agent = Arc::clone(&agent);
            Box::pin(async move {
                let msg = ChatMessage {
                    text: prompt,
                    user_id: Some("cron".into()),
                    channel_id: Some("scheduler".into()),
                    attachments: Vec::new(),
                };
                match agent.chat(msg).await {
                    Ok(resp) => Ok(truncate(&resp.text, 500)),
                    Err(e) => Err(e.to_string()),
                }
            })
        });

        let scheduler = orion_cron::CronScheduler::new(cron_store, executor, 30);
        scheduler.start()
    }
}

/// Derive a 32-byte vault key.
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

        // Skills dir should exist
        assert!(tmp.path().join("skills").exists());

        // Cron db should exist
        assert!(tmp.path().join("cron.db").exists());
    }

    #[test]
    fn test_custom_tool_definitions() {
        let defs = custom_tool_definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();

        // Memory tools
        assert!(names.contains(&"MemorySearch"));
        assert!(names.contains(&"MemoryWrite"));
        assert!(names.contains(&"MemoryAppendDaily"));
        // Vault tools
        assert!(names.contains(&"VaultGet"));
        assert!(names.contains(&"VaultSet"));
        // Skill tools
        assert!(names.contains(&"SkillCreate"));
        assert!(names.contains(&"SkillUpdate"));
        assert!(names.contains(&"SkillDelete"));
        assert!(names.contains(&"SkillList"));
        // Cron tools
        assert!(names.contains(&"CronAdd"));
        assert!(names.contains(&"CronList"));
        assert!(names.contains(&"CronRemove"));
        assert!(names.contains(&"CronRuns"));

        assert_eq!(defs.len(), 13);
    }

    #[tokio::test]
    async fn test_custom_tool_handler() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let agent = OrionAgent::new(config).unwrap();

        let ctx = ToolContext {
            memory: Arc::clone(agent.memory()),
            vault: Arc::clone(agent.vault()),
            skills: Arc::clone(agent.skills()),
            cron: Arc::clone(agent.cron()),
        };

        // Test MemorySearch
        let result = handle_custom_tool(
            &ctx,
            "MemorySearch",
            &serde_json::json!({"query": "Orion", "limit": 3}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        // Test SkillCreate + SkillList
        let result = handle_custom_tool(
            &ctx,
            "SkillCreate",
            &serde_json::json!({"name": "test-skill", "content": "Do testing."}),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        let result = handle_custom_tool(
            &ctx,
            "SkillList",
            &serde_json::json!({}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("test-skill"));

        // Test CronAdd + CronList
        let result = handle_custom_tool(
            &ctx,
            "CronAdd",
            &serde_json::json!({
                "name": "test-job",
                "prompt": "Check status",
                "schedule": {"kind": "interval", "every_ms": 60000}
            }),
        )
        .await;
        assert!(result.is_some());
        assert!(!result.unwrap().is_error);

        let result = handle_custom_tool(
            &ctx,
            "CronList",
            &serde_json::json!({}),
        )
        .await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("test-job"));

        // Test unknown tool
        let result = handle_custom_tool(
            &ctx,
            "UnknownTool",
            &serde_json::json!({}),
        )
        .await;
        assert!(result.is_none());
    }
}
