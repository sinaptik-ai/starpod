use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::types::permissions::PermissionUpdate;

/// Hook return value - either async (fire-and-forget) or sync (blocking).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookOutput {
    Async(AsyncHookOutput),
    Sync(SyncHookOutput),
}

impl Default for HookOutput {
    fn default() -> Self {
        HookOutput::Sync(SyncHookOutput::default())
    }
}

/// Async hook output - the agent proceeds without waiting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncHookOutput {
    /// Must be true to signal async mode.
    #[serde(rename = "async")]
    pub is_async: bool,
    /// Optional timeout in milliseconds for the background operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub async_timeout: Option<u64>,
}

/// Sync hook output - controls the agent's behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncHookOutput {
    /// Whether the agent should continue running after this hook.
    #[serde(rename = "continue", skip_serializing_if = "Option::is_none")]
    pub should_continue: Option<bool>,

    /// Suppress output from being shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_output: Option<bool>,

    /// Reason for stopping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,

    /// Approve or block decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<HookDecision>,

    /// Inject a system message into the conversation visible to the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,

    /// Reason for the decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Hook-specific output that controls the current operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookSpecificOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HookDecision {
    Approve,
    Block,
}

/// Hook-specific output varies by hook event type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "hookEventName")]
pub enum HookSpecificOutput {
    PreToolUse {
        #[serde(skip_serializing_if = "Option::is_none")]
        permission_decision: Option<PermissionDecision>,
        #[serde(skip_serializing_if = "Option::is_none")]
        permission_decision_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        updated_input: Option<HashMap<String, serde_json::Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_context: Option<String>,
    },

    PostToolUse {
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_context: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        updated_mcp_tool_output: Option<serde_json::Value>,
    },

    PostToolUseFailure {
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_context: Option<String>,
    },

    UserPromptSubmit {
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_context: Option<String>,
    },

    SessionStart {
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_context: Option<String>,
    },

    Setup {
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_context: Option<String>,
    },

    SubagentStart {
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_context: Option<String>,
    },

    Notification {
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_context: Option<String>,
    },

    PermissionRequest {
        decision: PermissionRequestDecision,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

/// Decision for PermissionRequest hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "behavior")]
pub enum PermissionRequestDecision {
    #[serde(rename = "allow")]
    Allow {
        #[serde(skip_serializing_if = "Option::is_none")]
        updated_input: Option<HashMap<String, serde_json::Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        updated_permissions: Option<Vec<PermissionUpdate>>,
    },
    #[serde(rename = "deny")]
    Deny {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        interrupt: Option<bool>,
    },
}
