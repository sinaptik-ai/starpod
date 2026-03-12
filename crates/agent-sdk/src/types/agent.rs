use serde::{Deserialize, Serialize};

/// Configuration for a subagent defined programmatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// Natural language description of when to use this agent.
    pub description: String,

    /// The agent's system prompt defining its role and behavior.
    pub prompt: String,

    /// Array of allowed tool names. If None, inherits all tools from parent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,

    /// Array of tool names to explicitly disallow for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disallowed_tools: Option<Vec<String>>,

    /// Model override for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<AgentModel>,

    /// MCP server specifications for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<AgentMcpServerSpec>>,

    /// Array of skill names to preload into the agent context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,

    /// Maximum number of agentic turns before stopping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

impl AgentDefinition {
    pub fn new(description: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            prompt: prompt.into(),
            tools: None,
            disallowed_tools: None,
            model: None,
            mcp_servers: None,
            skills: None,
            max_turns: None,
        }
    }

    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn with_model(mut self, model: AgentModel) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = Some(max_turns);
        self
    }
}

/// Model selection for subagents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentModel {
    Sonnet,
    Opus,
    Haiku,
    Inherit,
}

/// MCP server specification for subagents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentMcpServerSpec {
    /// Reference to a parent MCP server by name.
    Name(String),
    /// Inline MCP server configuration.
    Config(std::collections::HashMap<String, serde_json::Value>),
}

/// Input for the Agent tool (spawning subagents).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInput {
    pub description: String,
    pub prompt: String,
    pub subagent_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<AgentModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_in_background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolation: Option<AgentIsolation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentIsolation {
    Worktree,
}
