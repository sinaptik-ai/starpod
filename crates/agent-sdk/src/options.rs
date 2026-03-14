use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use serde::{Deserialize, Serialize};

use crate::hooks::{HookCallbackMatcher, HookEvent};
use crate::mcp::McpServerConfig;
use crate::tools::executor::ToolResult;
use crate::types::agent::AgentDefinition;
use crate::types::permissions::{CanUseToolOptions, PermissionResult};

/// Permission mode controls how Claude uses tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Standard permission behavior - unmatched tools trigger `can_use_tool`.
    Default,
    /// Auto-accept file edits.
    AcceptEdits,
    /// Bypass all permission checks (use with caution).
    BypassPermissions,
    /// Planning mode - no tool execution.
    Plan,
    /// Don't prompt for permissions, deny if not pre-approved.
    DontAsk,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionMode::Default => write!(f, "default"),
            PermissionMode::AcceptEdits => write!(f, "acceptEdits"),
            PermissionMode::BypassPermissions => write!(f, "bypassPermissions"),
            PermissionMode::Plan => write!(f, "plan"),
            PermissionMode::DontAsk => write!(f, "dontAsk"),
        }
    }
}

/// Effort level controlling how much reasoning Claude applies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Max,
}

/// Setting sources to load from filesystem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SettingSource {
    /// Global user settings (~/.claude/settings.json).
    User,
    /// Shared project settings (.claude/settings.json).
    Project,
    /// Local project settings (.claude/settings.local.json).
    Local,
}

/// Thinking configuration for Claude's reasoning behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ThinkingConfig {
    /// Adaptive thinking - Claude decides when to think.
    #[serde(rename = "adaptive")]
    Adaptive,
    /// Disabled thinking.
    #[serde(rename = "disabled")]
    Disabled,
    /// Enabled with a specific budget.
    #[serde(rename = "enabled")]
    Enabled {
        budget_tokens: u64,
    },
}

/// System prompt configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemPrompt {
    /// Custom system prompt string.
    Custom(String),
    /// Use Claude Code's built-in system prompt.
    Preset {
        #[serde(rename = "type")]
        prompt_type: String,
        preset: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        append: Option<String>,
    },
}

/// Sandbox settings for tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_network: Option<bool>,
}

/// Configuration for the AskUserQuestion tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask_user_question: Option<AskUserQuestionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserQuestionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_format: Option<PreviewFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PreviewFormat {
    Markdown,
    Html,
}

/// Plugin configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(rename = "type")]
    pub plugin_type: String,
    pub path: String,
}

/// Configuration options for a query.
pub struct Options {
    /// Tools to auto-approve without prompting.
    pub allowed_tools: Vec<String>,

    /// Tools to always deny.
    pub disallowed_tools: Vec<String>,

    /// Permission mode for the session.
    pub permission_mode: PermissionMode,

    /// Custom permission function for tool usage.
    pub can_use_tool: Option<CanUseToolFn>,

    /// Current working directory.
    pub cwd: Option<String>,

    /// Claude model to use.
    pub model: Option<String>,

    /// Fallback model if primary fails.
    pub fallback_model: Option<String>,

    /// Controls reasoning depth.
    pub effort: Option<Effort>,

    /// Maximum agentic turns (tool-use round trips).
    pub max_turns: Option<u32>,

    /// Maximum budget in USD.
    pub max_budget_usd: Option<f64>,

    /// System prompt configuration.
    pub system_prompt: Option<SystemPrompt>,

    /// Thinking configuration.
    pub thinking: Option<ThinkingConfig>,

    /// Hook callbacks for events.
    pub hooks: HashMap<HookEvent, Vec<HookCallbackMatcher>>,

    /// MCP server configurations.
    pub mcp_servers: HashMap<String, McpServerConfig>,

    /// Programmatically defined subagents.
    pub agents: HashMap<String, AgentDefinition>,

    /// Continue the most recent conversation.
    pub continue_session: bool,

    /// Session ID to resume.
    pub resume: Option<String>,

    /// Fork session when resuming.
    pub fork_session: bool,

    /// Use a specific UUID for the session.
    pub session_id: Option<String>,

    /// Control which filesystem settings to load.
    pub setting_sources: Vec<SettingSource>,

    /// Enable debug mode.
    pub debug: bool,

    /// Write debug logs to a specific file.
    pub debug_file: Option<String>,

    /// Include partial message events.
    pub include_partial_messages: bool,

    /// When false, disables session persistence to disk.
    pub persist_session: bool,

    /// Enable file change tracking for rewinding.
    pub enable_file_checkpointing: bool,

    /// Environment variables.
    pub env: HashMap<String, String>,

    /// Additional directories Claude can access.
    pub additional_directories: Vec<String>,

    /// Structured output schema.
    pub output_format: Option<serde_json::Value>,

    /// Sandbox settings.
    pub sandbox: Option<SandboxSettings>,

    /// Tool configuration.
    pub tool_config: Option<ToolConfig>,

    /// Plugin configurations.
    pub plugins: Vec<PluginConfig>,

    /// Enable prompt suggestions.
    pub prompt_suggestions: bool,

    /// External tool handler for custom tools.
    ///
    /// Called before the built-in executor. If it returns `Some(ToolResult)`,
    /// the built-in executor is skipped for that tool call.
    pub external_tool_handler: Option<ExternalToolHandlerFn>,

    /// Custom tool definitions (JSON schemas) sent to the API alongside built-in tools.
    ///
    /// These are typically used with `external_tool_handler` to register and handle
    /// tools that aren't part of the built-in set (e.g. MemorySearch, VaultGet).
    pub custom_tool_definitions: Vec<CustomToolDefinition>,

    /// Explicit API key. When set, bypasses the `ANTHROPIC_API_KEY` env var lookup.
    pub api_key: Option<String>,
}

/// A custom tool definition to send to the Claude API.
#[derive(Debug, Clone)]
pub struct CustomToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Type alias for external tool handler callback.
///
/// When set, this handler is called before the built-in tool executor. If it returns
/// `Some(ToolResult)`, the built-in executor is skipped for that tool call.
/// This allows embedding custom tools (e.g. MemorySearch, VaultGet) alongside
/// the built-in tools (Read, Write, Bash, etc.).
pub type ExternalToolHandlerFn = Box<
    dyn Fn(
            String,
            serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Option<ToolResult>> + Send>>
        + Send
        + Sync,
>;

/// Type alias for the can_use_tool callback.
pub type CanUseToolFn = Box<
    dyn Fn(
            String,
            serde_json::Value,
            CanUseToolOptions,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = crate::error::Result<PermissionResult>> + Send>,
        > + Send
        + Sync,
>;

impl Default for Options {
    fn default() -> Self {
        Self {
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            permission_mode: PermissionMode::Default,
            can_use_tool: None,
            cwd: None,
            model: None,
            fallback_model: None,
            effort: None,
            max_turns: None,
            max_budget_usd: None,
            system_prompt: None,
            thinking: None,
            hooks: HashMap::new(),
            mcp_servers: HashMap::new(),
            agents: HashMap::new(),
            continue_session: false,
            resume: None,
            fork_session: false,
            session_id: None,
            setting_sources: Vec::new(),
            debug: false,
            debug_file: None,
            include_partial_messages: false,
            persist_session: true,
            enable_file_checkpointing: false,
            env: HashMap::new(),
            additional_directories: Vec::new(),
            output_format: None,
            sandbox: None,
            tool_config: None,
            plugins: Vec::new(),
            prompt_suggestions: false,
            external_tool_handler: None,
            custom_tool_definitions: Vec::new(),
            api_key: None,
        }
    }
}

impl Options {
    /// Create a new Options builder.
    pub fn builder() -> OptionsBuilder {
        OptionsBuilder::default()
    }
}

/// Builder for constructing Options.
#[derive(Default)]
pub struct OptionsBuilder {
    options: Options,
}

impl OptionsBuilder {
    pub fn allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.options.allowed_tools = tools;
        self
    }

    pub fn disallowed_tools(mut self, tools: Vec<String>) -> Self {
        self.options.disallowed_tools = tools;
        self
    }

    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.options.permission_mode = mode;
        self
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.options.cwd = Some(cwd.into());
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.options.model = Some(model.into());
        self
    }

    pub fn fallback_model(mut self, model: impl Into<String>) -> Self {
        self.options.fallback_model = Some(model.into());
        self
    }

    pub fn effort(mut self, effort: Effort) -> Self {
        self.options.effort = Some(effort);
        self
    }

    pub fn max_turns(mut self, max_turns: u32) -> Self {
        self.options.max_turns = Some(max_turns);
        self
    }

    pub fn max_budget_usd(mut self, budget: f64) -> Self {
        self.options.max_budget_usd = Some(budget);
        self
    }

    pub fn system_prompt(mut self, prompt: SystemPrompt) -> Self {
        self.options.system_prompt = Some(prompt);
        self
    }

    pub fn thinking(mut self, config: ThinkingConfig) -> Self {
        self.options.thinking = Some(config);
        self
    }

    pub fn hook(mut self, event: HookEvent, matchers: Vec<HookCallbackMatcher>) -> Self {
        self.options.hooks.insert(event, matchers);
        self
    }

    pub fn mcp_server(mut self, name: impl Into<String>, config: McpServerConfig) -> Self {
        self.options.mcp_servers.insert(name.into(), config);
        self
    }

    pub fn agent(mut self, name: impl Into<String>, definition: AgentDefinition) -> Self {
        self.options.agents.insert(name.into(), definition);
        self
    }

    pub fn continue_session(mut self, value: bool) -> Self {
        self.options.continue_session = value;
        self
    }

    pub fn resume(mut self, session_id: impl Into<String>) -> Self {
        self.options.resume = Some(session_id.into());
        self
    }

    pub fn session_id(mut self, id: impl Into<String>) -> Self {
        self.options.session_id = Some(id.into());
        self
    }

    pub fn fork_session(mut self, value: bool) -> Self {
        self.options.fork_session = value;
        self
    }

    pub fn setting_sources(mut self, sources: Vec<SettingSource>) -> Self {
        self.options.setting_sources = sources;
        self
    }

    pub fn debug(mut self, value: bool) -> Self {
        self.options.debug = value;
        self
    }

    pub fn include_partial_messages(mut self, value: bool) -> Self {
        self.options.include_partial_messages = value;
        self
    }

    pub fn persist_session(mut self, value: bool) -> Self {
        self.options.persist_session = value;
        self
    }

    pub fn enable_file_checkpointing(mut self, value: bool) -> Self {
        self.options.enable_file_checkpointing = value;
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.env.insert(key.into(), value.into());
        self
    }

    pub fn output_format(mut self, schema: serde_json::Value) -> Self {
        self.options.output_format = Some(schema);
        self
    }

    pub fn sandbox(mut self, settings: SandboxSettings) -> Self {
        self.options.sandbox = Some(settings);
        self
    }

    pub fn external_tool_handler(mut self, handler: ExternalToolHandlerFn) -> Self {
        self.options.external_tool_handler = Some(handler);
        self
    }

    pub fn custom_tool(mut self, def: CustomToolDefinition) -> Self {
        self.options.custom_tool_definitions.push(def);
        self
    }

    pub fn custom_tools(mut self, defs: Vec<CustomToolDefinition>) -> Self {
        self.options.custom_tool_definitions.extend(defs);
        self
    }

    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.options.api_key = Some(key.into());
        self
    }

    pub fn build(self) -> Options {
        self.options
    }
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("allowed_tools", &self.allowed_tools)
            .field("disallowed_tools", &self.disallowed_tools)
            .field("permission_mode", &self.permission_mode)
            .field("cwd", &self.cwd)
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("max_turns", &self.max_turns)
            .field("max_budget_usd", &self.max_budget_usd)
            .field("hooks_count", &self.hooks.len())
            .field("mcp_servers_count", &self.mcp_servers.len())
            .field("agents_count", &self.agents.len())
            .field("continue_session", &self.continue_session)
            .field("resume", &self.resume)
            .field("persist_session", &self.persist_session)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_api_key_sets_field() {
        let opts = Options::builder()
            .api_key("sk-ant-test-key")
            .build();
        assert_eq!(opts.api_key.as_deref(), Some("sk-ant-test-key"));
    }

    #[test]
    fn builder_api_key_default_is_none() {
        let opts = Options::builder().build();
        assert!(opts.api_key.is_none());
    }

    #[test]
    fn builder_api_key_with_other_options() {
        let opts = Options::builder()
            .model("claude-haiku-4-5")
            .api_key("sk-ant-combined")
            .max_turns(10)
            .build();
        assert_eq!(opts.api_key.as_deref(), Some("sk-ant-combined"));
        assert_eq!(opts.model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(opts.max_turns, Some(10));
    }
}
