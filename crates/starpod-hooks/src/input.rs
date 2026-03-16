//! Hook input types — the data passed to hook callbacks.

use serde::{Deserialize, Serialize};
use crate::permissions::PermissionUpdate;

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

/// Union of all hook input types, tagged by event name.
///
/// Each variant carries the [`BaseHookInput`] plus event-specific fields.
///
/// # Example
///
/// ```
/// use starpod_hooks::{HookInput, BaseHookInput};
///
/// let input = HookInput::UserPromptSubmit {
///     base: BaseHookInput {
///         session_id: "sess-1".into(),
///         transcript_path: String::new(),
///         cwd: "/tmp".into(),
///         permission_mode: None,
///         agent_id: None,
///         agent_type: None,
///     },
///     prompt: "Hello!".into(),
/// };
/// assert_eq!(input.event_name(), "UserPromptSubmit");
/// assert!(input.tool_name().is_none());
/// ```
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
            HookInput::PreToolUse { tool_name, .. }
            | HookInput::PostToolUse { tool_name, .. }
            | HookInput::PostToolUseFailure { tool_name, .. }
            | HookInput::PermissionRequest { tool_name, .. } => Some(tool_name),
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

    /// Returns a reference to the base input fields.
    pub fn base(&self) -> &BaseHookInput {
        match self {
            HookInput::PreToolUse { base, .. }
            | HookInput::PostToolUse { base, .. }
            | HookInput::PostToolUseFailure { base, .. }
            | HookInput::Notification { base, .. }
            | HookInput::UserPromptSubmit { base, .. }
            | HookInput::SessionStart { base, .. }
            | HookInput::SessionEnd { base, .. }
            | HookInput::Stop { base, .. }
            | HookInput::SubagentStart { base, .. }
            | HookInput::SubagentStop { base, .. }
            | HookInput::PreCompact { base, .. }
            | HookInput::PermissionRequest { base, .. }
            | HookInput::Setup { base, .. }
            | HookInput::TeammateIdle { base, .. }
            | HookInput::TaskCompleted { base, .. }
            | HookInput::ConfigChange { base, .. }
            | HookInput::WorktreeCreate { base, .. }
            | HookInput::WorktreeRemove { base, .. } => base,
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
    /// First-time initialization (BOOTSTRAP.md).
    Init,
    /// Routine maintenance.
    Maintenance,
    /// Server boot (BOOT.md) — fires on every server start.
    Boot,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> BaseHookInput {
        BaseHookInput {
            session_id: "sess-1".into(),
            transcript_path: "/tmp/transcript.json".into(),
            cwd: "/projects/test".into(),
            permission_mode: Some("default".into()),
            agent_id: None,
            agent_type: None,
        }
    }

    #[test]
    fn tool_name_returns_some_for_tool_events() {
        let input = HookInput::PreToolUse {
            base: base(),
            tool_name: "Bash".into(),
            tool_input: serde_json::json!({"command": "ls"}),
            tool_use_id: "tu-1".into(),
        };
        assert_eq!(input.tool_name(), Some("Bash"));
    }

    #[test]
    fn tool_name_returns_none_for_non_tool_events() {
        let input = HookInput::SessionStart {
            base: base(),
            source: SessionStartSource::Startup,
            model: Some("claude-haiku-4-5".into()),
        };
        assert_eq!(input.tool_name(), None);
    }

    #[test]
    fn event_name_matches_variant() {
        let input = HookInput::PostToolUseFailure {
            base: base(),
            tool_name: "Write".into(),
            tool_input: serde_json::json!({}),
            tool_use_id: "tu-2".into(),
            error: "permission denied".into(),
            is_interrupt: Some(false),
        };
        assert_eq!(input.event_name(), "PostToolUseFailure");
    }

    #[test]
    fn base_accessor_returns_correct_fields() {
        let b = base();
        let input = HookInput::Stop {
            base: b.clone(),
            stop_hook_active: true,
            last_assistant_message: None,
        };
        assert_eq!(input.base().session_id, "sess-1");
        assert_eq!(input.base().cwd, "/projects/test");
    }

    #[test]
    fn serde_roundtrip_tagged() {
        let input = HookInput::UserPromptSubmit {
            base: base(),
            prompt: "hello world".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"hook_event_name\":\"UserPromptSubmit\""));
        let back: HookInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.event_name(), "UserPromptSubmit");
    }

    #[test]
    fn session_start_source_serde() {
        let src = SessionStartSource::Resume;
        let json = serde_json::to_string(&src).unwrap();
        assert_eq!(json, "\"resume\"");
        let back: SessionStartSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SessionStartSource::Resume);
    }

    #[test]
    fn config_change_source_serde() {
        let src = ConfigChangeSource::ProjectSettings;
        let json = serde_json::to_string(&src).unwrap();
        assert_eq!(json, "\"project_settings\"");
    }

    #[test]
    fn all_event_names_covered() {
        // Ensure event_name() returns non-empty for every variant
        let inputs = vec![
            HookInput::PreToolUse { base: base(), tool_name: "t".into(), tool_input: serde_json::json!(null), tool_use_id: "id".into() },
            HookInput::PostToolUse { base: base(), tool_name: "t".into(), tool_input: serde_json::json!(null), tool_response: serde_json::json!(null), tool_use_id: "id".into() },
            HookInput::PostToolUseFailure { base: base(), tool_name: "t".into(), tool_input: serde_json::json!(null), tool_use_id: "id".into(), error: "e".into(), is_interrupt: None },
            HookInput::Notification { base: base(), message: "m".into(), title: None, notification_type: "info".into() },
            HookInput::UserPromptSubmit { base: base(), prompt: "p".into() },
            HookInput::SessionStart { base: base(), source: SessionStartSource::Startup, model: None },
            HookInput::SessionEnd { base: base(), reason: "done".into() },
            HookInput::Stop { base: base(), stop_hook_active: false, last_assistant_message: None },
            HookInput::SubagentStart { base: base(), agent_id: "a".into(), agent_type: "general".into() },
            HookInput::SubagentStop { base: base(), stop_hook_active: false, agent_id: "a".into(), agent_transcript_path: "/t".into(), agent_type: "general".into(), last_assistant_message: None },
            HookInput::PreCompact { base: base(), trigger: CompactTriggerType::Auto, custom_instructions: None },
            HookInput::PermissionRequest { base: base(), tool_name: "t".into(), tool_input: serde_json::json!(null), permission_suggestions: None },
            HookInput::Setup { base: base(), trigger: SetupTrigger::Init },
            HookInput::TeammateIdle { base: base(), teammate_name: "n".into(), team_name: "t".into() },
            HookInput::TaskCompleted { base: base(), task_id: "1".into(), task_subject: "s".into(), task_description: None, teammate_name: None, team_name: None },
            HookInput::ConfigChange { base: base(), source: ConfigChangeSource::Skills, file_path: None },
            HookInput::WorktreeCreate { base: base(), name: "wt".into() },
            HookInput::WorktreeRemove { base: base(), worktree_path: "/wt".into() },
        ];

        for input in &inputs {
            assert!(!input.event_name().is_empty());
            // base should always be accessible
            assert!(!input.base().session_id.is_empty());
        }
        assert_eq!(inputs.len(), 18, "should cover all 18 HookInput variants");
    }
}
