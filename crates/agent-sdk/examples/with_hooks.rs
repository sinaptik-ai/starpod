//! Example: using hooks to intercept and control agent behavior.
//!
//! ```bash
//! cargo run --example with_hooks
//! ```

use agent_sdk::{
    hook_fn, query, HookCallbackMatcher, HookEvent, HookInput, HookOutput, Message, Options,
};
use starpod_hooks::{HookSpecificOutput, PermissionDecision, SyncHookOutput};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Create a hook that blocks writes to .env files
    let protect_env = hook_fn(|input, _tool_use_id, _cancel| async move {
        if let HookInput::PreToolUse { tool_input, .. } = &input {
            if let Some(file_path) = tool_input.get("file_path").and_then(|v| v.as_str()) {
                if file_path.ends_with(".env") {
                    return Ok(HookOutput::Sync(SyncHookOutput {
                        hook_specific_output: Some(HookSpecificOutput::PreToolUse {
                            permission_decision: Some(PermissionDecision::Deny),
                            permission_decision_reason: Some(
                                "Cannot modify .env files".to_string(),
                            ),
                            updated_input: None,
                            additional_context: None,
                        }),
                        ..Default::default()
                    }));
                }
            }
        }
        Ok(HookOutput::default())
    });

    // Create a logging hook for all tool calls
    let log_tools = hook_fn(|input, _tool_use_id, _cancel| async move {
        if let Some(tool_name) = input.tool_name() {
            println!("[hook] Tool called: {}", tool_name);
        }
        Ok(HookOutput::default())
    });

    let mut stream = query(
        "Update the database configuration",
        Options::builder()
            .allowed_tools(vec![
                "Read".into(),
                "Edit".into(),
                "Write".into(),
                "Glob".into(),
            ])
            .hook(
                HookEvent::PreToolUse,
                vec![
                    HookCallbackMatcher::new(vec![protect_env]).with_matcher("Write|Edit"),
                    HookCallbackMatcher::new(vec![log_tools]),
                ],
            )
            .build(),
    );

    while let Some(message) = stream.next().await {
        let message = message?;
        if let Message::Result(result) = &message {
            if let Some(ref text) = result.result {
                println!("[result] {}", text);
            }
        }
    }

    Ok(())
}
