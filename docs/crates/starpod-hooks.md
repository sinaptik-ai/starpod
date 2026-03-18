# starpod-hooks

Lifecycle hook system for the Starpod platform. Provides hook events, typed input/output, callback registration, and an execution engine with timeout, cancellation, circuit breaking, eligibility checks, and file-based discovery.

## Architecture

Hooks are lifecycle callbacks that fire at specific points during agent execution. They can observe events (fire-and-forget) or control behavior (blocking with decisions).

- **`HookEvent`** -- enum of 18 lifecycle events
- **`HookInput`** -- typed payload for each event (session ID, tool name, tool input/output)
- **`HookOutput`** -- return value: async (fire-and-forget) or sync (with decisions like approve/block)
- **`HookCallback`** -- async function signature for hook implementations
- **`HookCallbackMatcher`** -- groups callbacks with an optional regex filter, identity (`name`), timeout, and eligibility requirements
- **`HookRegistry`** -- manages hooks by event type, runs them with circuit breaker and eligibility cache

## Quick Start

```rust
use starpod_hooks::{HookRegistry, HookEvent, HookCallbackMatcher, hook_fn, HookOutput};

let mut registry = HookRegistry::new();

// Register a hook that fires after any Bash tool use
registry.register(HookEvent::PostToolUse, vec![
    HookCallbackMatcher::new(vec![
        hook_fn(|input, _id, _cancel| async move {
            println!("Tool used: {}", input.tool_name().unwrap_or("unknown"));
            Ok(HookOutput::default())
        }),
    ])
    .with_name("bash-logger")
    .with_matcher("Bash")
    .with_timeout(30),
]);
```

## HookEvent (18 variants)

| Event | When it fires | Can block? |
|-------|--------------|------------|
| `PreToolUse` | Before tool execution | Yes -- can modify input or deny |
| `PostToolUse` | After successful tool execution | No |
| `PostToolUseFailure` | After failed tool execution | No |
| `UserPromptSubmit` | When user sends a message | Yes |
| `SessionStart` | Session begins | No |
| `SessionEnd` | Session ends | No |
| `Stop` | Agent stopping | No |
| `Notification` | System notification | No |
| `SubagentStart` | Subagent launching | No |
| `SubagentStop` | Subagent finished | No |
| `PreCompact` | Before conversation compaction | No |
| `PermissionRequest` | Permission decision needed | Yes |
| `Setup` | Initial/maintenance setup | No |
| `TeammateIdle` | Teammate idle | No |
| `TaskCompleted` | Task finished | No |
| `ConfigChange` | Configuration changed | No |
| `WorktreeCreate` | Git worktree created | No |
| `WorktreeRemove` | Git worktree removed | No |

## HookInput

Typed payload for each event, carrying context. All variants include a `BaseHookInput`:

```rust
pub struct BaseHookInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    pub permission_mode: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
}
```

`HookInput` is a per-event enum with variants like `PreToolUse { base, tool_name, tool_input }`, `PostToolUse { base, tool_name, tool_input, tool_output }`, `UserPromptSubmit { base, prompt }`, etc.

## HookOutput

```rust
pub enum HookOutput {
    Async(AsyncHookOutput),   // Fire-and-forget
    Sync(SyncHookOutput),     // Blocking with decisions
}
```

### SyncHookOutput

```rust
pub struct SyncHookOutput {
    pub should_continue: Option<bool>,       // "continue" in JSON
    pub suppress_output: Option<bool>,
    pub stop_reason: Option<String>,
    pub decision: Option<HookDecision>,      // Approve or Block
    pub system_message: Option<String>,      // Inject into conversation
    pub reason: Option<String>,
    pub hook_specific_output: Option<HookSpecificOutput>,
}

pub enum HookDecision {
    Approve,
    Block,
}
```

### AsyncHookOutput

```rust
pub struct AsyncHookOutput {
    pub is_async: bool,                      // "async" in JSON; must be true
    pub async_timeout: Option<u64>,          // Timeout in milliseconds
}
```

## HookCallback

```rust
pub type HookCallback = Arc<
    dyn Fn(HookInput, Option<String>, CancellationToken)
        -> Pin<Box<dyn Future<Output = Result<HookOutput>> + Send>>
        + Send + Sync,
>;

// Helper to create a callback from an async closure
pub fn hook_fn<F, Fut>(f: F) -> HookCallback;
```

## HookCallbackMatcher

Groups callbacks with optional regex filtering and metadata:

```rust
pub struct HookCallbackMatcher {
    pub name: Option<String>,              // Identity for circuit breaker / logging
    pub matcher: Option<String>,           // Regex pattern (e.g. "Bash|Write")
    pub hooks: Vec<HookCallback>,
    pub timeout: Option<u64>,              // Timeout in seconds
    pub requires: Option<HookRequirements>,
}
```

Builder methods: `.with_name()`, `.with_matcher()`, `.with_timeout()`, `.with_requirements()`.

If no matcher is set, the hook runs for every event of its type. The regex is tested against the tool name for tool-related hooks.

## HookRegistry

```rust
let mut registry = HookRegistry::new();

// Register matchers for an event
registry.register(HookEvent::PreToolUse, vec![matcher1, matcher2]);

// Run hooks
registry.run_pre_tool_use("Bash", input).await;   // Returns Option<HookOutput>
registry.run_post_tool_use("Bash", input).await;   // Fire-and-forget
registry.run_event(&HookEvent::SessionStart, input).await; // Generic
```

The registry includes an integrated circuit breaker per named hook. After 5 consecutive failures (configurable), the hook is tripped and skipped for a 60-second cooldown. After cooldown, one retry is allowed; success resets the breaker.

## Circuit Breaker

```rust
pub struct CircuitBreakerConfig {
    pub max_consecutive_failures: u32,  // default: 5
    pub cooldown_secs: u64,             // default: 60
}

pub enum BreakerStatus {
    Closed,    // Healthy, allow calls
    Open,      // Tripped, skipping calls
    HalfOpen,  // Cooldown expired, allowing one retry
}
```

## Eligibility Requirements

Hooks can declare requirements that must be met for them to run:

```rust
pub struct HookRequirements {
    pub bins: Vec<String>,   // Binaries that must be on PATH
    pub envs: Vec<String>,   // Environment variables that must be set
    pub os: Vec<String>,     // Allowed operating systems (e.g. "macos", "linux")
}
```

Results are cached per named hook to avoid repeated `which` syscalls.

## File-Based Discovery

`HookDiscovery` scans directories for `<hook-name>/HOOK.md` files with TOML frontmatter. Each manifest declares the hook's event, matcher, timeout, requirements, and a shell command. The command receives `HookInput` as JSON on stdin and returns `HookOutput` as JSON on stdout.

```rust
pub struct HookManifest {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub timeout: Option<u64>,
    pub command: String,
    pub requires: Option<HookRequirements>,
}
```

## Permission Types

Shared types used by hooks and the broader permission system:

```rust
pub struct PermissionUpdate {
    pub tool: String,
    pub permission: PermissionLevel,
}

pub enum PermissionLevel { Allow, Deny, Ask }
pub enum PermissionDecision { Allow, Deny, Ask }
```

## Public Re-exports

The crate re-exports its main API at the top level:

- `HookEvent`, `HookInput`, `BaseHookInput`, `HookOutput`, `SyncHookOutput`, `AsyncHookOutput`
- `HookCallback`, `HookCallbackMatcher`, `hook_fn`
- `HookRegistry`
- `HookDecision`, `HookSpecificOutput`, `PermissionRequestDecision`
- `PermissionUpdate`, `PermissionLevel`, `PermissionDecision`
- `CircuitBreaker`, `CircuitBreakerConfig`, `BreakerStatus`
- `HookDiscovery`, `HookManifest`
- `HookRequirements`, `EligibilityError`
- `HookError`

## Tests

41 tests + 8 doc-tests covering event display/equality/hashing, callback creation, matcher regex filtering, circuit breaker state transitions, eligibility checks, registry execution, permission type serde, and file-based discovery.
