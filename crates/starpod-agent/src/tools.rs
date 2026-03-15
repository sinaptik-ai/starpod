//! Custom tool definitions and handler for Starpod-specific tools.

use std::sync::Arc;

use agent_sdk::{CustomToolDefinition, ToolResult};
use serde_json::json;
use tracing::debug;

use starpod_cron::store::epoch_to_rfc3339;
use starpod_cron::CronStore;
use starpod_memory::MemoryStore;
use starpod_skills::SkillStore;
use starpod_vault::Vault;

/// Shared context for custom tool handlers.
pub struct ToolContext {
    pub memory: Arc<MemoryStore>,
    pub vault: Arc<Vault>,
    pub skills: Arc<SkillStore>,
    pub cron: Arc<CronStore>,
    pub user_tz: Option<String>,
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
                                "description": "ISO 8601 timestamp (for 'one_shot' kind)"
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
            description: "Update properties of an existing cron job by name.".into(),
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

            match ctx.memory.write_file(file, content).await {
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

            match ctx.memory.append_daily(text).await {
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

        // --- Vault tools ---
        "VaultGet" => {
            let key = input.get("key")?.as_str()?;

            debug!(key = %key, "VaultGet");

            match ctx.vault.get(key).await {
                Ok(Some(value)) => Some(ToolResult {
                    content: value,
                    is_error: false,
                    raw_content: None,
                }),
                Ok(None) => Some(ToolResult {
                    content: format!("No vault entry found for key: {}", key),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Vault get error: {}", e),
                    is_error: true,
                    raw_content: None,
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
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Vault set error: {}", e),
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

            match ctx.skills.create(name, description, body) {
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

            match ctx.skills.update(name, description, body) {
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

            debug!(job = %name, "CronAdd");

            match ctx.cron.add_job_full(name, prompt, &schedule, delete_after_run, ctx.user_tz.as_deref(), max_retries, timeout_secs, session_mode).await {
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
                            // Show retry info only when relevant
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

            // Look up job by name
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

            // Record run
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

            Some(ToolResult {
                content: format!(
                    "Manual run started for job '{}' (run_id: {}). The job prompt will now execute with session_mode={}.",
                    name, &run_id[..8], job.session_mode.as_str()
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

            let update = starpod_cron::JobUpdate {
                prompt: input.get("prompt").and_then(|v| v.as_str()).map(String::from),
                schedule: None, // schedule updates not exposed via this tool
                enabled: input.get("enabled").and_then(|v| v.as_bool()),
                max_retries: input.get("max_retries").and_then(|v| v.as_u64()).map(|v| v as u32),
                timeout_secs: input.get("timeout_secs").and_then(|v| v.as_u64()).map(|v| v as u32),
                session_mode: input.get("session_mode").and_then(|v| v.as_str()).map(|s| {
                    starpod_cron::SessionMode::from_str(s)
                }),
            };

            match ctx.cron.update_job(&job.id, &update).await {
                Ok(()) => Some(ToolResult {
                    content: format!("Updated job '{}'.", name),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Cron update error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "HeartbeatWake" => {
            let mode = input.get("mode").and_then(|v| v.as_str()).unwrap_or("next");

            debug!(mode = %mode, "HeartbeatWake");

            if mode == "now" {
                // Set heartbeat's next_run_at to now
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
                        // If a message was provided, update the heartbeat prompt
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
