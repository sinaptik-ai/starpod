//! Permission evaluation engine.
//!
//! When Claude requests a tool, permissions are evaluated in this order:
//! 1. Hooks (PreToolUse) - can allow, deny, or pass through
//! 2. Deny rules (disallowed_tools) - always block if matched
//! 3. Permission mode - bypassPermissions approves all, acceptEdits approves file ops
//! 4. Allow rules (allowed_tools) - auto-approve if matched
//! 5. canUseTool callback - final decision for unresolved tools

use regex::Regex;
use tracing::{debug, warn};

use crate::error::Result;
use crate::hooks::{HookCallbackMatcher, HookEvent, HookInput, HookOutput};
use crate::hooks::input::BaseHookInput;
use crate::hooks::output::{
    HookSpecificOutput, PermissionDecision,
};
use crate::options::{Options, PermissionMode};

/// Result of permission evaluation.
#[derive(Debug, Clone)]
pub enum PermissionVerdict {
    /// Tool is allowed to execute.
    Allow,
    /// Tool is allowed with modified input.
    AllowWithUpdatedInput(serde_json::Value),
    /// Tool is denied.
    Deny { reason: String },
}

/// The permission evaluator checks whether a tool call should proceed.
pub struct PermissionEvaluator<'a> {
    options: &'a Options,
}

impl<'a> PermissionEvaluator<'a> {
    pub fn new(options: &'a Options) -> Self {
        Self { options }
    }

    /// Evaluate whether a tool call is permitted.
    ///
    /// Follows the evaluation order:
    /// 1. Run PreToolUse hooks
    /// 2. Check deny rules (disallowed_tools)
    /// 3. Apply permission mode
    /// 4. Check allow rules (allowed_tools)
    /// 5. Fall through to canUseTool or deny
    pub async fn evaluate(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_use_id: &str,
        session_id: &str,
        cwd: &str,
    ) -> Result<PermissionVerdict> {
        // Step 1: Run PreToolUse hooks
        if let Some(matchers) = self.options.hooks.get(&HookEvent::PreToolUse) {
            let verdict = self
                .run_pre_tool_use_hooks(matchers, tool_name, tool_input, tool_use_id, session_id, cwd)
                .await?;
            if let Some(v) = verdict {
                return Ok(v);
            }
        }

        // Step 2: Check deny rules
        if self.is_disallowed(tool_name) {
            debug!(tool = tool_name, "Tool denied by disallowed_tools rule");
            return Ok(PermissionVerdict::Deny {
                reason: format!("Tool '{}' is in disallowed_tools", tool_name),
            });
        }

        // Step 3: Apply permission mode
        match self.options.permission_mode {
            PermissionMode::BypassPermissions => {
                warn!(tool = tool_name, "Tool approved by bypassPermissions mode — all permission checks bypassed");
                return Ok(PermissionVerdict::Allow);
            }
            PermissionMode::AcceptEdits => {
                if is_file_operation(tool_name) {
                    debug!(tool = tool_name, "Tool approved by acceptEdits mode");
                    return Ok(PermissionVerdict::Allow);
                }
                // Fall through for non-file-operation tools
            }
            PermissionMode::Plan => {
                return Ok(PermissionVerdict::Deny {
                    reason: "Plan mode - no tool execution allowed".to_string(),
                });
            }
            PermissionMode::DontAsk => {
                // Only pre-approved tools pass; everything else is denied
                if self.is_allowed(tool_name) {
                    debug!(tool = tool_name, "Tool approved by allowed_tools in dontAsk mode");
                    return Ok(PermissionVerdict::Allow);
                }
                return Ok(PermissionVerdict::Deny {
                    reason: format!(
                        "Tool '{}' not pre-approved and permissionMode is dontAsk",
                        tool_name
                    ),
                });
            }
            PermissionMode::Default => {
                // Fall through to allow rules and canUseTool
            }
        }

        // Step 4: Check allow rules
        if self.is_allowed(tool_name) {
            debug!(tool = tool_name, "Tool approved by allowed_tools rule");
            return Ok(PermissionVerdict::Allow);
        }

        // Step 5: canUseTool callback
        if let Some(ref _can_use_tool) = self.options.can_use_tool {
            // TODO: Call canUseTool callback
            // For now, deny if no callback handles it
        }

        // Default: deny
        Ok(PermissionVerdict::Deny {
            reason: format!(
                "Tool '{}' not approved by any permission rule",
                tool_name
            ),
        })
    }

    /// Check if a tool matches the disallowed_tools list.
    fn is_disallowed(&self, tool_name: &str) -> bool {
        self.options
            .disallowed_tools
            .iter()
            .any(|pattern| tool_matches(tool_name, pattern))
    }

    /// Check if a tool matches the allowed_tools list.
    fn is_allowed(&self, tool_name: &str) -> bool {
        self.options
            .allowed_tools
            .iter()
            .any(|pattern| tool_matches(tool_name, pattern))
    }

    /// Run PreToolUse hooks and return a verdict if any hook makes a decision.
    async fn run_pre_tool_use_hooks(
        &self,
        matchers: &[HookCallbackMatcher],
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_use_id: &str,
        session_id: &str,
        cwd: &str,
    ) -> Result<Option<PermissionVerdict>> {
        for matcher in matchers {
            if !matcher.matches(tool_name)? {
                continue;
            }

            let input = HookInput::PreToolUse {
                base: BaseHookInput {
                    session_id: session_id.to_string(),
                    transcript_path: String::new(),
                    cwd: cwd.to_string(),
                    permission_mode: Some(self.options.permission_mode.to_string()),
                    agent_id: None,
                    agent_type: None,
                },
                tool_name: tool_name.to_string(),
                tool_input: tool_input.clone(),
                tool_use_id: tool_use_id.to_string(),
            };

            let cancel = tokio_util::sync::CancellationToken::new();

            for hook in &matcher.hooks {
                let output = hook(
                    input.clone(),
                    Some(tool_use_id.to_string()),
                    cancel.clone(),
                )
                .await?;

                match output {
                    HookOutput::Sync(sync_output) => {
                        if let Some(ref specific) = sync_output.hook_specific_output {
                            match specific {
                                HookSpecificOutput::PreToolUse {
                                    permission_decision: Some(PermissionDecision::Deny),
                                    permission_decision_reason,
                                    ..
                                } => {
                                    let reason = permission_decision_reason
                                        .clone()
                                        .unwrap_or_else(|| "Denied by hook".to_string());
                                    debug!(tool = tool_name, reason = %reason, "Tool denied by PreToolUse hook");
                                    return Ok(Some(PermissionVerdict::Deny { reason }));
                                }
                                HookSpecificOutput::PreToolUse {
                                    permission_decision: Some(PermissionDecision::Allow),
                                    updated_input,
                                    ..
                                } => {
                                    debug!(tool = tool_name, "Tool approved by PreToolUse hook");
                                    if let Some(new_input) = updated_input {
                                        return Ok(Some(PermissionVerdict::AllowWithUpdatedInput(
                                            serde_json::to_value(new_input)
                                                .unwrap_or_default(),
                                        )));
                                    }
                                    return Ok(Some(PermissionVerdict::Allow));
                                }
                                _ => {}
                            }
                        }
                    }
                    HookOutput::Async(_) => {
                        // Async hooks don't affect permission decisions
                    }
                }
            }
        }

        Ok(None)
    }
}

/// Check if a tool name matches a permission pattern.
///
/// Patterns can be:
/// - Exact match: "Read"
/// - Wildcard: "mcp__github__*"
/// - With scope: "Bash(npm:*)"
fn tool_matches(tool_name: &str, pattern: &str) -> bool {
    if pattern == tool_name {
        return true;
    }

    // Handle scoped patterns like "Bash(npm:*)" — check before wildcard
    // matching so that the parenthesized scope isn't treated as a regex pattern.
    if let Some(paren_pos) = pattern.find('(') {
        let base_tool = &pattern[..paren_pos];
        if tool_name == base_tool {
            // Tool name matches the base; scope checking would happen
            // at the input level, not here. Return true for the name match.
            return true;
        }
    }

    // Handle wildcard patterns
    if pattern.contains('*') {
        let regex_pattern = pattern
            .replace('.', "\\.")
            .replace('*', ".*");
        if let Ok(re) = Regex::new(&format!("^{}$", regex_pattern)) {
            return re.is_match(tool_name);
        }
    }

    false
}

/// Check if a tool is a file operation (for acceptEdits mode).
fn is_file_operation(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Edit" | "Write" | "NotebookEdit"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_matches_exact() {
        assert!(tool_matches("Read", "Read"));
        assert!(!tool_matches("Read", "Write"));
    }

    #[test]
    fn test_tool_matches_wildcard() {
        assert!(tool_matches("mcp__github__list_issues", "mcp__github__*"));
        assert!(tool_matches("mcp__github__search", "mcp__*"));
        assert!(!tool_matches("Read", "mcp__*"));
    }

    #[test]
    fn test_tool_matches_scoped() {
        assert!(tool_matches("Bash", "Bash(npm:*)"));
        assert!(!tool_matches("Read", "Bash(npm:*)"));
    }

    #[test]
    fn test_is_file_operation() {
        assert!(is_file_operation("Edit"));
        assert!(is_file_operation("Write"));
        assert!(!is_file_operation("Bash"));
        assert!(!is_file_operation("Read"));
    }
}
