use serde::{Deserialize, Serialize};
use crate::types::permissions::PermissionUpdate;

/// Base fields shared by all hook inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseHookInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// Union of all hook input types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookInput {
    PreToolUse {
        #[serde(flatten)]
        base: BaseHookInput,
        tool_name: String,
        tool_input: serde_json::Value,
        tool_use_id: String,
    },

    PostToolUse {
        #[serde(flatten)]
        base: BaseHookInput,
        tool_name: String,
        tool_input: serde_json::Value,
        tool_response: serde_json::Value,
        tool_use_id: String,
    },

    PostToolUseFailure {
        #[serde(flatten)]
        base: BaseHookInput,
        tool_name: String,
        tool_input: serde_json::Value,
        tool_use_id: String,
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_interrupt: Option<bool>,
    },

    Notification {
        #[serde(flatten)]
        base: BaseHookInput,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        notification_type: String,
    },

    UserPromptSubmit {
        #[serde(flatten)]
        base: BaseHookInput,
        prompt: String,
    },

    SessionStart {
        #[serde(flatten)]
        base: BaseHookInput,
        source: SessionStartSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },

    SessionEnd {
        #[serde(flatten)]
        base: BaseHookInput,
        reason: String,
    },

    Stop {
        #[serde(flatten)]
        base: BaseHookInput,
        stop_hook_active: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_assistant_message: Option<String>,
    },

    SubagentStart {
        #[serde(flatten)]
        base: BaseHookInput,
        agent_id: String,
        agent_type: String,
    },

    SubagentStop {
        #[serde(flatten)]
        base: BaseHookInput,
        stop_hook_active: bool,
        agent_id: String,
        agent_transcript_path: String,
        agent_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_assistant_message: Option<String>,
    },

    PreCompact {
        #[serde(flatten)]
        base: BaseHookInput,
        trigger: CompactTriggerType,
        custom_instructions: Option<String>,
    },

    PermissionRequest {
        #[serde(flatten)]
        base: BaseHookInput,
        tool_name: String,
        tool_input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        permission_suggestions: Option<Vec<PermissionUpdate>>,
    },

    Setup {
        #[serde(flatten)]
        base: BaseHookInput,
        trigger: SetupTrigger,
    },

    TeammateIdle {
        #[serde(flatten)]
        base: BaseHookInput,
        teammate_name: String,
        team_name: String,
    },

    TaskCompleted {
        #[serde(flatten)]
        base: BaseHookInput,
        task_id: String,
        task_subject: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        task_description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        teammate_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        team_name: Option<String>,
    },

    ConfigChange {
        #[serde(flatten)]
        base: BaseHookInput,
        source: ConfigChangeSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_path: Option<String>,
    },

    WorktreeCreate {
        #[serde(flatten)]
        base: BaseHookInput,
        name: String,
    },

    WorktreeRemove {
        #[serde(flatten)]
        base: BaseHookInput,
        worktree_path: String,
    },
}

impl HookInput {
    /// Returns the tool name if this is a tool-related hook.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            HookInput::PreToolUse { tool_name, .. } => Some(tool_name),
            HookInput::PostToolUse { tool_name, .. } => Some(tool_name),
            HookInput::PostToolUseFailure { tool_name, .. } => Some(tool_name),
            HookInput::PermissionRequest { tool_name, .. } => Some(tool_name),
            _ => None,
        }
    }

    /// Returns the hook event name as a string.
    pub fn event_name(&self) -> &str {
        match self {
            HookInput::PreToolUse { .. } => "PreToolUse",
            HookInput::PostToolUse { .. } => "PostToolUse",
            HookInput::PostToolUseFailure { .. } => "PostToolUseFailure",
            HookInput::Notification { .. } => "Notification",
            HookInput::UserPromptSubmit { .. } => "UserPromptSubmit",
            HookInput::SessionStart { .. } => "SessionStart",
            HookInput::SessionEnd { .. } => "SessionEnd",
            HookInput::Stop { .. } => "Stop",
            HookInput::SubagentStart { .. } => "SubagentStart",
            HookInput::SubagentStop { .. } => "SubagentStop",
            HookInput::PreCompact { .. } => "PreCompact",
            HookInput::PermissionRequest { .. } => "PermissionRequest",
            HookInput::Setup { .. } => "Setup",
            HookInput::TeammateIdle { .. } => "TeammateIdle",
            HookInput::TaskCompleted { .. } => "TaskCompleted",
            HookInput::ConfigChange { .. } => "ConfigChange",
            HookInput::WorktreeCreate { .. } => "WorktreeCreate",
            HookInput::WorktreeRemove { .. } => "WorktreeRemove",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStartSource {
    Startup,
    Resume,
    Clear,
    Compact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompactTriggerType {
    Manual,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SetupTrigger {
    Init,
    Maintenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigChangeSource {
    UserSettings,
    ProjectSettings,
    LocalSettings,
    PolicySettings,
    Skills,
}
