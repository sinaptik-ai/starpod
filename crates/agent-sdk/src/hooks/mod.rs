pub mod input;
pub mod output;

pub use input::*;
pub use output::*;

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Available hook event types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Notification,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Stop,
    SubagentStart,
    SubagentStop,
    PreCompact,
    PermissionRequest,
    Setup,
    TeammateIdle,
    TaskCompleted,
    ConfigChange,
    WorktreeCreate,
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

/// Type alias for hook callback functions.
///
/// A hook callback receives:
/// - `input`: typed hook input data
/// - `tool_use_id`: optional correlation ID for tool-related hooks
/// - `cancellation`: a tokio CancellationToken for aborting
///
/// Returns a `HookOutput` that controls the agent's behavior.
pub type HookCallback = Arc<
    dyn Fn(
            HookInput,
            Option<String>,
            tokio_util::sync::CancellationToken,
        ) -> Pin<Box<dyn Future<Output = crate::error::Result<HookOutput>> + Send>>
        + Send
        + Sync,
>;

/// Helper to create a HookCallback from an async function.
pub fn hook_fn<F, Fut>(f: F) -> HookCallback
where
    F: Fn(HookInput, Option<String>, tokio_util::sync::CancellationToken) -> Fut
        + Send
        + Sync
        + 'static,
    Fut: Future<Output = crate::error::Result<HookOutput>> + Send + 'static,
{
    Arc::new(move |input, tool_use_id, cancel| Box::pin(f(input, tool_use_id, cancel)))
}

/// Hook configuration with optional matcher pattern.
#[derive(Clone)]
pub struct HookCallbackMatcher {
    /// Regex pattern to match against the event's filter field (e.g., tool name).
    /// If None, the hook runs for every event of its type.
    pub matcher: Option<String>,

    /// Array of callback functions to execute when the pattern matches.
    pub hooks: Vec<HookCallback>,

    /// Timeout in seconds for all hooks in this matcher.
    pub timeout: Option<u64>,
}

impl HookCallbackMatcher {
    pub fn new(hooks: Vec<HookCallback>) -> Self {
        Self {
            matcher: None,
            hooks,
            timeout: None,
        }
    }

    pub fn with_matcher(mut self, matcher: impl Into<String>) -> Self {
        self.matcher = Some(matcher.into());
        self
    }

    pub fn with_timeout(mut self, timeout: u64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Check if this matcher applies to the given target string.
    pub fn matches(&self, target: &str) -> crate::error::Result<bool> {
        match &self.matcher {
            None => Ok(true),
            Some(pattern) => {
                let re = regex::Regex::new(pattern)?;
                Ok(re.is_match(target))
            }
        }
    }
}

impl fmt::Debug for HookCallbackMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookCallbackMatcher")
            .field("matcher", &self.matcher)
            .field("hooks_count", &self.hooks.len())
            .field("timeout", &self.timeout)
            .finish()
    }
}
