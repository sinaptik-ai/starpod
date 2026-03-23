//! Custom tool definitions and handler for Starpod-specific tools.
//!
//! This module implements all Starpod-specific tools that extend the agent-sdk's
//! built-in tool set. Tools are grouped into categories:
//!
//! - **Memory** — search, read, write, and append to long-term memory
//! - **Environment** — read environment variables (with sensitive-key blocking)
//! - **File sandbox** — read/write/list/delete files within the instance home directory
//! - **Skills** — CRUD for self-extension skills
//! - **Cron** — schedule and manage recurring/one-shot jobs
//! - **Web** — search the internet via Brave Search and fetch web pages
//!
//! Each tool is defined as a [`CustomToolDefinition`] (JSON schema sent to Claude)
//! and handled by [`handle_custom_tool`], which dispatches on tool name.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_sdk::{CustomToolDefinition, ToolResult};
use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};
use reqwest::{Client, Url};
use serde_json::json;
use tracing::debug;

use starpod_core::config::InternetConfig;
use starpod_browser::BrowserSession;
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
    pub browser: Arc<tokio::sync::Mutex<Option<BrowserSession>>>,
    pub browser_enabled: bool,
    pub browser_cdp_url: Option<String>,
    pub user_tz: Option<String>,
    pub home_dir: PathBuf,
    /// The `.starpod/` directory path — used to detect and block Bash commands
    /// that try to access internal config/data files.
    pub agent_home: PathBuf,
    pub user_id: Option<String>,
    /// Shared HTTP client for web tools (WebSearch, WebFetch).
    pub http_client: Client,
    /// Internet access configuration (enabled flag, timeouts, size limits).
    pub internet: InternetConfig,
    /// Brave Search API key, read from the `BRAVE_API_KEY` environment variable.
    /// When `None`, WebSearch returns an error prompting the user to set it.
    pub brave_api_key: Option<String>,
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
            name: "MemoryRead".into(),
            description: "Read a file from memory. Use after MemorySearch to get full context around a result.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file": {
                        "type": "string",
                        "description": "Relative file path (e.g. 'MEMORY.md', 'memory/2026-03-21.md')"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "Start line (1-indexed, optional — omit to read entire file)"
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "End line (optional)"
                    }
                },
                "required": ["file"]
            }),
        },
        CustomToolDefinition {
            name: "MemoryWrite".into(),
            description: "Write or update a file in the user's memory store (e.g. USER.md, MEMORY.md, memory/*.md).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file": {
                        "type": "string",
                        "description": "Relative file path within the memory store (e.g. 'USER.md', 'memory/notes.md')"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write (or append) to the file"
                    },
                    "append": {
                        "type": "boolean",
                        "description": "If true, append to existing file instead of overwriting (default: false)"
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
            description: "Read a file from the agent's filesystem sandbox. Path must be relative to the home directory (e.g. \"notes.txt\", \"reports/weekly.md\"). No \"..\" traversal or absolute paths. The .starpod/ directory is internal and cannot be accessed — use MemorySearch/MemoryWrite for USER.md, MEMORY.md, and memory files instead.".into(),
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
            description: "Write a file to the agent's filesystem sandbox. Path must be relative to the home directory (e.g. \"notes.txt\", \"reports/weekly.md\"). No \"..\" traversal or absolute paths. Creates parent directories as needed. The .starpod/ directory is internal and cannot be accessed — use MemoryWrite for USER.md, MEMORY.md, and memory files instead.".into(),
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
        // --- Web tools ---
        CustomToolDefinition {
            name: "WebSearch".into(),
            description: "Search the web using Brave Search and return results. Use this to find current information, answer questions about recent events, look up documentation, etc.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of results to return (default: 5, max: 20)"
                    }
                },
                "required": ["query"]
            }),
        },
        CustomToolDefinition {
            name: "WebFetch".into(),
            description: "Fetch a web page and extract its text content. Use this to read articles, documentation, or any web page. Returns the page content as markdown.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    }
                },
                "required": ["url"]
            }),
        },
        // --- Browser tools ---
        CustomToolDefinition {
            name: "BrowserOpen".into(),
            description: "Open a browser and navigate to a URL. Auto-launches a lightweight browser process if not already running. Returns the page title.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to navigate to"
                    }
                },
                "required": ["url"]
            }),
        },
        CustomToolDefinition {
            name: "BrowserClick".into(),
            description: "Click an element on the page by CSS selector.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for the element to click (e.g. 'button.submit', '#login-btn')"
                    }
                },
                "required": ["selector"]
            }),
        },
        CustomToolDefinition {
            name: "BrowserType".into(),
            description: "Type text into an input element identified by CSS selector.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for the input element"
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to type into the element"
                    }
                },
                "required": ["selector", "text"]
            }),
        },
        CustomToolDefinition {
            name: "BrowserExtract".into(),
            description: "Extract text content from the current page or a specific element. Without a selector, returns the full page text.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "Optional CSS selector to extract text from a specific element"
                    }
                }
            }),
        },
        CustomToolDefinition {
            name: "BrowserEval".into(),
            description: "Execute JavaScript on the current browser page and return the result.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "javascript": {
                        "type": "string",
                        "description": "JavaScript code to execute in the page context"
                    }
                },
                "required": ["javascript"]
            }),
        },
        CustomToolDefinition {
            name: "BrowserWaitFor".into(),
            description: "Wait for a condition on the current page. Use after clicking a button or submitting a form to wait for navigation or DOM changes. Provide exactly one of: url_contains, selector, or javascript.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url_contains": {
                        "type": "string",
                        "description": "Wait until the page URL contains this substring"
                    },
                    "selector": {
                        "type": "string",
                        "description": "Wait until an element matching this CSS selector exists on the page"
                    },
                    "javascript": {
                        "type": "string",
                        "description": "Wait until this JavaScript expression returns a truthy value"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Max wait time in milliseconds (default: 10000)"
                    }
                }
            }),
        },
        CustomToolDefinition {
            name: "BrowserClose".into(),
            description: "Close the browser session and stop the browser process.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
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
            return Err("Path traversal (..) is not allowed. Paths must be relative to the home directory (e.g. \"notes.txt\"). To access USER.md or memory files, use MemorySearch/MemoryWrite tools instead.".into());
        }
    }

    // Reject paths starting with .starpod
    let normalized = relative.replace('\\', "/");
    if normalized == ".starpod" || normalized.starts_with(".starpod/") {
        return Err("Cannot access .starpod/ directory — use MemorySearch/MemoryWrite tools for USER.md, MEMORY.md, and memory files.".into());
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
        // --- Bash sandbox guard ---
        // Intercept Bash calls to block access to .starpod/ internals.
        // Returns Some(error) if blocked, None to fall through to the built-in executor.
        "Bash" => {
            if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                // Canonicalize agent_home so we also catch absolute-path references
                let agent_home_canon = ctx.agent_home.canonicalize()
                    .unwrap_or_else(|_| ctx.agent_home.clone());
                let agent_home_str = agent_home_canon.to_string_lossy();

                if command.contains(".starpod") || command.contains(&*agent_home_str) {
                    return Some(ToolResult {
                        content: "Cannot access .starpod/ directory via Bash. Use the dedicated tools instead:\n\
                                  • Memory: MemorySearch, MemoryWrite, MemoryAppendDaily\n\
                                  • Files: FileRead, FileWrite, FileList, FileDelete\n\
                                  • Skills: SkillCreate, SkillUpdate, SkillDelete, SkillList\n\
                                  • Cron: CronAdd, CronList, CronRemove, CronUpdate\n\
                                  • Vault: VaultGet, VaultSet".to_string(),
                        is_error: true,
                        raw_content: None,
                    });
                }
            }
            // Fall through to built-in Bash executor
            None
        }

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
                                "citation": format!("{}#L{}-L{}", r.source, r.line_start, r.line_end),
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

        "MemoryRead" => {
            let file = input.get("file")?.as_str()?;
            let start_line = input.get("start_line").and_then(|v| v.as_u64()).map(|v| v as usize);
            let end_line = input.get("end_line").and_then(|v| v.as_u64()).map(|v| v as usize);

            debug!(file = %file, "MemoryRead");

            let read_result = if let Some(ref uv) = ctx.user_view {
                uv.read_file(file)
            } else {
                ctx.memory.read_file(file)
            };
            match read_result {
                Ok(content) => {
                    let output = match (start_line, end_line) {
                        (Some(start), Some(end)) => {
                            let lines: Vec<&str> = content.lines().collect();
                            let start = start.saturating_sub(1).min(lines.len());
                            let end = end.min(lines.len());
                            lines[start..end].join("\n")
                        }
                        (Some(start), None) => {
                            let lines: Vec<&str> = content.lines().collect();
                            let start = start.saturating_sub(1).min(lines.len());
                            lines[start..].join("\n")
                        }
                        _ => content,
                    };
                    if output.is_empty() {
                        Some(ToolResult {
                            content: format!("File '{}' is empty or does not exist.", file),
                            is_error: false,
                            raw_content: None,
                        })
                    } else {
                        Some(ToolResult {
                            content: output,
                            is_error: false,
                            raw_content: None,
                        })
                    }
                }
                Err(e) => Some(ToolResult {
                    content: format!("Memory read error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "MemoryWrite" => {
            let file = input.get("file")?.as_str()?;
            let content = input.get("content")?.as_str()?;
            let append = input.get("append").and_then(|v| v.as_bool()).unwrap_or(false);

            debug!(file = %file, append = append, "MemoryWrite");

            let final_content = if append {
                // Read existing content and append
                let existing = if let Some(ref uv) = ctx.user_view {
                    uv.read_file(file).unwrap_or_default()
                } else {
                    ctx.memory.read_file(file).unwrap_or_default()
                };
                if existing.is_empty() {
                    content.to_string()
                } else {
                    format!("{}\n{}", existing, content)
                }
            } else {
                content.to_string()
            };

            let write_result = if let Some(ref uv) = ctx.user_view {
                uv.write_file(file, &final_content).await
            } else {
                ctx.memory.write_file(file, &final_content).await
            };
            match write_result {
                Ok(()) => Some(ToolResult {
                    content: if append {
                        format!("Appended to {}", file)
                    } else {
                        format!("Successfully wrote {}", file)
                    },
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

        // --- Web tools ---

        "WebSearch" => {
            if !ctx.internet.enabled {
                return Some(ToolResult {
                    content: "Internet access is disabled in config.".into(),
                    is_error: true,
                    raw_content: None,
                });
            }

            let api_key = match &ctx.brave_api_key {
                Some(k) => k.clone(),
                None => {
                    return Some(ToolResult {
                        content: "BRAVE_API_KEY not set. Add it to .env to enable web search."
                            .into(),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            let query = input.get("query")?.as_str()?;
            let count = input
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .min(20) as u32;

            debug!(query = %query, count = count, "WebSearch");

            match brave_search(&ctx.http_client, &api_key, query, count, ctx.internet.timeout_secs).await {
                Ok(results) => Some(ToolResult {
                    content: results,
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Web search error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "WebFetch" => {
            if !ctx.internet.enabled {
                return Some(ToolResult {
                    content: "Internet access is disabled in config.".into(),
                    is_error: true,
                    raw_content: None,
                });
            }

            let url = input.get("url")?.as_str()?;

            debug!(url = %url, "WebFetch");

            // Block private/local URLs
            if is_private_url(url) {
                return Some(ToolResult {
                    content: "Fetching private/local URLs is not allowed.".into(),
                    is_error: true,
                    raw_content: None,
                });
            }

            match web_fetch(
                &ctx.http_client,
                url,
                ctx.internet.max_fetch_bytes,
                ctx.internet.max_text_chars,
                ctx.internet.timeout_secs,
            )
            .await
            {
                Ok(content) => Some(ToolResult {
                    content,
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Web fetch error: {}", e),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        // --- Browser tools ---
        "BrowserOpen" => {
            let url = input.get("url")?.as_str()?;
            debug!(url = %url, "BrowserOpen");

            if !ctx.browser_enabled {
                return Some(ToolResult {
                    content: "Browser tools are disabled. Enable them in Settings > Browser.".into(),
                    is_error: true,
                    raw_content: None,
                });
            }

            let mut browser_guard = ctx.browser.lock().await;

            // Launch or connect browser if not already running
            if browser_guard.is_none() {
                let result = if let Some(ref cdp_url) = ctx.browser_cdp_url {
                    BrowserSession::connect(cdp_url).await
                } else {
                    BrowserSession::launch().await
                };
                match result {
                    Ok(session) => {
                        *browser_guard = Some(session);
                    }
                    Err(e) => {
                        return Some(ToolResult {
                            content: format!("Failed to launch browser: {}. Make sure 'lightpanda' is installed and on PATH.", e),
                            is_error: true,
                            raw_content: None,
                        });
                    }
                }
            }

            let session = browser_guard.as_ref().unwrap();
            match session.navigate(url).await {
                Ok(title) => Some(ToolResult {
                    content: format!("Navigated to {url}. Page title: \"{title}\""),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => {
                    let msg = e.to_string();
                    // If the connection died (timeout, closed), drop the dead
                    // session so the next BrowserOpen reconnects automatically.
                    if msg.contains("closed") || msg.contains("Timeout") {
                        *browser_guard = None;
                    }
                    Some(ToolResult {
                        content: format!("Navigation failed: {msg}"),
                        is_error: true,
                        raw_content: None,
                    })
                }
            }
        }

        "BrowserClick" => {
            let selector = input.get("selector")?.as_str()?;
            debug!(selector = %selector, "BrowserClick");

            let browser_guard = ctx.browser.lock().await;
            let session = match browser_guard.as_ref() {
                Some(s) => s,
                None => {
                    return Some(ToolResult {
                        content: "No browser session. Use BrowserOpen first.".into(),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            match session.click(selector).await {
                Ok(()) => Some(ToolResult {
                    content: format!("Clicked element: {selector}"),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Click failed: {e}"),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "BrowserType" => {
            let selector = input.get("selector")?.as_str()?;
            let text = input.get("text")?.as_str()?;
            debug!(selector = %selector, "BrowserType");

            let browser_guard = ctx.browser.lock().await;
            let session = match browser_guard.as_ref() {
                Some(s) => s,
                None => {
                    return Some(ToolResult {
                        content: "No browser session. Use BrowserOpen first.".into(),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            match session.type_text(selector, text).await {
                Ok(()) => Some(ToolResult {
                    content: format!("Typed text into: {selector}"),
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Type failed: {e}"),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "BrowserExtract" => {
            let selector = input.get("selector").and_then(|v| v.as_str());
            debug!(selector = ?selector, "BrowserExtract");

            let browser_guard = ctx.browser.lock().await;
            let session = match browser_guard.as_ref() {
                Some(s) => s,
                None => {
                    return Some(ToolResult {
                        content: "No browser session. Use BrowserOpen first.".into(),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            match session.extract(selector).await {
                Ok(text) => Some(ToolResult {
                    content: text,
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("Extract failed: {e}"),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "BrowserEval" => {
            let js = input.get("javascript")?.as_str()?;
            debug!("BrowserEval");

            let browser_guard = ctx.browser.lock().await;
            let session = match browser_guard.as_ref() {
                Some(s) => s,
                None => {
                    return Some(ToolResult {
                        content: "No browser session. Use BrowserOpen first.".into(),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            match session.evaluate(js).await {
                Ok(result) => Some(ToolResult {
                    content: result,
                    is_error: false,
                    raw_content: None,
                }),
                Err(e) => Some(ToolResult {
                    content: format!("JS evaluation failed: {e}"),
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "BrowserWaitFor" => {
            debug!("BrowserWaitFor");
            let browser_guard = ctx.browser.lock().await;
            let session = match browser_guard.as_ref() {
                Some(s) => s,
                None => {
                    return Some(ToolResult {
                        content: "No browser session. Use BrowserOpen first.".into(),
                        is_error: true,
                        raw_content: None,
                    });
                }
            };

            let timeout_ms = input
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(10_000);
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_millis(timeout_ms);

            let result: std::result::Result<String, String> =
                if let Some(url_substr) = input.get("url_contains").and_then(|v| v.as_str()) {
                    // Wait until URL contains substring
                    loop {
                        match session.url().await {
                            Ok(url) if url.contains(url_substr) => {
                                break Ok(format!("URL matched: {url}"));
                            }
                            _ if std::time::Instant::now() > deadline => {
                                break Err(format!(
                                    "Timeout: URL did not contain \"{url_substr}\" within {timeout_ms}ms"
                                ));
                            }
                            _ => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
                        }
                    }
                } else if let Some(sel) = input.get("selector").and_then(|v| v.as_str()) {
                    // Wait until element exists
                    let sel_json = serde_json::to_string(sel).unwrap_or_default();
                    let js = format!("!!document.querySelector({sel_json})");
                    loop {
                        match session.evaluate(&js).await {
                            Ok(ref v) if v == "true" => {
                                break Ok(format!("Element found: {sel}"));
                            }
                            _ if std::time::Instant::now() > deadline => {
                                break Err(format!(
                                    "Timeout: element \"{sel}\" not found within {timeout_ms}ms"
                                ));
                            }
                            _ => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
                        }
                    }
                } else if let Some(js_expr) = input.get("javascript").and_then(|v| v.as_str()) {
                    // Wait until JS expression is truthy
                    loop {
                        match session.evaluate(js_expr).await {
                            Ok(ref v) if !v.is_empty() && v != "false" && v != "null" && v != "0" => {
                                break Ok(format!("Condition met: {v}"));
                            }
                            _ if std::time::Instant::now() > deadline => {
                                break Err(format!(
                                    "Timeout: JS condition not met within {timeout_ms}ms"
                                ));
                            }
                            _ => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
                        }
                    }
                } else {
                    Err("Provide one of: url_contains, selector, or javascript".into())
                };

            match result {
                Ok(msg) => Some(ToolResult {
                    content: msg,
                    is_error: false,
                    raw_content: None,
                }),
                Err(msg) => Some(ToolResult {
                    content: msg,
                    is_error: true,
                    raw_content: None,
                }),
            }
        }

        "BrowserClose" => {
            debug!("BrowserClose");
            let mut browser_guard = ctx.browser.lock().await;
            match browser_guard.take() {
                Some(session) => {
                    match session.close().await {
                        Ok(()) => Some(ToolResult {
                            content: "Browser session closed.".into(),
                            is_error: false,
                            raw_content: None,
                        }),
                        Err(e) => Some(ToolResult {
                            content: format!("Close error: {e}"),
                            is_error: true,
                            raw_content: None,
                        }),
                    }
                }
                None => Some(ToolResult {
                    content: "No browser session to close.".into(),
                    is_error: false,
                    raw_content: None,
                }),
            }
        }

        _ => None,
    }
}

// ── Web tool helpers ─────────────────────────────────────────────────────────

/// Call the Brave Search API and format results as numbered plain-text entries.
///
/// Each result is formatted as:
/// ```text
/// 1. Title
///    https://example.com/page
///    Description snippet from the search engine
/// ```
///
/// Returns `"No results found."` when the response contains no web results.
async fn brave_search(
    client: &Client,
    api_key: &str,
    query: &str,
    count: u32,
    timeout_secs: u64,
) -> Result<String, String> {
    let resp = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", &count.to_string())])
        .timeout(Duration::from_secs(timeout_secs))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Brave API returned {}: {}", status, body));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(format_brave_results(&body))
}

/// Format a Brave Search API JSON response into numbered plain-text results.
///
/// Expects the standard Brave response structure: `{ "web": { "results": [...] } }`.
/// Each result should have `title`, `url`, and `description` fields.
fn format_brave_results(body: &serde_json::Value) -> String {
    let mut output = String::new();
    if let Some(results) =
        body.get("web").and_then(|w| w.get("results")).and_then(|r| r.as_array())
    {
        if results.is_empty() {
            return "No results found.".into();
        }
        for (i, result) in results.iter().enumerate() {
            let title = result
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("(no title)");
            let url = result.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let description = result
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("(no description)");

            output.push_str(&format!(
                "{}. {}\n   {}\n   {}\n\n",
                i + 1,
                title,
                url,
                description
            ));
        }
    } else {
        return "No results found.".into();
    }

    output.trim_end().to_string()
}

/// Strip invisible and non-content HTML elements using `lol_html`.
///
/// Removes `<script>`, `<style>`, `<noscript>`, `<svg>`, `<canvas>`, `<iframe>`,
/// `<meta>`, `<head>`, `<link>`, `<template>`, `<object>`, `<embed>` elements,
/// HTML comments, and elements with `hidden`, `aria-hidden="true"`, or inline
/// `display:none` / `visibility:hidden` styles.
///
/// This is designed to run *before* readability extraction to shrink the HTML
/// and remove noise that confuses content-detection heuristics.
fn strip_invisible_html(html: &str) -> String {
    const REMOVE_TAGS: &[&str] = &[
        "script", "style", "noscript", "svg", "canvas", "iframe", "meta",
        "head", "link", "template", "object", "embed",
    ];

    let result = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                // Remove entire elements for non-content tags.
                element!(
                    &REMOVE_TAGS
                        .iter()
                        .copied()
                        .collect::<Vec<_>>()
                        .join(","),
                    |el| {
                        el.remove();
                        Ok(())
                    }
                ),
                // Remove elements hidden via attributes or inline styles.
                element!("*", |el| {
                    // hidden attribute
                    if el.has_attribute("hidden") {
                        el.remove();
                        return Ok(());
                    }
                    // aria-hidden="true"
                    if el
                        .get_attribute("aria-hidden")
                        .map_or(false, |v| v.trim() == "true")
                    {
                        el.remove();
                        return Ok(());
                    }
                    // inline style: display:none or visibility:hidden
                    if let Some(style) = el.get_attribute("style") {
                        let s = style.to_lowercase();
                        let s = s.replace(' ', "");
                        if s.contains("display:none") || s.contains("visibility:hidden") {
                            el.remove();
                            return Ok(());
                        }
                    }
                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    );

    result.unwrap_or_else(|_| html.to_string())
}

/// Extract readable content from HTML using readability + markdown conversion.
///
/// Pipeline: strip invisible elements → readability extraction → markdown
/// conversion → blank-line collapsing. Falls back to stripped-HTML-to-markdown
/// if readability returns fewer than 200 characters.
fn extract_readable_content(html: &str, url: &str) -> String {
    let stripped = strip_invisible_html(html);

    // Try readability extraction on the stripped HTML.
    let readable_text = {
        let parsed_url = Url::parse(url)
            .unwrap_or_else(|_| Url::parse("https://example.com").unwrap());
        readability::extractor::extract(&mut stripped.as_bytes(), &parsed_url)
            .ok()
            .map(|p| p.content)
    };

    // Use readability output if substantial, otherwise fall back to stripped HTML.
    let source_html = match &readable_text {
        Some(content) if content.len() >= 200 => content.as_str(),
        _ => &stripped,
    };

    // Convert to markdown and collapse blank lines.
    let md = htmd::convert(source_html).unwrap_or_else(|_| source_html.to_string());
    collapse_blank_lines(&md)
}

/// Collapse runs of more than 2 consecutive blank lines and trim trailing whitespace.
fn collapse_blank_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut blank_count = 0u32;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(trimmed);
            result.push('\n');
        }
    }
    result.trim().to_string()
}

/// Truncate a string at a char boundary, appending a marker if truncated.
fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let boundary = text
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let mut truncated = text[..boundary].to_string();
    truncated.push_str(&format!(
        "\n\n[Content truncated at {} characters — original was {} characters]",
        max_chars,
        text.chars().count()
    ));
    truncated
}

/// Fetch a web page and convert its content to a readable format.
///
/// For HTML pages (`text/html`, `application/xhtml`), the body is run through
/// readability extraction (strip invisible elements → extract main content →
/// convert to markdown). For other content types (JSON, plain text, etc.), the
/// raw body is returned as-is.
///
/// Two-stage truncation:
/// 1. Raw HTTP body is capped at `max_bytes` (before parsing)
/// 2. Extracted text is capped at `max_text_chars` (after extraction)
async fn web_fetch(
    client: &Client,
    url: &str,
    max_bytes: usize,
    max_text_chars: usize,
    timeout_secs: u64,
) -> Result<String, String> {
    let resp = client
        .get(url)
        .header("User-Agent", "Starpod/1.0 (AI Assistant)")
        .timeout(Duration::from_secs(timeout_secs))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read body: {}", e))?;

    let body_str = if body_bytes.len() > max_bytes {
        String::from_utf8_lossy(&body_bytes[..max_bytes]).into_owned()
    } else {
        String::from_utf8_lossy(&body_bytes).into_owned()
    };

    let text = if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        extract_readable_content(&body_str, url)
    } else {
        body_str
    };

    Ok(truncate_text(&text, max_text_chars))
}

/// Check if a URL points to a private/local network address.
///
/// Blocks requests to localhost, loopback, RFC 1918 private ranges
/// (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16), and `.local`/`.internal`
/// TLD suffixes. This is a defence-in-depth measure to prevent SSRF when
/// the agent fetches arbitrary URLs provided by the LLM.
fn is_private_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let host = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
        .unwrap_or(&lower);
    let host = host.split('/').next().unwrap_or(host);
    let host = if host.starts_with('[') {
        host.split(']').next().unwrap_or(host).trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or(host)
    };

    host == "localhost"
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || host == "::1"
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("172.16.")
        || host.starts_with("172.17.")
        || host.starts_with("172.18.")
        || host.starts_with("172.19.")
        || host.starts_with("172.2")
        || host.starts_with("172.30.")
        || host.starts_with("172.31.")
        || host.ends_with(".local")
        || host.ends_with(".internal")
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            agent_home: tmp.path().join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            agent_home: tmp.path().join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            agent_home: tmp.path().join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: home_dir.clone(),
            agent_home: home_dir.join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: home_dir.clone(),
            agent_home: home_dir.join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: home_dir.clone(),
            agent_home: home_dir.join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: home_dir.clone(),
            agent_home: home_dir.join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: home_dir.clone(),
            agent_home: home_dir.join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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
            .await
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
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            agent_home: tmp.path().join(".starpod"),
            user_id: Some("alice".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
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

    // ── MemoryRead tests ────────────────────────────────────────────

    #[tokio::test]
    async fn memory_read_returns_file_content() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // Write a user file first
        handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "USER.md", "content": "# User\nAlice is a developer.\nShe likes Rust."}),
        ).await.unwrap();

        // Read it back
        let result = handle_custom_tool(
            &ctx,
            "MemoryRead",
            &serde_json::json!({"file": "USER.md"}),
        ).await.unwrap();
        assert!(!result.is_error, "MemoryRead should succeed: {}", result.content);
        assert!(result.content.contains("Alice is a developer"));
        assert!(result.content.contains("She likes Rust"));
    }

    #[tokio::test]
    async fn memory_read_with_line_range() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // Write a multi-line file
        handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({
                "file": "USER.md",
                "content": "Line 1\nLine 2\nLine 3\nLine 4\nLine 5"
            }),
        ).await.unwrap();

        // Read lines 2-4
        let result = handle_custom_tool(
            &ctx,
            "MemoryRead",
            &serde_json::json!({"file": "USER.md", "start_line": 2, "end_line": 4}),
        ).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Line 2"));
        assert!(result.content.contains("Line 4"));
        assert!(!result.content.contains("Line 1"), "Should not contain lines before start_line");
        assert!(!result.content.contains("Line 5"), "Should not contain lines after end_line");
    }

    #[tokio::test]
    async fn memory_read_with_start_line_only() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "USER.md", "content": "Line 1\nLine 2\nLine 3"}),
        ).await.unwrap();

        // Read from line 2 onward
        let result = handle_custom_tool(
            &ctx,
            "MemoryRead",
            &serde_json::json!({"file": "USER.md", "start_line": 2}),
        ).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Line 2"));
        assert!(result.content.contains("Line 3"));
        assert!(!result.content.contains("Line 1"));
    }

    #[tokio::test]
    async fn memory_read_empty_file() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // Read a file that doesn't exist — read_file returns empty string
        let result = handle_custom_tool(
            &ctx,
            "MemoryRead",
            &serde_json::json!({"file": "nonexistent.md"}),
        ).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("empty") || result.content.contains("does not exist"));
    }

    #[tokio::test]
    async fn memory_read_routes_soul_to_agent_store() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // SOUL.md should come from agent store
        let result = handle_custom_tool(
            &ctx,
            "MemoryRead",
            &serde_json::json!({"file": "SOUL.md"}),
        ).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Aster"), "SOUL.md should come from agent store");
    }

    // ── MemoryWrite append mode tests ───────────────────────────────

    #[tokio::test]
    async fn memory_write_append_mode() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // Write initial content
        handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "MEMORY.md", "content": "# Memory\n\nFirst entry."}),
        ).await.unwrap();

        // Append more content
        let result = handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "MEMORY.md", "content": "Second entry.", "append": true}),
        ).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Appended"), "Should report append");

        // Verify both entries are present
        let read = handle_custom_tool(
            &ctx,
            "MemoryRead",
            &serde_json::json!({"file": "MEMORY.md"}),
        ).await.unwrap();
        assert!(read.content.contains("First entry"), "Original content preserved");
        assert!(read.content.contains("Second entry"), "Appended content present");
    }

    #[tokio::test]
    async fn memory_write_append_to_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // Append to a file that doesn't exist yet
        let result = handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "memory/notes.md", "content": "New note.", "append": true}),
        ).await.unwrap();
        assert!(!result.is_error);

        // Verify content
        let read = handle_custom_tool(
            &ctx,
            "MemoryRead",
            &serde_json::json!({"file": "memory/notes.md"}),
        ).await.unwrap();
        assert!(read.content.contains("New note"));
    }

    #[tokio::test]
    async fn memory_write_overwrite_is_default() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // Write twice without append flag
        handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "MEMORY.md", "content": "Version 1"}),
        ).await.unwrap();
        handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "MEMORY.md", "content": "Version 2"}),
        ).await.unwrap();

        let read = handle_custom_tool(
            &ctx,
            "MemoryRead",
            &serde_json::json!({"file": "MEMORY.md"}),
        ).await.unwrap();
        assert!(!read.content.contains("Version 1"), "Old content should be overwritten");
        assert!(read.content.contains("Version 2"), "New content should be present");
    }

    // ── MemorySearch citation tests ─────────────────────────────────

    #[tokio::test]
    async fn memory_search_results_include_citations() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_with_user_view(&tmp).await;

        // Write searchable content
        handle_custom_tool(
            &ctx,
            "MemoryWrite",
            &serde_json::json!({"file": "MEMORY.md", "content": "# Memory\n\nAlice prefers dark mode."}),
        ).await.unwrap();

        // Search
        let result = handle_custom_tool(
            &ctx,
            "MemorySearch",
            &serde_json::json!({"query": "dark mode"}),
        ).await.unwrap();
        assert!(!result.is_error);

        // Parse result and verify citation field
        let results: Vec<serde_json::Value> = serde_json::from_str(&result.content).unwrap();
        assert!(!results.is_empty(), "Should find results");
        for r in &results {
            assert!(r.get("citation").is_some(), "Each result should have a citation field");
            let citation = r["citation"].as_str().unwrap();
            assert!(citation.contains("#L"), "Citation should include line reference: {}", citation);
        }
    }

    // ── Bash sandbox guard ─────────────────────────────────────────

    async fn bash_ctx(tmp: &TempDir) -> ToolContext {
        let memory = Arc::new(
            starpod_memory::MemoryStore::new(
                &tmp.path().join("agent"),
                &tmp.path().join("agent").join("config"),
                &tmp.path().join("db"),
            ).await.unwrap(),
        );
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: tmp.path().join("home"),
            agent_home: tmp.path().join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
        }
    }

    #[tokio::test]
    async fn bash_blocks_starpod_dir_reference() {
        let tmp = TempDir::new().unwrap();
        let ctx = bash_ctx(&tmp).await;

        let result = handle_custom_tool(
            &ctx,
            "Bash",
            &serde_json::json!({"command": "cat .starpod/config/agent.toml"}),
        ).await;

        let result = result.expect("Should return Some for blocked command");
        assert!(result.is_error);
        assert!(result.content.contains("Cannot access .starpod/"));
    }

    #[tokio::test]
    async fn bash_blocks_starpod_in_piped_command() {
        let tmp = TempDir::new().unwrap();
        let ctx = bash_ctx(&tmp).await;

        let result = handle_custom_tool(
            &ctx,
            "Bash",
            &serde_json::json!({"command": "ls -la | grep something && cat .starpod/db/memory.db"}),
        ).await;

        let result = result.expect("Should return Some for blocked command");
        assert!(result.is_error);
        assert!(result.content.contains("Cannot access .starpod/"));
    }

    #[tokio::test]
    async fn bash_blocks_absolute_agent_home_path() {
        let tmp = TempDir::new().unwrap();
        // Create the .starpod dir so canonicalization works
        std::fs::create_dir_all(tmp.path().join(".starpod")).unwrap();
        let ctx = bash_ctx(&tmp).await;

        let abs_path = tmp.path().join(".starpod").canonicalize().unwrap();
        let command = format!("cat {}/config/agent.toml", abs_path.display());

        let result = handle_custom_tool(
            &ctx,
            "Bash",
            &serde_json::json!({"command": command}),
        ).await;

        let result = result.expect("Should return Some for blocked command");
        assert!(result.is_error);
        assert!(result.content.contains("Cannot access .starpod/"));
    }

    #[tokio::test]
    async fn bash_allows_normal_commands() {
        let tmp = TempDir::new().unwrap();
        let ctx = bash_ctx(&tmp).await;

        // Normal commands should fall through (return None)
        let result = handle_custom_tool(
            &ctx,
            "Bash",
            &serde_json::json!({"command": "echo hello && ls -la"}),
        ).await;

        assert!(result.is_none(), "Normal commands should fall through to built-in executor");
    }

    #[tokio::test]
    async fn bash_allows_commands_with_starpod_in_string_content() {
        let tmp = TempDir::new().unwrap();
        let ctx = bash_ctx(&tmp).await;

        // "starpod" without the dot prefix should be allowed
        let result = handle_custom_tool(
            &ctx,
            "Bash",
            &serde_json::json!({"command": "echo 'starpod is great'"}),
        ).await;

        assert!(result.is_none(), "Commands mentioning 'starpod' (without dot) should pass");
    }

    #[tokio::test]
    async fn bash_blocks_starpod_with_find_command() {
        let tmp = TempDir::new().unwrap();
        let ctx = bash_ctx(&tmp).await;

        let result = handle_custom_tool(
            &ctx,
            "Bash",
            &serde_json::json!({"command": "find .starpod -name '*.toml'"}),
        ).await;

        let result = result.expect("Should return Some for blocked command");
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn bash_error_message_suggests_tools() {
        let tmp = TempDir::new().unwrap();
        let ctx = bash_ctx(&tmp).await;

        let result = handle_custom_tool(
            &ctx,
            "Bash",
            &serde_json::json!({"command": "ls .starpod/"}),
        ).await.unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("MemorySearch"), "Should suggest MemorySearch");
        assert!(result.content.contains("FileRead"), "Should suggest FileRead");
        assert!(result.content.contains("SkillCreate"), "Should suggest SkillCreate");
        assert!(result.content.contains("CronAdd"), "Should suggest CronAdd");
    }

    // ── Browser tool handler tests ──────────────────────────────────

    async fn browser_ctx(tmp: &TempDir) -> ToolContext {
        let memory = Arc::new(
            starpod_memory::MemoryStore::new(
                &tmp.path().join("agent"),
                &tmp.path().join("agent").join("config"),
                &tmp.path().join("db"),
            ).await.unwrap(),
        );
        let skills = Arc::new(starpod_skills::SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(starpod_cron::CronStore::new(&tmp.path().join("cron.db")).await.unwrap());

        ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: true,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            agent_home: tmp.path().join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
        }
    }

    #[tokio::test]
    async fn browser_click_without_session_returns_error() {
        let tmp = TempDir::new().unwrap();
        let ctx = browser_ctx(&tmp).await;
        let result = handle_custom_tool(&ctx, "BrowserClick", &serde_json::json!({"selector": "button"}))
            .await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("No browser session"));
    }

    #[tokio::test]
    async fn browser_type_without_session_returns_error() {
        let tmp = TempDir::new().unwrap();
        let ctx = browser_ctx(&tmp).await;
        let result = handle_custom_tool(&ctx, "BrowserType", &serde_json::json!({"selector": "input", "text": "hello"}))
            .await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("No browser session"));
    }

    #[tokio::test]
    async fn browser_extract_without_session_returns_error() {
        let tmp = TempDir::new().unwrap();
        let ctx = browser_ctx(&tmp).await;
        let result = handle_custom_tool(&ctx, "BrowserExtract", &serde_json::json!({}))
            .await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("No browser session"));
    }

    #[tokio::test]
    async fn browser_eval_without_session_returns_error() {
        let tmp = TempDir::new().unwrap();
        let ctx = browser_ctx(&tmp).await;
        let result = handle_custom_tool(&ctx, "BrowserEval", &serde_json::json!({"javascript": "1+1"}))
            .await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("No browser session"));
    }

    #[tokio::test]
    async fn browser_close_without_session_is_not_error() {
        let tmp = TempDir::new().unwrap();
        let ctx = browser_ctx(&tmp).await;
        let result = handle_custom_tool(&ctx, "BrowserClose", &serde_json::json!({}))
            .await.unwrap();
        assert!(!result.is_error, "BrowserClose with no session should not error");
        assert!(result.content.contains("No browser session to close"));
    }

    #[tokio::test]
    async fn browser_click_missing_selector_returns_none() {
        let tmp = TempDir::new().unwrap();
        let ctx = browser_ctx(&tmp).await;
        let result = handle_custom_tool(&ctx, "BrowserClick", &serde_json::json!({})).await;
        assert!(result.is_none(), "missing required field should return None");
    }

    #[tokio::test]
    async fn browser_open_missing_url_returns_none() {
        let tmp = TempDir::new().unwrap();
        let ctx = browser_ctx(&tmp).await;
        let result = handle_custom_tool(&ctx, "BrowserOpen", &serde_json::json!({})).await;
        assert!(result.is_none(), "missing required url should return None");
    }

    // ── is_private_url ────────────────────────────────────────────────

    #[test]
    fn private_url_blocks_localhost() {
        assert!(is_private_url("http://localhost/foo"));
        assert!(is_private_url("https://localhost:8080/bar"));
        assert!(is_private_url("http://LOCALHOST/baz"));
    }

    #[test]
    fn private_url_blocks_loopback() {
        assert!(is_private_url("http://127.0.0.1/"));
        assert!(is_private_url("http://127.0.0.1:3000/api"));
        assert!(is_private_url("http://0.0.0.0/"));
        assert!(is_private_url("http://[::1]/"));
    }

    #[test]
    fn private_url_blocks_rfc1918_class_a() {
        assert!(is_private_url("http://10.0.0.1/"));
        assert!(is_private_url("http://10.255.255.255/page"));
    }

    #[test]
    fn private_url_blocks_rfc1918_class_b() {
        assert!(is_private_url("http://172.16.0.1/"));
        assert!(is_private_url("http://172.20.10.5/"));
        assert!(is_private_url("http://172.31.255.255/"));
    }

    #[test]
    fn private_url_blocks_rfc1918_class_c() {
        assert!(is_private_url("http://192.168.0.1/"));
        assert!(is_private_url("http://192.168.1.100:8080/api"));
    }

    #[test]
    fn private_url_blocks_local_tld() {
        assert!(is_private_url("http://mydevbox.local/"));
        assert!(is_private_url("https://service.internal/api"));
    }

    #[test]
    fn private_url_allows_public_urls() {
        assert!(!is_private_url("https://example.com/"));
        assert!(!is_private_url("https://api.brave.com/search"));
        assert!(!is_private_url("http://8.8.8.8/dns"));
        assert!(!is_private_url("https://docs.rs/reqwest"));
    }

    #[test]
    fn private_url_allows_non_private_172() {
        assert!(!is_private_url("http://172.32.0.1/"));
        assert!(!is_private_url("http://172.15.0.1/"));
    }

    // ── strip_invisible_html ─────────────────────────────────────────

    #[test]
    fn strip_invisible_removes_script_and_style() {
        let html = r#"<html><head><style>body{color:red}</style></head><body>
            <script>alert('xss')</script><p>Hello world</p></body></html>"#;
        let result = strip_invisible_html(html);
        assert!(!result.contains("alert"));
        assert!(!result.contains("color:red"));
        assert!(result.contains("Hello world"));
    }

    #[test]
    fn strip_invisible_removes_hidden_elements() {
        let html = r#"<div>Visible</div>
            <div hidden>Hidden attr</div>
            <div aria-hidden="true">Aria hidden</div>
            <div style="display:none">Display none</div>
            <div style="visibility: hidden">Vis hidden</div>"#;
        let result = strip_invisible_html(html);
        assert!(result.contains("Visible"));
        assert!(!result.contains("Hidden attr"));
        assert!(!result.contains("Aria hidden"));
        assert!(!result.contains("Display none"));
        assert!(!result.contains("Vis hidden"));
    }

    #[test]
    fn strip_invisible_removes_non_content_tags() {
        let html = r#"<body>
            <svg><circle r="50"/></svg>
            <canvas></canvas>
            <iframe src="ads.html"></iframe>
            <noscript>Enable JS</noscript>
            <p>Content</p></body>"#;
        let result = strip_invisible_html(html);
        assert!(!result.contains("circle"));
        assert!(!result.contains("canvas"));
        assert!(!result.contains("ads.html"));
        assert!(!result.contains("Enable JS"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn strip_invisible_removes_html_comments() {
        let html = "<p>Before</p><!-- secret comment --><p>After</p>";
        let result = strip_invisible_html(html);
        assert!(!result.contains("secret comment"));
        assert!(result.contains("Before"));
        assert!(result.contains("After"));
    }

    #[test]
    fn strip_invisible_preserves_clean_html() {
        let html = "<article><h1>Title</h1><p>Paragraph text.</p></article>";
        let result = strip_invisible_html(html);
        assert!(result.contains("Title"));
        assert!(result.contains("Paragraph text."));
    }

    #[test]
    fn strip_invisible_removes_nested_hidden_elements() {
        let html = r#"<div hidden><p>Hidden parent<span>and nested child</span></p></div>
            <p>Visible</p>"#;
        let result = strip_invisible_html(html);
        assert!(!result.contains("Hidden parent"));
        assert!(!result.contains("nested child"));
        assert!(result.contains("Visible"));
    }

    // ── extract_readable_content ──────────────────────────────────────

    #[test]
    fn extract_readable_content_from_article_page() {
        let html = format!(
            r#"<html><head><title>Test</title><style>*{{margin:0}}</style></head>
            <body>
            <nav><a href="/">Home</a><a href="/about">About</a></nav>
            <article><h1>Main Article</h1><p>{}</p></article>
            <footer>Copyright 2024</footer>
            </body></html>"#,
            "This is the main article content. ".repeat(20)
        );
        let result = extract_readable_content(&html, "https://example.com/article");
        // Readability extracts body text; the h1 may be pulled into title
        assert!(result.contains("main article content"));
        // Style should be stripped
        assert!(!result.contains("margin:0"));
        // Nav and footer should be stripped by readability
        assert!(!result.contains("Copyright 2024"));
    }

    #[test]
    fn extract_readable_content_fallback_on_short_readability() {
        // Minimal HTML where readability likely returns very little
        let html = "<html><body><p>Short.</p></body></html>";
        let result = extract_readable_content(html, "https://example.com");
        // Should still return something (fallback to stripped HTML)
        assert!(result.contains("Short."));
    }

    #[test]
    fn extract_readable_content_handles_malformed_html() {
        // Severely broken HTML — unclosed tags, mismatched nesting
        let html = "<div><p>Unclosed paragraph<span>broken<div>nesting</p></span>";
        let result = extract_readable_content(html, "https://example.com");
        // Should not panic, should return something
        assert!(!result.is_empty());
    }

    #[test]
    fn extract_readable_content_handles_invalid_url() {
        let html = format!(
            "<html><body><article><p>{}</p></article></body></html>",
            "Content here. ".repeat(30)
        );
        // Invalid URL — should fallback to dummy URL without panicking
        let result = extract_readable_content(&html, "not a valid url at all");
        assert!(!result.is_empty());
        assert!(result.contains("Content here"));
    }

    #[test]
    fn extract_readable_content_handles_empty_html() {
        let result = extract_readable_content("", "https://example.com");
        // Should not panic on empty input
        assert!(result.is_empty() || result.len() < 50);
    }

    #[test]
    fn collapse_blank_lines_limits_to_two() {
        let input = "Line 1\n\n\n\n\nLine 2\n\nLine 3";
        let result = collapse_blank_lines(input);
        assert_eq!(result, "Line 1\n\n\nLine 2\n\nLine 3");
    }

    // ── truncate_text ─────────────────────────────────────────────────

    #[test]
    fn truncate_text_no_op_when_within_limit() {
        let text = "Hello, world!";
        let result = truncate_text(text, 100);
        assert_eq!(result, "Hello, world!");
        assert!(!result.contains("[Content truncated"));
    }

    #[test]
    fn truncate_text_truncates_at_char_boundary() {
        let text = "a".repeat(200);
        let result = truncate_text(&text, 50);
        assert!(result.starts_with(&"a".repeat(50)));
        assert!(result.contains("[Content truncated at 50 characters"));
        assert!(result.contains("original was 200 characters"));
    }

    #[test]
    fn truncate_text_handles_multibyte_chars() {
        // 10 emoji, each 1 char but multiple bytes
        let text = "🎉".repeat(10);
        let result = truncate_text(&text, 5);
        assert_eq!(result.chars().take(5).collect::<String>(), "🎉".repeat(5));
        assert!(result.contains("[Content truncated at 5 characters"));
    }

    // ── format_brave_results ──────────────────────────────────────────

    #[test]
    fn format_brave_results_with_results() {
        let body = json!({
            "web": {
                "results": [
                    {
                        "title": "Rust Programming Language",
                        "url": "https://www.rust-lang.org/",
                        "description": "A language empowering everyone to build reliable software."
                    },
                    {
                        "title": "Rust Documentation",
                        "url": "https://doc.rust-lang.org/",
                        "description": "Official Rust documentation and guides."
                    }
                ]
            }
        });

        let output = format_brave_results(&body);
        assert!(output.contains("1. Rust Programming Language"));
        assert!(output.contains("https://www.rust-lang.org/"));
        assert!(output.contains("2. Rust Documentation"));
    }

    #[test]
    fn format_brave_results_empty_results() {
        let body = json!({ "web": { "results": [] } });
        assert_eq!(format_brave_results(&body), "No results found.");
    }

    #[test]
    fn format_brave_results_missing_web_key() {
        let body = json!({ "query": { "original": "test" } });
        assert_eq!(format_brave_results(&body), "No results found.");
    }

    #[test]
    fn format_brave_results_missing_fields_in_result() {
        let body = json!({ "web": { "results": [{ "title": "Only Title" }] } });
        let output = format_brave_results(&body);
        assert!(output.contains("1. Only Title"));
        assert!(output.contains("(no description)"));
    }

    // ── WebSearch handler ─────────────────────────────────────────────

    async fn web_tool_context(tmp: &TempDir) -> ToolContext {
        let memory = Arc::new(
            MemoryStore::new(tmp.path(), &tmp.path().join("config"), &tmp.path().join("db"))
                .await.unwrap(),
        );
        let skills = Arc::new(SkillStore::new(&tmp.path().join("skills")).unwrap());
        let cron = Arc::new(CronStore::new(&tmp.path().join("cron.db")).await.unwrap());
        ToolContext {
            memory,
            user_view: None,
            skills,
            cron,
            browser: Arc::new(tokio::sync::Mutex::new(None)),
            browser_enabled: false,
            browser_cdp_url: None,
            user_tz: None,
            home_dir: tmp.path().to_path_buf(),
            agent_home: tmp.path().join(".starpod"),
            user_id: Some("admin".into()),
            http_client: Client::new(),
            internet: InternetConfig::default(),
            brave_api_key: None,
        }
    }

    #[tokio::test]
    async fn web_search_errors_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = web_tool_context(&tmp).await;
        ctx.internet.enabled = false;
        let result = handle_custom_tool(&ctx, "WebSearch", &json!({"query": "rust"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("disabled"));
    }

    #[tokio::test]
    async fn web_search_errors_when_no_api_key() {
        let tmp = TempDir::new().unwrap();
        let ctx = web_tool_context(&tmp).await;
        let result = handle_custom_tool(&ctx, "WebSearch", &json!({"query": "rust"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("BRAVE_API_KEY"));
    }

    #[tokio::test]
    async fn web_search_returns_none_for_missing_query() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = web_tool_context(&tmp).await;
        ctx.brave_api_key = Some("test-key".into());
        let result = handle_custom_tool(&ctx, "WebSearch", &json!({})).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn web_fetch_errors_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = web_tool_context(&tmp).await;
        ctx.internet.enabled = false;
        let result = handle_custom_tool(&ctx, "WebFetch", &json!({"url": "https://example.com"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("disabled"));
    }

    #[tokio::test]
    async fn web_fetch_blocks_private_urls() {
        let tmp = TempDir::new().unwrap();
        let ctx = web_tool_context(&tmp).await;
        for url in &["http://localhost/secret", "http://127.0.0.1:8080/api", "http://10.0.0.1/internal",
                     "http://192.168.1.1/admin", "http://172.16.0.1/", "http://mybox.local/"] {
            let result = handle_custom_tool(&ctx, "WebFetch", &json!({"url": url})).await.unwrap();
            assert!(result.is_error, "Should block private URL: {}", url);
            assert!(result.content.contains("private/local"), "for: {}", url);
        }
    }

    #[tokio::test]
    async fn web_fetch_returns_none_for_missing_url() {
        let tmp = TempDir::new().unwrap();
        let ctx = web_tool_context(&tmp).await;
        let result = handle_custom_tool(&ctx, "WebFetch", &json!({})).await;
        assert!(result.is_none());
    }
}
