//! Hook system — re-exported from the `starpod-hooks` crate.
//!
//! All hook types, callbacks, and execution logic live in `starpod-hooks`.
//! This module re-exports them for backward compatibility.

pub mod input {
    pub use starpod_hooks::input::*;
}

pub mod output {
    pub use starpod_hooks::output::*;
}

pub use starpod_hooks::{
    hook_fn, HookCallback, HookCallbackMatcher, HookDiscovery, HookEvent, HookInput, HookOutput,
    HookRegistry, HookRequirements,
};
