//! Custom tool definitions and handler for Orion-specific tools.

use std::sync::Arc;

use agent_sdk::{CustomToolDefinition, ToolResult};
use serde_json::json;
use tracing::debug;

use orion_memory::MemoryStore;
use orion_vault::Vault;

/// Build the JSON schema definitions for Orion's custom tools.
pub fn custom_tool_definitions() -> Vec<CustomToolDefinition> {
    vec![
        CustomToolDefinition {
            name: "MemorySearch".into(),
            description: "Search the user's memory (long-term knowledge, daily logs, notes) using full-text search.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 5)"
                    }
                },
                "required": ["query"]
            }),
        },
        CustomToolDefinition {
            name: "MemoryWrite".into(),
            description: "Write or update a file in the user's memory store (e.g. USER.md, MEMORY.md, knowledge/*.md).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file": {
                        "type": "string",
                        "description": "Relative file path within the memory store (e.g. 'USER.md', 'knowledge/rust.md')"
                    },
                    "content": {
                        "type": "string",
                        "description": "The full content to write to the file"
                    }
                },
                "required": ["file", "content"]
            }),
        },
        CustomToolDefinition {
            name: "MemoryAppendDaily".into(),
            description: "Append a timestamped entry to today's daily log.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The text to append to today's daily log"
                    }
                },
                "required": ["text"]
            }),
        },
        CustomToolDefinition {
            name: "VaultGet".into(),
            description: "Retrieve an encrypted credential from the vault by key.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "The key to look up"
                    }
                },
                "required": ["key"]
            }),
        },
        CustomToolDefinition {
            name: "VaultSet".into(),
            description: "Store an encrypted credential in the vault.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "The key to store under"
                    },
                    "value": {
                        "type": "string",
                        "description": "The secret value to encrypt and store"
                    }
                },
                "required": ["key", "value"]
            }),
        },
    ]
}

/// Handle a custom tool call. Returns `Some(ToolResult)` if handled, `None` if not a custom tool.
pub async fn handle_custom_tool(
    memory: &Arc<MemoryStore>,
    vault: &Arc<Vault>,
    tool_name: &str,
    input: &serde_json::Value,
) -> Option<ToolResult> {
    match tool_name {
        "MemorySearch" => {
            let query = input.get("query")?.as_str()?;
            let limit = input
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(5) as usize;

            debug!(query = %query, limit = limit, "MemorySearch");

            match memory.search(query, limit) {
                Ok(results) => {
                    let formatted: Vec<serde_json::Value> = results
                        .iter()
                        .map(|r| {
                            json!({
                                "source": r.source,
                                "text": r.text,
                                "lines": format!("{}-{}", r.line_start, r.line_end),
                            })
                        })
                        .collect();

                    Some(ToolResult {
                        content: serde_json::to_string_pretty(&formatted).unwrap_or_default(),
                        is_error: false,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Memory search error: {}", e),
                    is_error: true,
                }),
            }
        }

        "MemoryWrite" => {
            let file = input.get("file")?.as_str()?;
            let content = input.get("content")?.as_str()?;

            debug!(file = %file, "MemoryWrite");

            match memory.write_file(file, content) {
                Ok(()) => Some(ToolResult {
                    content: format!("Successfully wrote {}", file),
                    is_error: false,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Memory write error: {}", e),
                    is_error: true,
                }),
            }
        }

        "MemoryAppendDaily" => {
            let text = input.get("text")?.as_str()?;

            debug!("MemoryAppendDaily");

            match memory.append_daily(text) {
                Ok(()) => Some(ToolResult {
                    content: "Appended to daily log.".into(),
                    is_error: false,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Daily append error: {}", e),
                    is_error: true,
                }),
            }
        }

        "VaultGet" => {
            let key = input.get("key")?.as_str()?;

            debug!(key = %key, "VaultGet");

            match vault.get(key) {
                Ok(Some(value)) => Some(ToolResult {
                    content: value,
                    is_error: false,
                }),
                Ok(None) => Some(ToolResult {
                    content: format!("No vault entry found for key: {}", key),
                    is_error: false,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Vault get error: {}", e),
                    is_error: true,
                }),
            }
        }

        "VaultSet" => {
            let key = input.get("key")?.as_str()?;
            let value = input.get("value")?.as_str()?;

            debug!(key = %key, "VaultSet");

            match vault.set(key, value) {
                Ok(()) => Some(ToolResult {
                    content: format!("Stored '{}' in vault.", key),
                    is_error: false,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Vault set error: {}", e),
                    is_error: true,
                }),
            }
        }

        _ => None,
    }
}
