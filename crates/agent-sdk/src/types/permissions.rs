use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result of a permission check from the `can_use_tool` callback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "behavior")]
pub enum PermissionResult {
    #[serde(rename = "allow")]
    Allow {
        #[serde(skip_serializing_if = "Option::is_none")]
        updated_input: Option<HashMap<String, serde_json::Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        updated_permissions: Option<Vec<PermissionUpdate>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
    },
    #[serde(rename = "deny")]
    Deny {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        interrupt: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
    },
}

/// A permission update suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionUpdate {
    pub tool: String,
    pub permission: PermissionLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionLevel {
    Allow,
    Deny,
    Ask,
}

/// Options passed to the `can_use_tool` callback.
#[derive(Debug, Clone)]
pub struct CanUseToolOptions {
    pub suggestions: Vec<PermissionUpdate>,
    pub blocked_path: Option<String>,
    pub decision_reason: Option<String>,
    pub tool_use_id: String,
    pub agent_id: Option<String>,
}
