//! Custom tool definitions and handler for Orion-specific tools.

use std::sync::Arc;

use agent_sdk::{CustomToolDefinition, ToolResult};
use serde_json::json;
use tracing::debug;

use orion_cron::CronStore;
use orion_memory::MemoryStore;
use orion_skills::SkillStore;
use orion_vault::Vault;

/// Shared context for custom tool handlers.
pub struct ToolContext {
    pub memory: Arc<MemoryStore>,
    pub vault: Arc<Vault>,
    pub skills: Arc<SkillStore>,
    pub cron: Arc<CronStore>,
}

/// Build the JSON schema definitions for all Orion custom tools.
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
        // --- Vault tools ---
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
        // --- Skill tools ---
        CustomToolDefinition {
            name: "SkillCreate".into(),
            description: "Create a new skill that extends your capabilities. Skills are markdown files that get injected into your system prompt.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Skill name (used as directory name, e.g. 'summarize-pr')"
                    },
                    "content": {
                        "type": "string",
                        "description": "Markdown content describing the skill's instructions and behavior"
                    }
                },
                "required": ["name", "content"]
            }),
        },
        CustomToolDefinition {
            name: "SkillUpdate".into(),
            description: "Update an existing skill's content.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to update"
                    },
                    "content": {
                        "type": "string",
                        "description": "New markdown content for the skill"
                    }
                },
                "required": ["name", "content"]
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
            description: "List all available skills.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Cron tools ---
        CustomToolDefinition {
            name: "CronAdd".into(),
            description: "Schedule a recurring or one-shot task. The prompt will be sent to you as a message when the job fires.".into(),
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
                                "description": "ISO 8601 timestamp (for 'one_shot' kind)"
                            }
                        },
                        "required": ["kind"]
                    },
                    "delete_after_run": {
                        "type": "boolean",
                        "description": "If true, automatically delete the job after it runs once (default: false)"
                    }
                },
                "required": ["name", "prompt", "schedule"]
            }),
        },
        CustomToolDefinition {
            name: "CronList".into(),
            description: "List all scheduled cron jobs.".into(),
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
    ]
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

            match ctx.memory.search(query, limit).await {
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

            match ctx.memory.write_file(file, content).await {
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

            match ctx.memory.append_daily(text).await {
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

        // --- Vault tools ---
        "VaultGet" => {
            let key = input.get("key")?.as_str()?;

            debug!(key = %key, "VaultGet");

            match ctx.vault.get(key).await {
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

            match ctx.vault.set(key, value).await {
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

        // --- Skill tools ---
        "SkillCreate" => {
            let name = input.get("name")?.as_str()?;
            let content = input.get("content")?.as_str()?;

            debug!(skill = %name, "SkillCreate");

            match ctx.skills.create(name, content) {
                Ok(()) => Some(ToolResult {
                    content: format!("Created skill '{}'.", name),
                    is_error: false,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Skill create error: {}", e),
                    is_error: true,
                }),
            }
        }

        "SkillUpdate" => {
            let name = input.get("name")?.as_str()?;
            let content = input.get("content")?.as_str()?;

            debug!(skill = %name, "SkillUpdate");

            match ctx.skills.update(name, content) {
                Ok(()) => Some(ToolResult {
                    content: format!("Updated skill '{}'.", name),
                    is_error: false,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Skill update error: {}", e),
                    is_error: true,
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
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Skill delete error: {}", e),
                    is_error: true,
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
                                "created_at": s.created_at,
                                "content_preview": if s.content.len() > 100 {
                                    format!("{}...", &s.content[..100])
                                } else {
                                    s.content.clone()
                                },
                            })
                        })
                        .collect();
                    Some(ToolResult {
                        content: serde_json::to_string_pretty(&formatted).unwrap_or_default(),
                        is_error: false,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Skill list error: {}", e),
                    is_error: true,
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

            let schedule: orion_cron::Schedule = match serde_json::from_value(schedule_val.clone())
            {
                Ok(s) => s,
                Err(e) => {
                    return Some(ToolResult {
                        content: format!("Invalid schedule: {}", e),
                        is_error: true,
                    });
                }
            };

            debug!(job = %name, "CronAdd");

            match ctx.cron.add_job(name, prompt, &schedule, delete_after_run).await {
                Ok(id) => Some(ToolResult {
                    content: format!("Scheduled job '{}' (id: {})", name, &id[..8]),
                    is_error: false,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Cron add error: {}", e),
                    is_error: true,
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
                            json!({
                                "name": j.name,
                                "prompt": j.prompt,
                                "schedule": j.schedule,
                                "enabled": j.enabled,
                                "last_run_at": j.last_run_at,
                                "next_run_at": j.next_run_at,
                            })
                        })
                        .collect();
                    Some(ToolResult {
                        content: serde_json::to_string_pretty(&formatted).unwrap_or_default(),
                        is_error: false,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Cron list error: {}", e),
                    is_error: true,
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
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Cron remove error: {}", e),
                    is_error: true,
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

            // Find job by name first
            let jobs = match ctx.cron.list_jobs().await {
                Ok(j) => j,
                Err(e) => {
                    return Some(ToolResult {
                        content: format!("Cron error: {}", e),
                        is_error: true,
                    });
                }
            };

            let job = match jobs.iter().find(|j| j.name == name) {
                Some(j) => j,
                None => {
                    return Some(ToolResult {
                        content: format!("No job found with name '{}'", name),
                        is_error: true,
                    });
                }
            };

            match ctx.cron.list_runs(&job.id, limit).await {
                Ok(runs) => {
                    let formatted: Vec<serde_json::Value> = runs
                        .iter()
                        .map(|r| {
                            json!({
                                "started_at": r.started_at,
                                "completed_at": r.completed_at,
                                "status": r.status,
                                "result_summary": r.result_summary,
                            })
                        })
                        .collect();
                    Some(ToolResult {
                        content: serde_json::to_string_pretty(&formatted).unwrap_or_default(),
                        is_error: false,
                    })
                }
                Err(e) => Some(ToolResult {
                    content: format!("Cron runs error: {}", e),
                    is_error: true,
                }),
            }
        }

        _ => None,
    }
}
