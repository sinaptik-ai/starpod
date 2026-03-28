//! Hook execution engine — runs matched hooks with timeout and cancellation support.

use std::collections::HashMap;

use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::callback::HookCallbackMatcher;
use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use crate::event::HookEvent;
use crate::input::{BaseHookInput, HookInput};
use crate::output::HookOutput;

/// A registry of hooks keyed by event type.
///
/// Wraps a `HashMap<HookEvent, Vec<HookCallbackMatcher>>` and provides
/// methods to run hooks for specific events.
///
/// # Example
///
/// ```
/// use starpod_hooks::{HookRegistry, HookEvent, HookCallbackMatcher, hook_fn, HookOutput};
///
/// let mut registry = HookRegistry::new();
/// registry.register(HookEvent::PostToolUse, vec![
///     HookCallbackMatcher::new(vec![
///         hook_fn(|_input, _id, _cancel| async move {
///             Ok(HookOutput::default())
///         }),
///     ]).with_matcher("Bash"),
/// ]);
///
/// assert!(registry.has_hooks(&HookEvent::PostToolUse));
/// assert!(!registry.has_hooks(&HookEvent::PreToolUse));
/// ```
#[derive(Debug, Default)]
pub struct HookRegistry {
    hooks: HashMap<HookEvent, Vec<HookCallbackMatcher>>,
    circuit_breaker: CircuitBreaker,
    /// Cache for eligibility check results (keyed by matcher name).
    eligibility_cache: std::sync::Mutex<HashMap<String, bool>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry from an existing HashMap.
    pub fn from_map(hooks: HashMap<HookEvent, Vec<HookCallbackMatcher>>) -> Self {
        Self {
            hooks,
            ..Default::default()
        }
    }

    /// Set a custom circuit breaker configuration.
    pub fn with_circuit_breaker(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker = CircuitBreaker::new(config);
        self
    }

    /// Register matchers for a hook event.
    pub fn register(&mut self, event: HookEvent, matchers: Vec<HookCallbackMatcher>) {
        self.hooks.insert(event, matchers);
    }

    /// Check if any hooks are registered for the given event.
    pub fn has_hooks(&self, event: &HookEvent) -> bool {
        self.hooks.get(event).is_some_and(|m| !m.is_empty())
    }

    /// Get the matchers for a given event.
    pub fn get(&self, event: &HookEvent) -> Option<&Vec<HookCallbackMatcher>> {
        self.hooks.get(event)
    }

    /// Consume the registry and return the inner HashMap.
    pub fn into_map(self) -> HashMap<HookEvent, Vec<HookCallbackMatcher>> {
        self.hooks
    }

    /// Merge another registry's hooks into this one.
    pub fn merge(&mut self, other: HookRegistry) {
        for (event, matchers) in other.hooks {
            self.hooks.entry(event).or_default().extend(matchers);
        }
    }

    /// Check whether a matcher is eligible to run (circuit breaker + requirements).
    ///
    /// Returns `true` if the matcher should be executed.
    fn is_matcher_eligible(&self, matcher: &HookCallbackMatcher) -> bool {
        if let Some(ref name) = matcher.name {
            // Circuit breaker check
            if self.circuit_breaker.is_tripped(name) {
                debug!(hook = %name, "Skipping hook — circuit breaker is open");
                return false;
            }
        }

        // Eligibility requirements check
        if let Some(ref requires) = matcher.requires {
            // Use cache if matcher has a name
            if let Some(ref name) = matcher.name {
                let cache = self.eligibility_cache.lock().unwrap();
                if let Some(&eligible) = cache.get(name) {
                    return eligible;
                }
                drop(cache); // release lock before check

                let eligible = match requires.check() {
                    Ok(()) => true,
                    Err(e) => {
                        debug!(hook = %name, reason = %e, "Skipping hook — eligibility check failed");
                        false
                    }
                };

                self.eligibility_cache
                    .lock()
                    .unwrap()
                    .insert(name.clone(), eligible);
                return eligible;
            }

            // No name — check without caching
            if let Err(e) = requires.check() {
                debug!(reason = %e, "Skipping unnamed hook — eligibility check failed");
                return false;
            }
        }

        true
    }

    /// Run all matching hooks for PostToolUse.
    ///
    /// Hooks are fire-and-forget: errors are logged but do not propagate.
    pub async fn run_post_tool_use(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_response: &serde_json::Value,
        tool_use_id: &str,
        session_id: &str,
        cwd: &str,
    ) {
        if let Some(matchers) = self.hooks.get(&HookEvent::PostToolUse) {
            self.run_hooks_for_tool(
                matchers,
                HookEvent::PostToolUse,
                tool_name,
                tool_input,
                Some(tool_response),
                tool_use_id,
                session_id,
                cwd,
            )
            .await;
        }
    }

    /// Run all matching hooks for PreToolUse.
    ///
    /// Returns the merged [`HookOutput`] from all matching hooks,
    /// or `None` if no hooks matched.
    pub async fn run_pre_tool_use(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_use_id: &str,
        session_id: &str,
        cwd: &str,
    ) -> Option<HookOutput> {
        let matchers = self.hooks.get(&HookEvent::PreToolUse)?;
        self.run_hooks_for_tool_with_output(
            matchers,
            HookEvent::PreToolUse,
            tool_name,
            tool_input,
            None,
            tool_use_id,
            session_id,
            cwd,
        )
        .await
    }

    /// Run hooks for a generic (non-tool) event.
    ///
    /// Fires all registered hooks for the event. Errors are logged.
    pub async fn run_event(&self, event: &HookEvent, input: HookInput) {
        if let Some(matchers) = self.hooks.get(event) {
            self.run_generic_hooks(matchers, input).await;
        }
    }

    /// Run hooks that match a tool name, fire-and-forget style.
    #[allow(clippy::too_many_arguments)]
    async fn run_hooks_for_tool(
        &self,
        matchers: &[HookCallbackMatcher],
        event: HookEvent,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_response: Option<&serde_json::Value>,
        tool_use_id: &str,
        session_id: &str,
        cwd: &str,
    ) {
        for matcher in matchers {
            if !matcher.matches(tool_name).unwrap_or(false) {
                continue;
            }
            if !self.is_matcher_eligible(matcher) {
                continue;
            }

            let input = build_tool_hook_input(
                &event,
                tool_name,
                tool_input,
                tool_response,
                tool_use_id,
                session_id,
                cwd,
            );

            let cancel = CancellationToken::new();
            let timeout_secs = matcher.timeout;
            let mut any_failed = false;
            let mut all_succeeded = true;

            for hook in &matcher.hooks {
                let fut = hook(input.clone(), Some(tool_use_id.to_string()), cancel.clone());

                if let Some(secs) = timeout_secs {
                    match tokio::time::timeout(std::time::Duration::from_secs(secs), fut).await {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            warn!("{} hook error: {}", event, e);
                            any_failed = true;
                            all_succeeded = false;
                        }
                        Err(_) => {
                            warn!("{} hook timed out after {}s", event, secs);
                            any_failed = true;
                            all_succeeded = false;
                        }
                    }
                } else if let Err(e) = fut.await {
                    warn!("{} hook error: {}", event, e);
                    any_failed = true;
                    all_succeeded = false;
                }
            }

            // Update circuit breaker for named hooks
            if let Some(ref name) = matcher.name {
                if any_failed {
                    self.circuit_breaker.record_failure(name);
                } else if all_succeeded {
                    self.circuit_breaker.record_success(name);
                }
            }
        }
    }

    /// Run hooks that match a tool name, collecting the last sync output.
    #[allow(clippy::too_many_arguments)]
    async fn run_hooks_for_tool_with_output(
        &self,
        matchers: &[HookCallbackMatcher],
        event: HookEvent,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_response: Option<&serde_json::Value>,
        tool_use_id: &str,
        session_id: &str,
        cwd: &str,
    ) -> Option<HookOutput> {
        let mut last_output: Option<HookOutput> = None;

        for matcher in matchers {
            if !matcher.matches(tool_name).unwrap_or(false) {
                continue;
            }
            if !self.is_matcher_eligible(matcher) {
                continue;
            }

            let input = build_tool_hook_input(
                &event,
                tool_name,
                tool_input,
                tool_response,
                tool_use_id,
                session_id,
                cwd,
            );

            let cancel = CancellationToken::new();
            let timeout_secs = matcher.timeout;
            let mut any_failed = false;
            let mut all_succeeded = true;

            for hook in &matcher.hooks {
                let fut = hook(input.clone(), Some(tool_use_id.to_string()), cancel.clone());

                let result = if let Some(secs) = timeout_secs {
                    match tokio::time::timeout(std::time::Duration::from_secs(secs), fut).await {
                        Ok(r) => r,
                        Err(_) => {
                            warn!("{} hook timed out after {}s", event, secs);
                            any_failed = true;
                            all_succeeded = false;
                            continue;
                        }
                    }
                } else {
                    fut.await
                };

                match result {
                    Ok(output) => last_output = Some(output),
                    Err(e) => {
                        warn!("{} hook error: {}", event, e);
                        any_failed = true;
                        all_succeeded = false;
                    }
                }
            }

            // Update circuit breaker for named hooks
            if let Some(ref name) = matcher.name {
                if any_failed {
                    self.circuit_breaker.record_failure(name);
                } else if all_succeeded {
                    self.circuit_breaker.record_success(name);
                }
            }
        }

        last_output
    }

    /// Run hooks for a non-tool event (no regex matching on tool name).
    async fn run_generic_hooks(&self, matchers: &[HookCallbackMatcher], input: HookInput) {
        let cancel = CancellationToken::new();

        for matcher in matchers {
            if !self.is_matcher_eligible(matcher) {
                continue;
            }

            let timeout_secs = matcher.timeout;
            let mut any_failed = false;
            let mut all_succeeded = true;

            for hook in &matcher.hooks {
                let fut = hook(input.clone(), None, cancel.clone());

                if let Some(secs) = timeout_secs {
                    match tokio::time::timeout(std::time::Duration::from_secs(secs), fut).await {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            warn!("Hook error: {}", e);
                            any_failed = true;
                            all_succeeded = false;
                        }
                        Err(_) => {
                            warn!("Hook timed out after {}s", secs);
                            any_failed = true;
                            all_succeeded = false;
                        }
                    }
                } else if let Err(e) = fut.await {
                    warn!("Hook error: {}", e);
                    any_failed = true;
                    all_succeeded = false;
                }
            }

            if let Some(ref name) = matcher.name {
                if any_failed {
                    self.circuit_breaker.record_failure(name);
                } else if all_succeeded {
                    self.circuit_breaker.record_success(name);
                }
            }
        }
    }
}

/// Build a HookInput for tool-related events.
fn build_tool_hook_input(
    event: &HookEvent,
    tool_name: &str,
    tool_input: &serde_json::Value,
    tool_response: Option<&serde_json::Value>,
    tool_use_id: &str,
    session_id: &str,
    cwd: &str,
) -> HookInput {
    let base = BaseHookInput {
        session_id: session_id.to_string(),
        transcript_path: String::new(),
        cwd: cwd.to_string(),
        permission_mode: None,
        agent_id: None,
        agent_type: None,
    };

    match event {
        HookEvent::PostToolUse => HookInput::PostToolUse {
            base,
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
            tool_response: tool_response.cloned().unwrap_or_default(),
            tool_use_id: tool_use_id.to_string(),
        },
        HookEvent::PreToolUse => HookInput::PreToolUse {
            base,
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
            tool_use_id: tool_use_id.to_string(),
        },
        HookEvent::PostToolUseFailure => HookInput::PostToolUseFailure {
            base,
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
            tool_use_id: tool_use_id.to_string(),
            error: tool_response
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            is_interrupt: None,
        },
        // For other events, fallback to PostToolUse shape (shouldn't happen
        // in practice since callers use the right event).
        _ => HookInput::PostToolUse {
            base,
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
            tool_response: tool_response.cloned().unwrap_or_default(),
            tool_use_id: tool_use_id.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::callback::{hook_fn, HookCallbackMatcher};
    use crate::output::HookOutput;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn registry_new_is_empty() {
        let reg = HookRegistry::new();
        assert!(!reg.has_hooks(&HookEvent::PostToolUse));
        assert!(!reg.has_hooks(&HookEvent::PreToolUse));
    }

    #[test]
    fn registry_register_and_has_hooks() {
        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                |_i, _id, _c| async { Ok(HookOutput::default()) },
            )])],
        );
        assert!(reg.has_hooks(&HookEvent::PostToolUse));
        assert!(!reg.has_hooks(&HookEvent::PreToolUse));
    }

    #[test]
    fn registry_from_map_and_into_map() {
        let mut map = HashMap::new();
        map.insert(
            HookEvent::Stop,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                |_i, _id, _c| async { Ok(HookOutput::default()) },
            )])],
        );
        let reg = HookRegistry::from_map(map);
        assert!(reg.has_hooks(&HookEvent::Stop));
        let map = reg.into_map();
        assert!(map.contains_key(&HookEvent::Stop));
    }

    #[test]
    fn registry_get_returns_matchers() {
        let mut reg = HookRegistry::new();
        let matcher = HookCallbackMatcher::new(vec![hook_fn(|_i, _id, _c| async {
            Ok(HookOutput::default())
        })])
        .with_matcher("Bash");
        reg.register(HookEvent::PostToolUse, vec![matcher]);
        let matchers = reg.get(&HookEvent::PostToolUse).unwrap();
        assert_eq!(matchers.len(), 1);
        assert_eq!(matchers[0].matcher.as_deref(), Some("Bash"));
    }

    #[tokio::test]
    async fn run_post_tool_use_fires_matching_hooks() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(move |_i, _id, _c| {
                let counter = counter_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(HookOutput::default())
                }
            })])
            .with_matcher("Bash")],
        );

        // Should fire for Bash
        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({"command": "ls"}),
            &serde_json::json!("output"),
            "tu-1",
            "sess-1",
            "/tmp",
        )
        .await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Should NOT fire for Read (doesn't match "Bash" regex)
        reg.run_post_tool_use(
            "Read",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "tu-2",
            "sess-1",
            "/tmp",
        )
        .await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn run_post_tool_use_no_matcher_fires_for_all() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                move |_i, _id, _c| {
                    let counter = counter_clone.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(HookOutput::default())
                    }
                },
            )])],
        );

        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "tu-1",
            "s",
            "/tmp",
        )
        .await;
        reg.run_post_tool_use(
            "Read",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "tu-2",
            "s",
            "/tmp",
        )
        .await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn run_pre_tool_use_returns_output() {
        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PreToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                |_i, _id, _c| async {
                    Ok(HookOutput::Sync(crate::output::SyncHookOutput {
                        decision: Some(crate::output::HookDecision::Block),
                        reason: Some("blocked".into()),
                        ..Default::default()
                    }))
                },
            )])],
        );

        let output = reg
            .run_pre_tool_use("Bash", &serde_json::json!({}), "tu-1", "s", "/tmp")
            .await;
        assert!(output.is_some());
        match output.unwrap() {
            HookOutput::Sync(sync) => {
                assert_eq!(sync.decision, Some(crate::output::HookDecision::Block));
            }
            _ => panic!("expected sync output"),
        }
    }

    #[tokio::test]
    async fn run_pre_tool_use_returns_none_when_no_hooks() {
        let reg = HookRegistry::new();
        let output = reg
            .run_pre_tool_use("Bash", &serde_json::json!({}), "tu-1", "s", "/tmp")
            .await;
        assert!(output.is_none());
    }

    #[tokio::test]
    async fn run_event_fires_generic_hooks() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::SessionStart,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                move |_i, _id, _c| {
                    let counter = counter_clone.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(HookOutput::default())
                    }
                },
            )])],
        );

        let input = HookInput::SessionStart {
            base: BaseHookInput {
                session_id: "s".into(),
                transcript_path: String::new(),
                cwd: "/tmp".into(),
                permission_mode: None,
                agent_id: None,
                agent_type: None,
            },
            source: crate::input::SessionStartSource::Startup,
            model: None,
        };

        reg.run_event(&HookEvent::SessionStart, input).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hook_error_is_logged_not_propagated() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![
                // First hook errors
                hook_fn(|_i, _id, _c| async {
                    Err(crate::error::HookError::CallbackFailed("oops".into()))
                }),
                // Second hook should still run
                hook_fn(move |_i, _id, _c| {
                    let counter = counter_clone.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(HookOutput::default())
                    }
                }),
            ])],
        );

        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "tu-1",
            "s",
            "/tmp",
        )
        .await;
        // Second hook should have fired despite the first one erroring
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hook_timeout_is_enforced() {
        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(|_i, _id, _c| async {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Ok(HookOutput::default())
            })])
            .with_timeout(1)], // 1 second timeout
        );

        let start = std::time::Instant::now();
        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "tu-1",
            "s",
            "/tmp",
        )
        .await;
        let elapsed = start.elapsed();
        // Should complete in ~1s, not 10s
        assert!(
            elapsed.as_secs() < 3,
            "hook should have timed out, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn multiple_matchers_all_fire() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c1 = counter.clone();
        let c2 = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![
                HookCallbackMatcher::new(vec![hook_fn(move |_i, _id, _c| {
                    let c = c1.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Ok(HookOutput::default())
                    }
                })])
                .with_matcher("Bash"),
                HookCallbackMatcher::new(vec![hook_fn(move |_i, _id, _c| {
                    let c = c2.clone();
                    async move {
                        c.fetch_add(10, Ordering::SeqCst);
                        Ok(HookOutput::default())
                    }
                })])
                .with_matcher("Bash|Read"),
            ],
        );

        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "tu-1",
            "s",
            "/tmp",
        )
        .await;
        assert_eq!(counter.load(Ordering::SeqCst), 11); // both matchers fired
    }

    #[tokio::test]
    async fn run_post_tool_use_noop_when_no_hooks_registered() {
        let reg = HookRegistry::new();
        // Should not panic or error
        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "tu-1",
            "s",
            "/tmp",
        )
        .await;
    }

    #[tokio::test]
    async fn run_event_noop_when_no_hooks_registered() {
        let reg = HookRegistry::new();
        let input = HookInput::SessionEnd {
            base: BaseHookInput {
                session_id: "s".into(),
                transcript_path: String::new(),
                cwd: "/tmp".into(),
                permission_mode: None,
                agent_id: None,
                agent_type: None,
            },
            reason: "user closed".into(),
        };
        // Should not panic
        reg.run_event(&HookEvent::SessionEnd, input).await;
    }

    #[tokio::test]
    async fn hook_receives_correct_input_fields() {
        let received_tool = Arc::new(std::sync::Mutex::new(String::new()));
        let received_clone = received_tool.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                move |input, tool_use_id, _c| {
                    let received = received_clone.clone();
                    async move {
                        if let HookInput::PostToolUse {
                            tool_name, base, ..
                        } = &input
                        {
                            *received.lock().unwrap() = format!(
                                "{}:{}:{}",
                                tool_name,
                                base.session_id,
                                tool_use_id.unwrap_or_default()
                            );
                        }
                        Ok(HookOutput::default())
                    }
                },
            )])],
        );

        reg.run_post_tool_use(
            "Write",
            &serde_json::json!({"file_path": "/tmp/test"}),
            &serde_json::json!("ok"),
            "tu-42",
            "sess-abc",
            "/projects/foo",
        )
        .await;

        assert_eq!(*received_tool.lock().unwrap(), "Write:sess-abc:tu-42");
    }

    #[tokio::test]
    async fn pre_tool_use_non_matching_returns_none() {
        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PreToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(|_i, _id, _c| async {
                Ok(HookOutput::Sync(crate::output::SyncHookOutput {
                    decision: Some(crate::output::HookDecision::Block),
                    ..Default::default()
                }))
            })])
            .with_matcher("Write")],
        );

        // "Bash" doesn't match "Write" regex
        let output = reg
            .run_pre_tool_use("Bash", &serde_json::json!({}), "tu-1", "s", "/tmp")
            .await;
        assert!(output.is_none());
    }

    #[tokio::test]
    async fn has_hooks_false_for_empty_matchers_vec() {
        let mut reg = HookRegistry::new();
        reg.register(HookEvent::Stop, vec![]); // registered but empty
        assert!(!reg.has_hooks(&HookEvent::Stop));
    }

    #[tokio::test]
    async fn circuit_breaker_skips_tripped_hook() {
        use crate::circuit_breaker::CircuitBreakerConfig;
        use std::time::Duration;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let reg = HookRegistry::new().with_circuit_breaker(CircuitBreakerConfig {
            max_consecutive_failures: 2,
            cooldown: Duration::from_secs(60),
        });
        let mut reg = reg;
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![
                // Always fails
                hook_fn(|_i, _id, _c| async {
                    Err(crate::error::HookError::CallbackFailed("boom".into()))
                }),
            ])
            .with_name("fragile-hook")],
        );

        // First two calls — hook fires (and fails), recording failures
        reg.run_post_tool_use(
            "X",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "t1",
            "s",
            "/tmp",
        )
        .await;
        reg.run_post_tool_use(
            "X",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "t2",
            "s",
            "/tmp",
        )
        .await;

        // Now add a counter hook with the same name to verify it's skipped
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(move |_i, _id, _c| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(HookOutput::default())
                }
            })])
            .with_name("fragile-hook")],
        );

        // Third call — breaker should be tripped, hook should be skipped
        reg.run_post_tool_use(
            "X",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "t3",
            "s",
            "/tmp",
        )
        .await;
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "hook should have been skipped by circuit breaker"
        );
    }

    #[tokio::test]
    async fn circuit_breaker_resets_on_success() {
        use crate::circuit_breaker::CircuitBreakerConfig;
        use std::time::Duration;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let reg = HookRegistry::new().with_circuit_breaker(CircuitBreakerConfig {
            max_consecutive_failures: 3,
            cooldown: Duration::from_secs(60),
        });
        let mut reg = reg;
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(move |_i, _id, _c| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(HookOutput::default())
                }
            })])
            .with_name("good-hook")],
        );

        // Run 5 times — all should succeed, counter should be 5
        for i in 0..5 {
            reg.run_post_tool_use(
                "X",
                &serde_json::json!({}),
                &serde_json::json!(""),
                &format!("t{}", i),
                "s",
                "/tmp",
            )
            .await;
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            5,
            "all calls should have fired"
        );
    }

    #[tokio::test]
    async fn unnamed_hook_bypasses_circuit_breaker() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                move |_i, _id, _c| {
                    let c = counter_clone.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Err(crate::error::HookError::CallbackFailed("err".into()))
                    }
                },
            )])],
            // No .with_name() — unnamed hooks should never be circuit-broken
        );

        // Run 10 times — unnamed hooks should always fire regardless of errors
        for i in 0..10 {
            reg.run_post_tool_use(
                "X",
                &serde_json::json!({}),
                &serde_json::json!(""),
                &format!("t{}", i),
                "s",
                "/tmp",
            )
            .await;
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            10,
            "unnamed hooks should always fire"
        );
    }

    #[tokio::test]
    async fn eligibility_skips_ineligible_hook() {
        use crate::eligibility::HookRequirements;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(move |_i, _id, _c| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(HookOutput::default())
                }
            })])
            .with_name("needs-missing-bin")
            .with_requirements(HookRequirements {
                bins: vec!["__totally_nonexistent_binary__".to_string()],
                ..Default::default()
            })],
        );

        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "t1",
            "s",
            "/tmp",
        )
        .await;
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "hook should be skipped due to missing binary"
        );
    }

    #[tokio::test]
    async fn eligibility_passes_eligible_hook() {
        use crate::eligibility::HookRequirements;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(move |_i, _id, _c| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(HookOutput::default())
                }
            })])
            .with_name("has-sh")
            .with_requirements(HookRequirements {
                #[cfg(unix)]
                bins: vec!["sh".to_string()],
                #[cfg(not(unix))]
                bins: vec![],
                ..Default::default()
            })],
        );

        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "t1",
            "s",
            "/tmp",
        )
        .await;
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "eligible hook should fire"
        );
    }

    #[tokio::test]
    async fn eligibility_os_mismatch_skips() {
        use crate::eligibility::HookRequirements;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let mut reg = HookRegistry::new();
        reg.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(move |_i, _id, _c| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(HookOutput::default())
                }
            })])
            .with_name("wrong-os")
            .with_requirements(HookRequirements {
                os: vec!["__nonexistent_os__".to_string()],
                ..Default::default()
            })],
        );

        reg.run_post_tool_use(
            "Bash",
            &serde_json::json!({}),
            &serde_json::json!(""),
            "t1",
            "s",
            "/tmp",
        )
        .await;
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "hook should be skipped due to OS mismatch"
        );
    }

    #[test]
    fn merge_combines_registries() {
        let mut reg1 = HookRegistry::new();
        reg1.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(|_i, _id, _c| async {
                Ok(HookOutput::default())
            })])
            .with_name("hook-a")],
        );

        let mut reg2 = HookRegistry::new();
        reg2.register(
            HookEvent::PostToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(|_i, _id, _c| async {
                Ok(HookOutput::default())
            })])
            .with_name("hook-b")],
        );
        reg2.register(
            HookEvent::PreToolUse,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                |_i, _id, _c| async { Ok(HookOutput::default()) },
            )])],
        );

        reg1.merge(reg2);

        // PostToolUse should now have 2 matchers (one from each registry)
        let matchers = reg1.get(&HookEvent::PostToolUse).unwrap();
        assert_eq!(matchers.len(), 2);
        assert_eq!(matchers[0].name.as_deref(), Some("hook-a"));
        assert_eq!(matchers[1].name.as_deref(), Some("hook-b"));

        // PreToolUse should have been added from reg2
        assert!(reg1.has_hooks(&HookEvent::PreToolUse));
    }

    #[test]
    fn merge_into_empty_registry() {
        let mut reg1 = HookRegistry::new();
        let mut reg2 = HookRegistry::new();
        reg2.register(
            HookEvent::Stop,
            vec![HookCallbackMatcher::new(vec![hook_fn(
                |_i, _id, _c| async { Ok(HookOutput::default()) },
            )])],
        );

        reg1.merge(reg2);
        assert!(reg1.has_hooks(&HookEvent::Stop));
    }

    #[test]
    fn with_circuit_breaker_builder() {
        use crate::circuit_breaker::CircuitBreakerConfig;
        use std::time::Duration;

        let reg = HookRegistry::new().with_circuit_breaker(CircuitBreakerConfig {
            max_consecutive_failures: 10,
            cooldown: Duration::from_secs(120),
        });

        // Just verify it doesn't panic and creates a valid registry
        assert!(!reg.has_hooks(&HookEvent::PostToolUse));
    }
}
