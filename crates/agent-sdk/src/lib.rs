//! # Agent SDK - Claude Agent SDK for Rust
//!
//! Build production AI agents with Claude. This is a Rust port of the
//! [Claude Agent SDK](https://platform.claude.com/docs/en/agent-sdk/overview).
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use agent_sdk::{query, Options, Message};
//! use tokio_stream::StreamExt;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut stream = query(
//!         "What files are in this directory?",
//!         Options::builder()
//!             .allowed_tools(vec!["Bash".into(), "Glob".into()])
//!             .build(),
//!     );
//!
//!     while let Some(message) = stream.next().await {
//!         let message = message?;
//!         if let Message::Result(result) = &message {
//!             println!("{}", result.result.as_deref().unwrap_or(""));
//!         }
//!     }
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod error;
pub mod hooks;
pub mod mcp;
pub mod options;
pub mod permissions;
pub mod query;
pub mod session;
pub mod tools;
pub mod types;

// Re-export main public API
pub use error::AgentError;
pub use hooks::{hook_fn, HookCallback, HookCallbackMatcher, HookEvent, HookInput, HookOutput};
pub use mcp::{McpServerConfig, McpStdioServerConfig, McpHttpServerConfig, McpSseServerConfig};
pub use options::{CustomToolDefinition, ExternalToolHandlerFn, Options, OptionsBuilder, PermissionMode};
pub use query::query;
pub use session::{Session, SessionInfo};
pub use types::agent::{AgentDefinition, AgentInput};
pub use types::messages::*;
pub use tools::executor::ToolResult;
pub use types::tools::*;
