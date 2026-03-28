//! Hook output types — the data returned by hook callbacks to control agent behavior.

use crate::permissions::{PermissionDecision, PermissionUpdate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Hook return value — either async (fire-and-forget) or sync (blocking).
///
/// # Example
///
/// ```
/// use starpod_hooks::HookOutput;
///
/// // Default is a no-op sync output
/// let output = HookOutput::default();
/// assert!(matches!(output, HookOutput::Sync(_)));
/// ```
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

/// Async hook output — the agent proceeds without waiting for the hook to finish.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncHookOutput {
    /// Must be true to signal async mode.
    #[serde(rename = "async")]
    pub is_async: bool,
    /// Optional timeout in milliseconds for the background operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub async_timeout: Option<u64>,
}

/// Sync hook output — controls the agent's behavior.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_is_sync() {
        let output = HookOutput::default();
        assert!(matches!(output, HookOutput::Sync(_)));
    }

    #[test]
    fn sync_output_default_has_no_fields_set() {
        let sync = SyncHookOutput::default();
        assert!(sync.should_continue.is_none());
        assert!(sync.suppress_output.is_none());
        assert!(sync.stop_reason.is_none());
        assert!(sync.decision.is_none());
        assert!(sync.system_message.is_none());
        assert!(sync.reason.is_none());
        assert!(sync.hook_specific_output.is_none());
    }

    #[test]
    fn hook_decision_serde() {
        let approve = HookDecision::Approve;
        let json = serde_json::to_string(&approve).unwrap();
        assert_eq!(json, "\"approve\"");

        let block = HookDecision::Block;
        let json = serde_json::to_string(&block).unwrap();
        assert_eq!(json, "\"block\"");
    }

    #[test]
    fn async_output_roundtrip() {
        let output = HookOutput::Async(AsyncHookOutput {
            is_async: true,
            async_timeout: Some(5000),
        });
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"async\":true"));
        assert!(json.contains("5000"));
    }

    #[test]
    fn sync_output_with_decision_roundtrip() {
        let output = HookOutput::Sync(SyncHookOutput {
            should_continue: Some(false),
            decision: Some(HookDecision::Block),
            reason: Some("blocked by policy".into()),
            ..Default::default()
        });
        let json = serde_json::to_string(&output).unwrap();
        let back: HookOutput = serde_json::from_str(&json).unwrap();
        match back {
            HookOutput::Sync(sync) => {
                assert_eq!(sync.should_continue, Some(false));
                assert_eq!(sync.decision, Some(HookDecision::Block));
                assert_eq!(sync.reason.as_deref(), Some("blocked by policy"));
            }
            _ => panic!("expected Sync output"),
        }
    }

    #[test]
    fn pre_tool_use_specific_output() {
        let specific = HookSpecificOutput::PreToolUse {
            permission_decision: Some(PermissionDecision::Deny),
            permission_decision_reason: Some("not allowed".into()),
            updated_input: None,
            additional_context: Some("context".into()),
        };
        let json = serde_json::to_string(&specific).unwrap();
        assert!(json.contains("\"hookEventName\":\"PreToolUse\""));
        assert!(json.contains("\"permission_decision\":\"deny\""));
    }

    #[test]
    fn permission_request_decision_allow() {
        let decision = PermissionRequestDecision::Allow {
            updated_input: None,
            updated_permissions: None,
        };
        let json = serde_json::to_string(&decision).unwrap();
        assert!(json.contains("\"behavior\":\"allow\""));
    }

    #[test]
    fn permission_request_decision_deny_with_message() {
        let decision = PermissionRequestDecision::Deny {
            message: Some("no access".into()),
            interrupt: Some(true),
        };
        let json = serde_json::to_string(&decision).unwrap();
        assert!(json.contains("\"behavior\":\"deny\""));
        assert!(json.contains("no access"));
    }
}
