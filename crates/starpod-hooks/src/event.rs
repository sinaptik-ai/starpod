use serde::{Deserialize, Serialize};
use std::fmt;

/// Available hook event types.
///
/// Each variant represents a lifecycle point where hooks can observe or
/// control agent behavior.
///
/// # Example
///
/// ```
/// use starpod_hooks::HookEvent;
///
/// let event = HookEvent::PostToolUse;
/// assert_eq!(event.to_string(), "PostToolUse");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    /// Before a tool is executed. Can modify input or block execution.
    PreToolUse,
    /// After successful tool execution. Can observe results.
    PostToolUse,
    /// When tool execution fails.
    PostToolUseFailure,
    /// System notification (info, warning, error).
    Notification,
    /// When the user submits a prompt.
    UserPromptSubmit,
    /// When a session begins.
    SessionStart,
    /// When a session ends.
    SessionEnd,
    /// Agent stop event.
    Stop,
    /// Before a subagent starts.
    SubagentStart,
    /// After a subagent stops.
    SubagentStop,
    /// Before conversation compaction.
    PreCompact,
    /// When a permission decision is requested.
    PermissionRequest,
    /// Initial setup event.
    Setup,
    /// Teammate idle notification.
    TeammateIdle,
    /// Task completion event.
    TaskCompleted,
    /// Configuration change event.
    ConfigChange,
    /// Git worktree created.
    WorktreeCreate,
    /// Git worktree removed.
    WorktreeRemove,
}

impl fmt::Display for HookEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookEvent::PreToolUse => write!(f, "PreToolUse"),
            HookEvent::PostToolUse => write!(f, "PostToolUse"),
            HookEvent::PostToolUseFailure => write!(f, "PostToolUseFailure"),
            HookEvent::Notification => write!(f, "Notification"),
            HookEvent::UserPromptSubmit => write!(f, "UserPromptSubmit"),
            HookEvent::SessionStart => write!(f, "SessionStart"),
            HookEvent::SessionEnd => write!(f, "SessionEnd"),
            HookEvent::Stop => write!(f, "Stop"),
            HookEvent::SubagentStart => write!(f, "SubagentStart"),
            HookEvent::SubagentStop => write!(f, "SubagentStop"),
            HookEvent::PreCompact => write!(f, "PreCompact"),
            HookEvent::PermissionRequest => write!(f, "PermissionRequest"),
            HookEvent::Setup => write!(f, "Setup"),
            HookEvent::TeammateIdle => write!(f, "TeammateIdle"),
            HookEvent::TaskCompleted => write!(f, "TaskCompleted"),
            HookEvent::ConfigChange => write!(f, "ConfigChange"),
            HookEvent::WorktreeCreate => write!(f, "WorktreeCreate"),
            HookEvent::WorktreeRemove => write!(f, "WorktreeRemove"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_variant_name() {
        assert_eq!(HookEvent::PreToolUse.to_string(), "PreToolUse");
        assert_eq!(HookEvent::PostToolUse.to_string(), "PostToolUse");
        assert_eq!(HookEvent::SessionStart.to_string(), "SessionStart");
        assert_eq!(HookEvent::WorktreeRemove.to_string(), "WorktreeRemove");
    }

    #[test]
    fn hook_event_equality() {
        assert_eq!(HookEvent::Stop, HookEvent::Stop);
        assert_ne!(HookEvent::Stop, HookEvent::Setup);
    }

    #[test]
    fn hook_event_hash_works_in_hashmap() {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(HookEvent::PreToolUse, "pre");
        map.insert(HookEvent::PostToolUse, "post");
        assert_eq!(map.get(&HookEvent::PreToolUse), Some(&"pre"));
        assert_eq!(map.get(&HookEvent::PostToolUse), Some(&"post"));
        assert_eq!(map.get(&HookEvent::Stop), None);
    }

    #[test]
    fn serde_roundtrip() {
        let event = HookEvent::ConfigChange;
        let json = serde_json::to_string(&event).unwrap();
        let back: HookEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }
}
