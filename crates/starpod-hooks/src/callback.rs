//! Hook callback types — the function signatures and matcher configuration.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error;
use crate::input::HookInput;
use crate::output::HookOutput;

/// Type alias for hook callback functions.
///
/// A hook callback receives:
/// - `input`: typed hook input data
/// - `tool_use_id`: optional correlation ID for tool-related hooks
/// - `cancellation`: a tokio CancellationToken for aborting
///
/// Returns a [`HookOutput`] that controls the agent's behavior.
///
/// # Example
///
/// ```
/// use starpod_hooks::{hook_fn, HookInput, HookOutput};
///
/// let callback = hook_fn(|input, _tool_use_id, _cancel| async move {
///     println!("Hook fired for: {}", input.event_name());
///     Ok(HookOutput::default())
/// });
/// ```
pub type HookCallback = Arc<
    dyn Fn(
            HookInput,
            Option<String>,
            tokio_util::sync::CancellationToken,
        ) -> Pin<Box<dyn Future<Output = error::Result<HookOutput>> + Send>>
        + Send
        + Sync,
>;

/// Helper to create a [`HookCallback`] from an async function.
///
/// # Example
///
/// ```
/// use starpod_hooks::{hook_fn, HookOutput};
///
/// let my_hook = hook_fn(|_input, _id, _cancel| async move {
///     Ok(HookOutput::default())
/// });
/// ```
pub fn hook_fn<F, Fut>(f: F) -> HookCallback
where
    F: Fn(HookInput, Option<String>, tokio_util::sync::CancellationToken) -> Fut
        + Send
        + Sync
        + 'static,
    Fut: Future<Output = error::Result<HookOutput>> + Send + 'static,
{
    Arc::new(move |input, tool_use_id, cancel| Box::pin(f(input, tool_use_id, cancel)))
}

/// Hook configuration with optional regex matcher pattern.
///
/// Groups one or more callbacks with a regex filter. The matcher pattern
/// is tested against the hook's filter field (typically the tool name for
/// tool-related hooks). If no matcher is set, the hooks run for every
/// event of their type.
///
/// # Example
///
/// ```
/// use starpod_hooks::{hook_fn, HookCallbackMatcher, HookOutput};
///
/// let matcher = HookCallbackMatcher::new(vec![
///     hook_fn(|_input, _id, _cancel| async move {
///         Ok(HookOutput::default())
///     }),
/// ])
/// .with_matcher("Bash|Write")
/// .with_timeout(30);
///
/// assert!(matcher.matches("Bash").unwrap());
/// assert!(!matcher.matches("Read").unwrap());
/// ```
#[derive(Clone)]
pub struct HookCallbackMatcher {
    /// Human-readable name for this hook group (used by circuit breaker and logging).
    pub name: Option<String>,

    /// Regex pattern to match against the event's filter field (e.g., tool name).
    /// If None, the hook runs for every event of its type.
    pub matcher: Option<String>,

    /// Array of callback functions to execute when the pattern matches.
    pub hooks: Vec<HookCallback>,

    /// Timeout in seconds for all hooks in this matcher.
    pub timeout: Option<u64>,

    /// Eligibility requirements (binaries, env vars, OS).
    pub requires: Option<crate::eligibility::HookRequirements>,
}

impl HookCallbackMatcher {
    pub fn new(hooks: Vec<HookCallback>) -> Self {
        Self {
            name: None,
            matcher: None,
            hooks,
            timeout: None,
            requires: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_matcher(mut self, matcher: impl Into<String>) -> Self {
        self.matcher = Some(matcher.into());
        self
    }

    pub fn with_timeout(mut self, timeout: u64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_requirements(mut self, requires: crate::eligibility::HookRequirements) -> Self {
        self.requires = Some(requires);
        self
    }

    /// Check if this matcher applies to the given target string.
    ///
    /// Returns `Ok(true)` if no matcher is set (matches everything) or
    /// if the regex pattern matches the target.
    pub fn matches(&self, target: &str) -> error::Result<bool> {
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
            .field("name", &self.name)
            .field("matcher", &self.matcher)
            .field("hooks_count", &self.hooks.len())
            .field("timeout", &self.timeout)
            .field("requires", &self.requires)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_hook() -> HookCallback {
        hook_fn(|_input, _id, _cancel| async move { Ok(HookOutput::default()) })
    }

    #[test]
    fn matcher_no_pattern_matches_everything() {
        let m = HookCallbackMatcher::new(vec![noop_hook()]);
        assert!(m.matches("Bash").unwrap());
        assert!(m.matches("anything").unwrap());
        assert!(m.matches("").unwrap());
    }

    #[test]
    fn matcher_regex_filters() {
        let m = HookCallbackMatcher::new(vec![noop_hook()]).with_matcher("Bash|Write");
        assert!(m.matches("Bash").unwrap());
        assert!(m.matches("Write").unwrap());
        assert!(!m.matches("Read").unwrap());
        assert!(!m.matches("Edit").unwrap());
    }

    #[test]
    fn matcher_invalid_regex_returns_error() {
        let m = HookCallbackMatcher::new(vec![noop_hook()]).with_matcher("[invalid");
        assert!(m.matches("test").is_err());
    }

    #[test]
    fn matcher_with_timeout() {
        let m = HookCallbackMatcher::new(vec![noop_hook()]).with_timeout(30);
        assert_eq!(m.timeout, Some(30));
    }

    #[test]
    fn matcher_with_name() {
        let m = HookCallbackMatcher::new(vec![noop_hook()]).with_name("my-hook");
        assert_eq!(m.name.as_deref(), Some("my-hook"));
    }

    #[test]
    fn matcher_with_requirements() {
        use crate::eligibility::HookRequirements;
        let req = HookRequirements {
            bins: vec!["sh".into()],
            ..Default::default()
        };
        let m = HookCallbackMatcher::new(vec![noop_hook()]).with_requirements(req);
        assert!(m.requires.is_some());
        assert_eq!(m.requires.unwrap().bins, vec!["sh"]);
    }

    #[test]
    fn matcher_builder_chaining() {
        use crate::eligibility::HookRequirements;
        let m = HookCallbackMatcher::new(vec![noop_hook()])
            .with_name("lint")
            .with_matcher("Write|Edit")
            .with_timeout(10)
            .with_requirements(HookRequirements {
                os: vec!["macos".into()],
                ..Default::default()
            });

        assert_eq!(m.name.as_deref(), Some("lint"));
        assert_eq!(m.matcher.as_deref(), Some("Write|Edit"));
        assert_eq!(m.timeout, Some(10));
        assert!(m.requires.is_some());
    }

    #[test]
    fn matcher_debug_shows_hook_count() {
        let m = HookCallbackMatcher::new(vec![noop_hook(), noop_hook()]).with_matcher("test");
        let debug = format!("{:?}", m);
        assert!(debug.contains("hooks_count: 2"));
        assert!(debug.contains("test"));
    }

    #[test]
    fn matcher_debug_includes_name_and_requires() {
        use crate::eligibility::HookRequirements;
        let m = HookCallbackMatcher::new(vec![noop_hook()])
            .with_name("my-hook")
            .with_requirements(HookRequirements::default());
        let debug = format!("{:?}", m);
        assert!(
            debug.contains("my-hook"),
            "debug should contain name: {}",
            debug
        );
        assert!(
            debug.contains("requires"),
            "debug should contain requires: {}",
            debug
        );
    }

    #[tokio::test]
    async fn hook_fn_creates_callable_callback() {
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();

        let hook = hook_fn(move |_input, _id, _cancel| {
            let called = called_clone.clone();
            async move {
                called.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(HookOutput::default())
            }
        });

        let input = HookInput::UserPromptSubmit {
            base: crate::input::BaseHookInput {
                session_id: "test".into(),
                transcript_path: String::new(),
                cwd: "/tmp".into(),
                permission_mode: None,
                agent_id: None,
                agent_type: None,
            },
            prompt: "hello".into(),
        };

        let cancel = tokio_util::sync::CancellationToken::new();
        let result = hook(input, None, cancel).await;
        assert!(result.is_ok());
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
