//! Built-in tool definitions and execution.
//!
//! The SDK includes the same tools that power Claude Code:
//! - File operations: Read, Edit, Write
//! - Search: Glob, Grep
//! - Execution: Bash
//! - Web: WebSearch, WebFetch
//! - Discovery: ToolSearch
//! - Orchestration: Agent, Skill, AskUserQuestion, TodoWrite

pub mod definitions;
pub mod executor;

pub use definitions::{get_tool_definitions, ToolDef};
pub use executor::{ToolExecutor, ToolResult};

/// All built-in tool names.
pub const BUILT_IN_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Glob",
    "Grep",
    "Agent",
    "AskUserQuestion",
    "TodoWrite",
    "NotebookEdit",
    "ToolSearch",
    "Skill",
    "Config",
    "EnterWorktree",
    "ExitPlanMode",
    "TaskOutput",
    "TaskStop",
    "ListMcpResources",
    "ReadMcpResource",
];

/// Tool categories for permission grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCategory {
    /// Read-only file operations (Read, Glob, Grep).
    ReadOnly,
    /// File modification (Edit, Write).
    FileModification,
    /// Command execution (Bash).
    Execution,
    /// Web access (WebSearch, WebFetch).
    Web,
    /// Agent orchestration (Agent, Skill).
    Orchestration,
    /// User interaction (AskUserQuestion).
    UserInteraction,
    /// Other tools.
    Other,
}

/// Get the category for a built-in tool.
pub fn tool_category(tool_name: &str) -> ToolCategory {
    match tool_name {
        "Read" | "Glob" | "Grep" | "ToolSearch" => ToolCategory::ReadOnly,
        "Write" | "Edit" | "NotebookEdit" => ToolCategory::FileModification,
        "Bash" | "TaskOutput" | "TaskStop" => ToolCategory::Execution,
        "WebSearch" | "WebFetch" => ToolCategory::Web, // custom tools, category still useful
        "Agent" | "Skill" => ToolCategory::Orchestration,
        "AskUserQuestion" => ToolCategory::UserInteraction,
        _ => ToolCategory::Other,
    }
}

/// Check if a tool is read-only (safe for parallel execution).
pub fn is_read_only(tool_name: &str) -> bool {
    matches!(
        tool_category(tool_name),
        ToolCategory::ReadOnly | ToolCategory::Web
    )
}

/// Common tool combinations for different use cases.
pub mod presets {
    /// Read-only analysis tools.
    pub fn read_only() -> Vec<String> {
        vec![
            "Read".into(),
            "Glob".into(),
            "Grep".into(),
        ]
    }

    /// Test execution tools.
    pub fn test_execution() -> Vec<String> {
        vec![
            "Bash".into(),
            "Read".into(),
            "Grep".into(),
        ]
    }

    /// Code modification tools (no command execution).
    pub fn code_modification() -> Vec<String> {
        vec![
            "Read".into(),
            "Edit".into(),
            "Write".into(),
            "Grep".into(),
            "Glob".into(),
        ]
    }

    /// Full access - all common tools.
    pub fn full_access() -> Vec<String> {
        vec![
            "Read".into(),
            "Edit".into(),
            "Write".into(),
            "Bash".into(),
            "Glob".into(),
            "Grep".into(),
            "Agent".into(),
        ]
    }
}
