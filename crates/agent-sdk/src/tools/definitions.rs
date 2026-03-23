//! Tool definitions (JSON schemas) for the Anthropic API.
//!
//! These definitions are sent to Claude so it knows what tools are available
//! and how to call them.

use serde_json::{json, Value};

/// A tool definition for the API.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// Get definitions for the specified tool names.
pub fn get_tool_definitions(tool_names: &[String]) -> Vec<ToolDef> {
    tool_names
        .iter()
        .filter_map(|name| get_tool_definition(name))
        .collect()
}

/// Get the definition for a single built-in tool.
pub fn get_tool_definition(name: &str) -> Option<ToolDef> {
    match name {
        "Read" => Some(ToolDef {
            name: "Read",
            description: "Read a file from the filesystem. Returns the file content with line numbers. Image files (png, jpg, gif, webp) are returned as visual content so you can see and analyze them directly.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to read"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start reading from (1-based)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read"
                    }
                },
                "required": ["file_path"]
            }),
        }),
        "Write" => Some(ToolDef {
            name: "Write",
            description: "Write content to a file, creating it if it doesn't exist or overwriting if it does.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the file"
                    }
                },
                "required": ["file_path", "content"]
            }),
        }),
        "Edit" => Some(ToolDef {
            name: "Edit",
            description: "Perform exact string replacement in a file. The old_string must be unique in the file unless replace_all is true.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The absolute path to the file to modify"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The text to find and replace"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text"
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace all occurrences (default: false)"
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
        }),
        "Bash" => Some(ToolDef {
            name: "Bash",
            description: "Execute a bash command and return its output. Use run_in_background for long-running processes like servers.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in milliseconds (default: 120000)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Description of what the command does"
                    },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "Run the command in the background. Use this for long-running processes like servers. Returns immediately with the PID."
                    }
                },
                "required": ["command"]
            }),
        }),
        "Glob" => Some(ToolDef {
            name: "Glob",
            description: "Find files matching a glob pattern.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The glob pattern to match (e.g., '**/*.rs')"
                    },
                    "path": {
                        "type": "string",
                        "description": "The directory to search in"
                    }
                },
                "required": ["pattern"]
            }),
        }),
        "Grep" => Some(ToolDef {
            name: "Grep",
            description: "Search file contents using regular expressions.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The regex pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search in"
                    },
                    "glob": {
                        "type": "string",
                        "description": "Glob pattern to filter files"
                    },
                    "type": {
                        "type": "string",
                        "description": "File type to search (e.g., 'rs', 'py')"
                    },
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files_with_matches", "count"],
                        "description": "Output mode (default: files_with_matches)"
                    },
                    "-i": {
                        "type": "boolean",
                        "description": "Case insensitive search"
                    },
                    "-n": {
                        "type": "boolean",
                        "description": "Show line numbers"
                    },
                    "-B": {
                        "type": "integer",
                        "description": "Lines to show before each match"
                    },
                    "-A": {
                        "type": "integer",
                        "description": "Lines to show after each match"
                    },
                    "-C": {
                        "type": "integer",
                        "description": "Lines of context around each match"
                    },
                    "head_limit": {
                        "type": "integer",
                        "description": "Limit output to first N entries"
                    }
                },
                "required": ["pattern"]
            }),
        }),
        "Agent" => Some(ToolDef {
            name: "Agent",
            description: "Launch a subagent to handle a complex, multi-step task autonomously.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "Short description of the task"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The task for the agent to perform"
                    },
                    "subagent_type": {
                        "type": "string",
                        "description": "The type of agent to use"
                    },
                    "model": {
                        "type": "string",
                        "enum": ["sonnet", "opus", "haiku"],
                        "description": "Model override for this agent"
                    }
                },
                "required": ["description", "prompt", "subagent_type"]
            }),
        }),
        "AskUserQuestion" => Some(ToolDef {
            name: "AskUserQuestion",
            description: "Ask the user a clarifying question with optional multiple choice options.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": { "type": "string" },
                                "header": { "type": "string" },
                                "options": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": { "type": "string" },
                                            "description": { "type": "string" }
                                        },
                                        "required": ["label", "description"]
                                    }
                                },
                                "multiSelect": { "type": "boolean" }
                            },
                            "required": ["question", "header", "options", "multiSelect"]
                        }
                    }
                },
                "required": ["questions"]
            }),
        }),
        "TodoWrite" => Some(ToolDef {
            name: "TodoWrite",
            description: "Create and manage a structured task list for tracking progress.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"]
                                },
                                "activeForm": { "type": "string" }
                            },
                            "required": ["content", "status", "activeForm"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        }),
        _ => None,
    }
}
