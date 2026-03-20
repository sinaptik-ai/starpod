//! Custom tool definitions and handler for Starpod-specific tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_sdk::{CustomToolDefinition, ToolResult};
use serde_json::json;
use tracing::debug;

use starpod_cron::store::epoch_to_rfc3339;
use starpod_cron::{CronStore, RunStatus};
use starpod_memory::{MemoryStore, UserMemoryView};
use starpod_skills::SkillStore;

/// Shared context for custom tool handlers.
///
/// When `user_view` is `Some`, memory tools (MemorySearch, MemoryWrite,
/// MemoryAppendDaily) route per-user files (USER.md, MEMORY.md, memory/*)
/// to the user's directory while agent-level files (SOUL.md, etc.) go to
/// the shared store. When `None`, all writes go to the agent-level store.
pub struct ToolContext {
    pub memory: Arc<MemoryStore>,
    pub user_view: Option<UserMemoryView>,
    pub skills: Arc<SkillStore>,
    pub cron: Arc<CronStore>,
    pub user_tz: Option<String>,
    pub home_dir: PathBuf,
    pub user_id: Option<String>,
}

/// Build the JSON schema definitions for all Starpod custom tools.
pub fn custom_tool_definitions() -> Vec<CustomToolDefinition> {
    vec![
        // --- Memory tools ---
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
        // --- Env tool (replaces vault) ---
        CustomToolDefinition {
            name: "EnvGet".into(),
            description: "Look up an environment variable by key.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "The environment variable name to look up"
                    }
                },
                "required": ["key"]
            }),
        },
        // --- File tools ---
        CustomToolDefinition {
            name: "FileRead".into(),
            description: "Read a file from the agent's filesystem sandbox. Path is relative to the home directory.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path within the agent's filesystem"
                    }
                },
                "required": ["path"]
            }),
        },
        CustomToolDefinition {
            name: "FileWrite".into(),
            description: "Write a file to the agent's filesystem sandbox. Path is relative to the home directory. Creates parent directories as needed.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path within the agent's filesystem"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        CustomToolDefinition {
            name: "FileList".into(),
            description: "List files and directories in the agent's filesystem sandbox. Path is relative to the home directory.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative directory path (default: root of sandbox)"
                    }
                }
            }),
        },
        CustomToolDefinition {
            name: "FileDelete".into(),
            description: "Delete a file from the agent's filesystem sandbox. Path is relative to the home directory.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path to delete"
                    }
                },
                "required": ["path"]
            }),
        },
        // --- Skill tools ---
        CustomToolDefinition {
            name: "SkillActivate".into(),
            description: "Activate a skill to load its full instructions into context. Use this when a task matches a skill's description from the skill catalog. Returns the skill's complete instructions and any bundled resources.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to activate (from the available_skills catalog)"
                    }
                },
                "required": ["name"]
            }),
        },
        CustomToolDefinition {
            name: "SkillCreate".into(),
            description: "Create a new AgentSkills-compatible skill. Skills are SKILL.md files with YAML frontmatter (name, description) and a markdown body containing instructions.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Skill name (lowercase letters, digits, hyphens only, e.g. 'summarize-pr')"
                    },
                    "description": {
                        "type": "string",
                        "description": "What the skill does and when to use it (used for skill discovery)"
                    },
                    "body": {
                        "type": "string",
                        "description": "Markdown instructions for the skill (the body after frontmatter)"
                    }
                },
                "required": ["name", "description", "body"]
            }),
        },
        CustomToolDefinition {
            name: "SkillUpdate".into(),
            description: "Update an existing skill's description and/or instructions.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to update"
                    },
                    "description": {
                        "type": "string",
                        "description": "New description for the skill"
                    },
                    "body": {
                        "type": "string",
                        "description": "New markdown instructions for the skill"
                    }
                },
                "required": ["name", "description", "body"]
            }),
        },
        CustomToolDefinition {
            name: "SkillDelete".into(),
            description: "Delete a skill. This cannot be undone — confirm with the user first.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to delete"
                    }
                },
                "required": ["name"]
            }),
        },
        CustomToolDefinition {
            name: "SkillList".into(),
            description: "List all available skills with their descriptions.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Cron tools ---
        CustomToolDefinition {
            name: "CronAdd".into(),
            description: "Schedule a recurring or one-shot task. Cron expressions are evaluated in the user's configured timezone. The prompt will be sent to you as a message when the job fires.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Human-readable job name (unique)"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The message/prompt to execute when the job fires"
                    },
                    "schedule": {
                        "type": "object",
                        "description": "Schedule configuration",
                        "properties": {
                            "kind": {
                                "type": "string",
                                "enum": ["interval", "cron", "one_shot"],
                                "description": "Schedule type"
                            },
                            "every_ms": {
                                "type": "integer",
                                "description": "Interval in milliseconds (for 'interval' kind)"
                            },
                            "expr": {
                                "type": "string",
                                "description": "Cron expression with seconds field, e.g. '0 0 9 * * *' for daily at 9am (for 'cron' kind)"
                            },
                            "at": {
                                "type": "string",
                                "description": "ISO 8601 timestamp for 'one_shot' kind. Prefer RFC 3339 with offset (e.g. '2026-03-19T09:00:00+01:00'). Naive timestamps (no offset) are interpreted in the user's configured timezone."
                            }
                        },
                        "required": ["kind"]
                    },
                    "delete_after_run": {
                        "type": "boolean",
                        "description": "If true, automatically delete the job after it runs once (default: false)"
                    },
                    "max_retries": {
                        "type": "integer",
                        "description": "Maximum retry attempts on failure with exponential backoff (default: 3)"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout in seconds before a stuck run is killed (default: 7200 = 2 hours)"
                    },
                    "session_mode": {
                        "type": "string",
                        "enum": ["isolated", "main"],
                        "description": "Session mode: 'isolated' (default) runs in its own session, 'main' runs in the shared main session"
                    }
                },
                "required": ["name", "prompt", "schedule"]
            }),
        },
        CustomToolDefinition {
            name: "CronList".into(),
            description: "List all scheduled cron jobs with status, retry info, and session mode.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        CustomToolDefinition {
            name: "CronRemove".into(),
            description: "Remove a scheduled cron job by name.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the job to remove"
                    }
                },
                "required": ["name"]
            }),
        },
        CustomToolDefinition {
            name: "CronRuns".into(),
            description: "View recent execution history for a cron job.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the job"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of runs to return (default: 10)"
                    }
                },
                "required": ["name"]
            }),
        },
        CustomToolDefinition {
            name: "CronRun".into(),
            description: "Immediately execute a cron job by name (manual trigger). The job runs as if it were scheduled, with its configured session mode.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the job to run immediately"
                    }
                },
                "required": ["name"]
            }),
        },
        CustomToolDefinition {
            name: "CronUpdate".into(),
            description: "Update properties of an existing cron job by name. Can change the schedule, prompt, and other settings. When the schedule changes, next_run_at is recomputed.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the job to update"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "New prompt for the job"
                    },
                    "schedule": {
                        "type": "object",
                        "description": "New schedule (same format as CronAdd)",
                        "properties": {
                            "kind": {
                                "type": "string",
                                "enum": ["interval", "cron", "one_shot"]
                            },
                            "every_ms": { "type": "integer" },
                            "expr": { "type": "string" },
                            "at": { "type": "string", "description": "ISO 8601 timestamp with offset preferred (e.g. '2026-03-19T09:00:00+01:00')" }
                        },
                        "required": ["kind"]
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Enable or disable the job"
                    },
                    "max_retries": {
                        "type": "integer",
                        "description": "New max retries"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "New timeout in seconds"
                    },
                    "session_mode": {
                        "type": "string",
                        "enum": ["isolated", "main"],
                        "description": "New session mode"
                    }
                },
                "required": ["name"]
            }),
        },
        CustomToolDefinition {
            name: "HeartbeatWake".into(),
            description: "Wake the heartbeat system. Use 'now' to trigger an immediate heartbeat, or 'next' (default) to wait for the natural schedule.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["now", "next"],
                        "description": "Wake mode: 'now' triggers immediately, 'next' waits for schedule (default: 'next')"
                    },
                    "message": {
                        "type": "string",
                        "description": "Optional message to prepend to the heartbeat prompt"
                    }
                }
            }),
        },
    ]
}

// ── Sandbox path validation ──────────────────────────────────────────────────

/// Validate and resolve a relative path within the home directory sandbox.
///
/// Rejects paths that:
/// - Start with `.starpod` (defense-in-depth)
/// - Contain `..` traversal
/// - Are absolute paths
fn validate_sandbox_path(relative: &str, home_dir: &Path) -> std::result::Result<PathBuf, String> {
    // Reject absolute paths
    if relative.starts_with('/') || relative.starts_with('\\') {
        return Err("Absolute paths are not allowed".into());
    }

    // Reject .. traversal
    for component in std::path::Path::new(relative).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal (..) is not allowed".into());
        }
    }

    // Reject paths starting with .starpod
    let normalized = relative.replace('\\', "/");
    if normalized == ".starpod" || normalized.starts_with(".starpod/") {
        return Err("Cannot access .starpod/ directory — it contains internal state".into());
    }

    let resolved = home_dir.join(relative);

    // Double-check: canonicalize if the path exists
    if resolved.exists() {
        let canonical = resolved.canonicalize().map_err(|e| format!("Failed to resolve path: {}", e))?;
        let root_canonical = home_dir.canonicalize().map_err(|e| format!("Failed to resolve root: {}", e))?;
        if !canonical.starts_with(&root_canonical) {
            return Err("Path resolves outside the sandbox".into());
        }
    }

    Ok(resolved)
}

/// Handle a custom tool call. Returns `Some(ToolResult)` if handled, `None` if not a custom tool.
pub async fn handle_custom_tool(
    ctx: &ToolContext,
    tool_name: &str,
    input: &serde_json::Value,
) -> Option<ToolResult> {
    match tool_name {
        // --- Memory tools ---
        "MemorySearch" => {
            let query = input.get("query")?.as_str()?;
            let limit = input
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(5) as usize;

            debug!(query = %query, limit = limit, "MemorySearch");

            let search_result = if let Some(ref uv) = ctx.user_view {
                uv.search(query, limit).await
            } else {
                ctx.memory.search(query, limit).await
            };
            match search_result {
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
                        raw_content: None,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Memory search error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "MemoryWrite" => {
            let file = input.get("file")?.as_str()?;
            let content = input.get("content")?.as_str()?;

            debug!(file = %file, "MemoryWrite");

            let write_result = if let Some(ref uv) = ctx.user_view {
                uv.write_file(file, content).await
            } else {
                ctx.memory.write_file(file, content).await
            };
            match write_result {
                Ok(()) => Some(ToolResult {
                    content: format!("Successfully wrote {}", file),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Memory write error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "MemoryAppendDaily" => {
            let text = input.get("text")?.as_str()?;

            debug!("MemoryAppendDaily");

            let append_result = if let Some(ref uv) = ctx.user_view {
                uv.append_daily(text).await
            } else {
                ctx.memory.append_daily(text).await
            };
            match append_result {
                Ok(()) => Some(ToolResult {
                    content: "Appended to daily log.".into(),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Daily append error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        // --- Env tool ---
        "EnvGet" => {
            let key = input.get("key")?.as_str()?;

            debug!(key = %key, "EnvGet");

            // Block sensitive environment variables
            let upper = key.to_uppercase();
            const BLOCKED: &[&str] = &["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH"];
            if BLOCKED.iter().any(|pat| upper.contains(pat)) {
                return Some(ToolResult {
                    content: format!("Access to environment variable '{}' is restricted.", key),
                    is_error: true,
                    raw_content: None,
                });
            }

            match std::env::var(key) {
                Ok(value) => Some(ToolResult {
                    content: value,
                    is_error: false,
                    raw_content: None,
                }),
                Err(_) => Some(ToolResult {
                    content: format!("Environment variable '{}' is not set.", key),
                    is_error: false,
                    raw_content: None,
                }),
            }
        }

        // --- File tools ---
        "FileRead" => {
            let path = input.get("path")?.as_str()?;

            debug!(path = %path, "FileRead");

            match validate_sandbox_path(path, &ctx.home_dir) {
                Ok(resolved) => {
                    if !resolved.is_file() {
                        return Some(ToolResult {
                            content: format!("File not found: {}", path),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                    match std::fs::read_to_string(&resolved) {
                        Ok(content) => Some(ToolResult {
                            content,
                            is_error: false,
                            raw_content: None,
                        }),
                        Err(e) => Some(ToolResult {
                            content: format!("Failed to read file: {}", e),
                            is_error: true,
                            raw_content: None,
                        }),
                    }
                }
                Err(e) => Some(ToolResult {
                    content: format!("Invalid path: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "FileWrite" => {
            let path = input.get("path")?.as_str()?;
            let content = input.get("content")?.as_str()?;

            debug!(path = %path, "FileWrite");

            match validate_sandbox_path(path, &ctx.home_dir) {
                Ok(resolved) => {
                    // Create parent directories
                    if let Some(parent) = resolved.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            return Some(ToolResult {
                                content: format!("Failed to create directories: {}", e),
                                is_error: true,
                                raw_content: None,
                            });
                        }
                    }
                    match std::fs::write(&resolved, content) {
                        Ok(()) => Some(ToolResult {
                            content: format!("Successfully wrote {}", path),
                            is_error: false,
                            raw_content: None,
                        }),
                        Err(e) => Some(ToolResult {
                            content: format!("Failed to write file: {}", e),
                            is_error: true,
                            raw_content: None,
                        }),
                    }
                }
                Err(e) => Some(ToolResult {
                    content: format!("Invalid path: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "FileList" => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");

            debug!(path = %path, "FileList");

            let resolved = if path == "." {
                ctx.home_dir.clone()
            } else {
                match validate_sandbox_path(path, &ctx.home_dir) {
                    Ok(p) => p,
                    Err(e) => return Some(ToolResult {
                        content: format!("Invalid path: {}", e),
                        is_error: true,
                        raw_content: None,
                    }),
                }
            };

            if !resolved.is_dir() {
                return Some(ToolResult {
                    content: format!("Not a directory: {}", path),
                    is_error: true,
                    raw_content: None,
                });
            }

            match std::fs::read_dir(&resolved) {
                Ok(entries) => {
                    let mut items: Vec<serde_json::Value> = Vec::new();
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        // Hide .starpod from listings
                        if name == ".starpod" {
                            continue;
                        }
                        let meta = entry.metadata().ok();
                        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        items.push(json!({
                            "name": if is_dir { format!("{}/", name) } else { name },
                            "size": size,
                            "type": if is_dir { "directory" } else { "file" },
                        }));
                    }
                    items.sort_by(|a, b| {
                        a.get("name").and_then(|v| v.as_str())
                            .cmp(&b.get("name").and_then(|v| v.as_str()))
                    });
                    Some(ToolResult {
                        content: serde_json::to_string_pretty(&items).unwrap_or_default(),
                        is_error: false,
                        raw_content: None,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Failed to list directory: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "FileDelete" => {
            let path = input.get("path")?.as_str()?;

            debug!(path = %path, "FileDelete");

            match validate_sandbox_path(path, &ctx.home_dir) {
                Ok(resolved) => {
                    if !resolved.exists() {
                        return Some(ToolResult {
                            content: format!("File not found: {}", path),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                    match std::fs::remove_file(&resolved) {
                        Ok(()) => Some(ToolResult {
                            content: format!("Deleted {}", path),
                            is_error: false,
                            raw_content: None,
                        }),
                        Err(e) => Some(ToolResult {
                            content: format!("Failed to delete file: {}", e),
                            is_error: true,
                            raw_content: None,
                        }),
                    }
                }
                Err(e) => Some(ToolResult {
                    content: format!("Invalid path: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        // --- Skill tools ---
        "SkillActivate" => {
            let name = input.get("name")?.as_str()?;

            debug!(skill = %name, "SkillActivate");

            match ctx.skills.activate_skill(name) {
                Ok(Some(content)) => Some(ToolResult {
                    content,
                    is_error: false,
                    raw_content: None,
                }),
                Ok(None) => Some(ToolResult {
                    content: format!("Skill '{}' not found.", name),
                    is_error: true,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Skill activate error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "SkillCreate" => {
            let name = input.get("name")?.as_str()?;
            let description = input.get("description")?.as_str()?;
            let body = input.get("body")?.as_str()?;

            debug!(skill = %name, "SkillCreate");

            match ctx.skills.create(name, description, None, body) {
                Ok(()) => Some(ToolResult {
                    content: format!("Created skill '{}'.", name),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Skill create error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "SkillUpdate" => {
            let name = input.get("name")?.as_str()?;
            let description = input.get("description")?.as_str()?;
            let body = input.get("body")?.as_str()?;

            debug!(skill = %name, "SkillUpdate");

            match ctx.skills.update(name, description, None, body) {
                Ok(()) => Some(ToolResult {
                    content: format!("Updated skill '{}'.", name),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Skill update error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "SkillDelete" => {
            let name = input.get("name")?.as_str()?;

            debug!(skill = %name, "SkillDelete");

            match ctx.skills.delete(name) {
                Ok(()) => Some(ToolResult {
                    content: format!("Deleted skill '{}'.", name),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Skill delete error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "SkillList" => {
            debug!("SkillList");

            match ctx.skills.list() {
                Ok(skills) => {
                    let formatted: Vec<serde_json::Value> = skills
                        .iter()
                        .map(|s| {
                            json!({
                                "name": s.name,
                                "description": s.description,
                                "created_at": s.created_at,
                            })
                        })
                        .collect();
                    Some(ToolResult {
                        content: serde_json::to_string_pretty(&formatted).unwrap_or_default(),
                        is_error: false,
                        raw_content: None,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Skill list error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        // --- Cron tools ---
        "CronAdd" => {
            let name = input.get("name")?.as_str()?;
            let prompt = input.get("prompt")?.as_str()?;
            let schedule_val = input.get("schedule")?;
            let delete_after_run = input
                .get("delete_after_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let max_retries = input
                .get("max_retries")
                .and_then(|v| v.as_u64())
                .unwrap_or(3) as u32;
            let timeout_secs = input
                .get("timeout_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(7200) as u32;
            let session_mode = match input.get("session_mode").and_then(|v| v.as_str()) {
                Some("main") => starpod_cron::SessionMode::Main,
                _ => starpod_cron::SessionMode::Isolated,
            };

            let schedule: starpod_cron::Schedule = match serde_json::from_value(schedule_val.clone())
            {
                Ok(s) => s,
                Err(e) => {
                    return Some(ToolResult {
                        content: format!("Invalid schedule: {}", e),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            // Validate one-shot timestamps: parseable and in the future
            if let starpod_cron::Schedule::OneShot { ref at } = schedule {
                match starpod_cron::store::compute_next_run(&schedule, None, ctx.user_tz.as_deref()) {
                    Ok(Some(_)) => {} // valid and in the future
                    Ok(None) => {
                        return Some(ToolResult {
                            content: format!(
                                "One-shot timestamp '{}' is in the past. Use a future timestamp.",
                                at
                            ),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                    Err(e) => {
                        return Some(ToolResult {
                            content: format!("Invalid one-shot timestamp '{}': {}", at, e),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                }
            }

            debug!(job = %name, "CronAdd");

            match ctx.cron.add_job_full(name, prompt, &schedule, delete_after_run, ctx.user_tz.as_deref(), max_retries, timeout_secs, session_mode, ctx.user_id.as_deref()).await {
                Ok(id) => Some(ToolResult {
                    content: format!("Scheduled job '{}' (id: {})", name, &id[..8]),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Cron add error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "CronList" => {
            debug!("CronList");

            match ctx.cron.list_jobs().await {
                Ok(jobs) => {
                    let formatted: Vec<serde_json::Value> = jobs
                        .iter()
                        .map(|j| {
                            let mut obj = json!({
                                "name": j.name,
                                "prompt": j.prompt,
                                "schedule": j.schedule,
                                "enabled": j.enabled,
                                "session_mode": j.session_mode,
                                "max_retries": j.max_retries,
                                "timeout_secs": j.timeout_secs,
                                "last_run_at": j.last_run_at.map(epoch_to_rfc3339),
                                "next_run_at": j.next_run_at.map(epoch_to_rfc3339),
                            });
                            if j.retry_count > 0 {
                                obj["retry_count"] = json!(j.retry_count);
                            }
                            if let Some(ref err) = j.last_error {
                                obj["last_error"] = json!(err);
                            }
                            if let Some(retry_at) = j.retry_at {
                                obj["retry_at"] = json!(epoch_to_rfc3339(retry_at));
                            }
                            obj
                        })
                        .collect();
                    Some(ToolResult {
                        content: serde_json::to_string_pretty(&formatted).unwrap_or_default(),
                        is_error: false,
                        raw_content: None,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Cron list error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "CronRemove" => {
            let name = input.get("name")?.as_str()?;

            debug!(job = %name, "CronRemove");

            match ctx.cron.remove_job_by_name(name).await {
                Ok(()) => Some(ToolResult {
                    content: format!("Removed job '{}'.", name),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Cron remove error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "CronRuns" => {
            let name = input.get("name")?.as_str()?;
            let limit = input
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;

            debug!(job = %name, "CronRuns");

            let job = match ctx.cron.get_job_by_name(name).await {
                Ok(Some(j)) => j,
                Ok(None) => {
                    return Some(ToolResult {
                        content: format!("No job found with name '{}'", name),
                        is_error: true,
                        raw_content: None,
                    });
                }
                Err(e) => {
                    return Some(ToolResult {
                        content: format!("Cron error: {}", e),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            match ctx.cron.list_runs(&job.id, limit).await {
                Ok(runs) => {
                    let formatted: Vec<serde_json::Value> = runs
                        .iter()
                        .map(|r| {
                            json!({
                                "started_at": epoch_to_rfc3339(r.started_at),
                                "completed_at": r.completed_at.map(epoch_to_rfc3339),
                                "status": r.status,
                                "result_summary": r.result_summary,
                            })
                        })
                        .collect();
                    Some(ToolResult {
                        content: serde_json::to_string_pretty(&formatted).unwrap_or_default(),
                        is_error: false,
                        raw_content: None,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Cron runs error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "CronRun" => {
            let name = input.get("name")?.as_str()?;

            debug!(job = %name, "CronRun");

            let job = match ctx.cron.get_job_by_name(name).await {
                Ok(Some(j)) => j,
                Ok(None) => {
                    return Some(ToolResult {
                        content: format!("No job found with name '{}'", name),
                        is_error: true,
                        raw_content: None,
                    });
                }
                Err(e) => {
                    return Some(ToolResult {
                        content: format!("Cron error: {}", e),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            let run_id = match ctx.cron.record_run_start(&job.id).await {
                Ok(id) => id,
                Err(e) => {
                    return Some(ToolResult {
                        content: format!("Failed to record run: {}", e),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            // Mark the run as complete immediately — the LLM will handle the
            // job's prompt inline within the current conversation.
            let _ = ctx
                .cron
                .record_run_complete(
                    &run_id,
                    RunStatus::Success,
                    Some("Manual run triggered inline by CronRun tool"),
                )
                .await;

            Some(ToolResult {
                content: format!(
                    "Manual run recorded for job '{}'. Execute the following prompt:\n\n{}",
                    name, job.prompt
                ),
                is_error: false,
                raw_content: None,
            })
        }

        "CronUpdate" => {
            let name = input.get("name")?.as_str()?;

            debug!(job = %name, "CronUpdate");

            let job = match ctx.cron.get_job_by_name(name).await {
                Ok(Some(j)) => j,
                Ok(None) => {
                    return Some(ToolResult {
                        content: format!("No job found with name '{}'", name),
                        is_error: true,
                        raw_content: None,
                    });
                }
                Err(e) => {
                    return Some(ToolResult {
                        content: format!("Cron error: {}", e),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            // Parse optional new schedule
            let new_schedule: Option<starpod_cron::Schedule> = match input.get("schedule") {
                Some(val) => match serde_json::from_value(val.clone()) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        return Some(ToolResult {
                            content: format!("Invalid schedule: {}", e),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                },
                None => None,
            };

            // Validate one-shot timestamps: parseable and in the future
            if let Some(ref sched @ starpod_cron::Schedule::OneShot { ref at }) = new_schedule {
                match starpod_cron::store::compute_next_run(sched, None, ctx.user_tz.as_deref()) {
                    Ok(Some(_)) => {} // valid and in the future
                    Ok(None) => {
                        return Some(ToolResult {
                            content: format!(
                                "One-shot timestamp '{}' is in the past. Use a future timestamp.",
                                at
                            ),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                    Err(e) => {
                        return Some(ToolResult {
                            content: format!("Invalid one-shot timestamp '{}': {}", at, e),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                }
            }

            let update = starpod_cron::JobUpdate {
                prompt: input.get("prompt").and_then(|v| v.as_str()).map(String::from),
                schedule: new_schedule.clone(),
                enabled: input.get("enabled").and_then(|v| v.as_bool()),
                max_retries: input.get("max_retries").and_then(|v| v.as_u64()).map(|v| v as u32),
                timeout_secs: input.get("timeout_secs").and_then(|v| v.as_u64()).map(|v| v as u32),
                session_mode: input.get("session_mode").and_then(|v| v.as_str()).map(|s| {
                    starpod_cron::SessionMode::from_str(s)
                }),
            };

            if let Err(e) = ctx.cron.update_job(&job.id, &update).await {
                return Some(ToolResult {
                    content: format!("Cron update error: {}", e),
                    is_error: true,
                    raw_content: None,
                });
            }

            // If schedule changed, recompute next_run_at
            if let Some(ref schedule) = new_schedule {
                match starpod_cron::store::compute_next_run(schedule, None, ctx.user_tz.as_deref()) {
                    Ok(next) => {
                        let _ = ctx.cron.update_next_run(&job.id, next).await;
                    }
                    Err(e) => {
                        return Some(ToolResult {
                            content: format!("Updated job '{}' but failed to recompute schedule: {}", name, e),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                }
            }

            Some(ToolResult {
                content: format!("Updated job '{}'.", name),
                is_error: false,
                raw_content: None,
            })
        }

        "HeartbeatWake" => {
            let mode = input.get("mode").and_then(|v| v.as_str()).unwrap_or("next");

            debug!(mode = %mode, "HeartbeatWake");

            if mode == "now" {
                let job = match ctx.cron.get_job_by_name("__heartbeat__").await {
                    Ok(Some(j)) => j,
                    Ok(None) => {
                        return Some(ToolResult {
                            content: "No heartbeat job found. Heartbeat will be created on next server start.".into(),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                    Err(e) => {
                        return Some(ToolResult {
                            content: format!("Heartbeat error: {}", e),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                };

                let now = chrono::Utc::now().timestamp();
                match ctx.cron.update_next_run(&job.id, Some(now)).await {
                    Ok(()) => {
                        if let Some(message) = input.get("message").and_then(|v| v.as_str()) {
                            let update = starpod_cron::JobUpdate {
                                prompt: Some(message.to_string()),
                                ..Default::default()
                            };
                            let _ = ctx.cron.update_job(&job.id, &update).await;
                        }
                        Some(ToolResult {
                            content: "Heartbeat will fire on the next scheduler tick.".into(),
                            is_error: false,
                            raw_content: None,
                        })
                    }
                    Err(e) => Some(ToolResult {
                        content: format!("Heartbeat wake error: {}", e),
                        is_error: true,
                        raw_content: None,
                    }),
                }
            } else {
                Some(ToolResult {
                    content: "Heartbeat will fire on its natural schedule (every 30 minutes).".into(),
                    is_error: false,
                    raw_content: None,
                })
            }
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── validate_sandbox_path ───────────────────────────────────────

    #[test]
    fn sandbox_rejects_absolute_path() {
        let tmp = TempDir::new().unwrap();
        let err = validate_sandbox_path("/etc/passwd", tmp.path());
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("Absolute"));
    }

    #[test]
    fn sandbox_rejects_dot_dot_traversal() {
        let tmp = TempDir::new().unwrap();
        let err = validate_sandbox_path("../escape.txt", tmp.path());
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("traversal"));
    }

    #[test]
    fn sandbox_rejects_starpod_dir() {
        let tmp = TempDir::new().unwrap();
        let err = validate_sandbox_path(".starpod/agent.toml", tmp.path());
        assert!(err.is_err());
        assert!(err.unwrap_err().contains(".starpod"));
    }

    #[test]
    fn sandbox_rejects_starpod_dir_exact() {
        let tmp = TempDir::new().unwrap();
        let err = validate_sandbox_path(".starpod", tmp.path());
        assert!(err.is_err());
    }

    #[test]
    fn sandbox_allows_normal_path() {
        let tmp = TempDir::new().unwrap();
        let result = validate_sandbox_path("reports/weekly.md", tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tmp.path().join("reports/weekly.md"));
    }

    #[test]
    fn sandbox_allows_root_file() {
        let tmp = TempDir::new().unwrap();
        let result = validate_sandbox_path("notes.txt", tmp.path());
        assert!(result.is_ok());
    }

    // ── EnvGet handler ──────────────────────────────────────────────

    #[tokio::test]
    async fn env_get_returns_value() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(starpod_memory::MemoryStore::new(&tmp.path().join("agent"), &tmp.path().join("agent").join("config"), &tmp.path().join("db")).await.unwrap());
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        let ctx = ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            user_id: Some("admin".into()),
        };

        std::env::set_var("STARPOD_ENVGET_TEST_VAR", "test_value_42");
        let result = handle_custom_tool(
            &ctx,
            "EnvGet",
            &serde_json::json!({"key": "STARPOD_ENVGET_TEST_VAR"}),
        ).await;
        std::env::remove_var("STARPOD_ENVGET_TEST_VAR");

        let result = result.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "test_value_42");
    }

    #[tokio::test]
    async fn env_get_missing_key() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(starpod_memory::MemoryStore::new(&tmp.path().join("agent"), &tmp.path().join("agent").join("config"), &tmp.path().join("db")).await.unwrap());
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        let ctx = ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            user_id: Some("admin".into()),
        };

        let result = handle_custom_tool(
            &ctx,
            "EnvGet",
            &serde_json::json!({"key": "STARPOD_DEFINITELY_NOT_SET_EVER"}),
        ).await;

        let result = result.unwrap();
        assert!(!result.is_error); // not an error, just "not set"
        assert!(result.content.contains("not set"));
    }

    #[tokio::test]
    async fn env_get_blocks_sensitive_vars() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(starpod_memory::MemoryStore::new(&tmp.path().join("agent"), &tmp.path().join("agent").join("config"), &tmp.path().join("db")).await.unwrap());
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        let ctx = ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            user_id: Some("admin".into()),
        };

        // All of these should be blocked
        for key in &["ANTHROPIC_API_KEY", "STARPOD_API_KEY", "TELEGRAM_BOT_TOKEN",
                     "DB_PASSWORD", "MY_SECRET", "AWS_CREDENTIAL", "OAUTH_AUTH_CODE"] {
            let result = handle_custom_tool(
                &ctx,
                "EnvGet",
                &serde_json::json!({"key": key}),
            ).await.unwrap();
            assert!(result.is_error, "EnvGet should block sensitive var: {}", key);
            assert!(result.content.contains("restricted"));
        }

        // These should be allowed
        for key in &["HOME", "PATH", "LANG", "TERM", "SHELL"] {
            let result = handle_custom_tool(
                &ctx,
                "EnvGet",
                &serde_json::json!({"key": key}),
            ).await.unwrap();
            assert!(!result.is_error, "EnvGet should allow safe var: {}", key);
        }
    }

    // ── File tool handlers ──────────────────────────────────────────

    #[tokio::test]
    async fn file_write_and_read() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(starpod_memory::MemoryStore::new(&tmp.path().join("agent"), &tmp.path().join("agent").join("config"), &tmp.path().join("db")).await.unwrap());
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        let home_dir = tmp.path().join("instance");
        std::fs::create_dir_all(&home_dir).unwrap();

        let ctx = ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            user_tz: None,
            home_dir: home_dir.clone(),
            user_id: Some("admin".into()),
        };

        // Write a file
        let result = handle_custom_tool(
            &ctx,
            "FileWrite",
            &serde_json::json!({"path": "reports/test.txt", "content": "Hello world"}),
        ).await.unwrap();
        assert!(!result.is_error, "FileWrite failed: {}", result.content);

        // Read it back
        let result = handle_custom_tool(
            &ctx,
            "FileRead",
            &serde_json::json!({"path": "reports/test.txt"}),
        ).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "Hello world");
    }

    #[tokio::test]
    async fn file_list_hides_starpod() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(starpod_memory::MemoryStore::new(&tmp.path().join("agent"), &tmp.path().join("agent").join("config"), &tmp.path().join("db")).await.unwrap());
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        let home_dir = tmp.path().join("instance");
        std::fs::create_dir_all(home_dir.join(".starpod")).unwrap();
        std::fs::write(home_dir.join("visible.txt"), "hi").unwrap();

        let ctx = ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            user_tz: None,
            home_dir: home_dir.clone(),
            user_id: Some("admin".into()),
        };

        let result = handle_custom_tool(
            &ctx,
            "FileList",
            &serde_json::json!({}),
        ).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("visible.txt"));
        assert!(!result.content.contains(".starpod"), "FileList should hide .starpod");
    }

    #[tokio::test]
    async fn file_delete_works() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(starpod_memory::MemoryStore::new(&tmp.path().join("agent"), &tmp.path().join("agent").join("config"), &tmp.path().join("db")).await.unwrap());
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        let home_dir = tmp.path().join("instance");
        std::fs::create_dir_all(&home_dir).unwrap();
        std::fs::write(home_dir.join("deleteme.txt"), "bye").unwrap();

        let ctx = ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            user_tz: None,
            home_dir: home_dir.clone(),
            user_id: Some("admin".into()),
        };

        let result = handle_custom_tool(
            &ctx,
            "FileDelete",
            &serde_json::json!({"path": "deleteme.txt"}),
        ).await.unwrap();
        assert!(!result.is_error);
        assert!(!home_dir.join("deleteme.txt").exists());
    }

    #[tokio::test]
    async fn file_read_rejects_starpod() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(starpod_memory::MemoryStore::new(&tmp.path().join("agent"), &tmp.path().join("agent").join("config"), &tmp.path().join("db")).await.unwrap());
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        let home_dir = tmp.path().join("instance");
        let starpod = home_dir.join(".starpod");
        std::fs::create_dir_all(&starpod).unwrap();
        std::fs::write(starpod.join("agent.toml"), "secret").unwrap();

        let ctx = ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            user_tz: None,
            home_dir: home_dir.clone(),
            user_id: Some("admin".into()),
        };

        let result = handle_custom_tool(
            &ctx,
            "FileRead",
            &serde_json::json!({"path": ".starpod/agent.toml"}),
        ).await.unwrap();
        assert!(result.is_error, "FileRead should reject .starpod/ paths");
    }

    #[tokio::test]
    async fn file_write_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(starpod_memory::MemoryStore::new(&tmp.path().join("agent"), &tmp.path().join("agent").join("config"), &tmp.path().join("db")).await.unwrap());
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        let home_dir = tmp.path().join("instance");
        std::fs::create_dir_all(&home_dir).unwrap();

        let ctx = ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            user_tz: None,
            home_dir,
            user_id: Some("admin".into()),
        };

        let result = handle_custom_tool(
            &ctx,
            "FileWrite",
            &serde_json::json!({"path": "../escape.txt", "content": "evil"}),
        ).await.unwrap();
        assert!(result.is_error, "FileWrite should reject .. traversal");
    }

    // ── Per-user memory routing via UserMemoryView ─────────────────

    /// Helper: build a ToolContext with a UserMemoryView attached.
    async fn ctx_with_user_view(tmp: &TempDir) -> ToolContext {
        let agent_home = tmp.path().join("agent");
        let config_dir = agent_home.join("config");
        let db_dir = tmp.path().join("db");
        let user_dir = tmp.path().join("users").join("alice");

        let memory = Arc::new(
            starpod_memory::MemoryStore::new(&agent_home, &config_dir, &db_dir)
                .await
                .unwrap(),
        );
        let user_view = starpod_memory::UserMemoryView::new(Arc::clone(&memory), user_dir)
            .unwrap();
        let skills = Arc::new(
            starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap(),
        );
        let cron = Arc::new(
            starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap(),
        );

        ToolContext {
            memory,
            user_view: Some(user_view),
            skills,
            cron,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            user_id: Some("alice".into()),
        }
    }

    #[tokio::test]
    async fn memory_write_routes_user_md_to_user_dir() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        let result = handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "USER.md", "content": "# User\n\nAlice likes Rust."}),
        ).await.unwrap();
        assert!(!result.is_error, "MemoryWrite should succeed: {}", result.content);

        // USER.md should be in the per-user directory, not agent home
        let user_file = tmp.path().join("users/alice/USER.md");
        let content = std::fs::read_to_string(&user_file).unwrap();
        assert!(content.contains("Alice likes Rust"), "USER.md should be in user dir");
    }

    #[tokio::test]
    async fn memory_write_routes_daily_to_user_dir() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        let result = handle_custom_tool(
            &ctx,
            "MemoryAppendDaily",
            &serde_json::json!({"text": "Learned about lifetimes today."}),
        ).await.unwrap();
        assert!(!result.is_error, "MemoryAppendDaily should succeed: {}", result.content);

        // Daily log should be in user's memory/ directory
        let memory_dir = tmp.path().join("users/alice/memory");
        assert!(memory_dir.is_dir(), "user memory dir should exist");
        let entries: Vec<_> = std::fs::read_dir(&memory_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(!entries.is_empty(), "daily log should be written");
        let content = std::fs::read_to_string(entries[0].path()).unwrap();
        assert!(content.contains("lifetimes"), "daily log should contain appended text");
    }

    #[tokio::test]
    async fn memory_search_uses_user_view_when_present() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // Write something searchable first
        handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "USER.md", "content": "# User\n\nAlice is a quantum physicist."}),
        ).await.unwrap();

        let result = handle_custom_tool(
            &ctx,
            "MemorySearch",
            &serde_json::json!({"query": "quantum physicist"}),
        ).await.unwrap();
        assert!(!result.is_error, "MemorySearch should succeed: {}", result.content);
    }

    #[tokio::test]
    async fn memory_write_agent_file_goes_to_agent_store() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // SOUL.md is an agent-level file, should NOT go to user dir
        let result = handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "SOUL.md", "content": "# Soul\n\nI am helpful."}),
        ).await.unwrap();
        assert!(!result.is_error, "MemoryWrite for SOUL.md should succeed");

        // SOUL.md should be in agent config dir, not user dir
        let agent_soul = tmp.path().join("agent/config/SOUL.md");
        assert!(agent_soul.is_file(), "SOUL.md should be in agent config dir");
        let user_soul = tmp.path().join("users/alice/SOUL.md");
        assert!(!user_soul.exists(), "SOUL.md should NOT be in user dir");
    }
}
