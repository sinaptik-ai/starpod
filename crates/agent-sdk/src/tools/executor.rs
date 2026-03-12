//! Built-in tool execution engine.
//!
//! Provides [`ToolExecutor`] which can run the core built-in tools:
//! Read, Write, Edit, Bash, Glob, and Grep.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use glob::glob as glob_match;
use regex::{Regex, RegexBuilder};
use serde_json::Value;
use tokio::process::Command;
use tracing::{debug, warn};

use crate::error::{AgentError, Result};
use crate::types::tools::{
    BashInput, FileEditInput, FileReadInput, FileWriteInput, GlobInput, GrepInput, GrepOutputMode,
};

/// Result of executing a built-in tool.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// The textual output of the tool.
    pub content: String,
    /// Whether the result represents an error condition.
    pub is_error: bool,
}

impl ToolResult {
    /// Create a successful result.
    fn ok(content: String) -> Self {
        Self {
            content,
            is_error: false,
        }
    }

    /// Create an error result.
    fn err(content: String) -> Self {
        Self {
            content,
            is_error: true,
        }
    }
}

/// Executes built-in tools (Read, Write, Edit, Bash, Glob, Grep).
pub struct ToolExecutor {
    /// Working directory for relative path resolution and command execution.
    cwd: PathBuf,
}

impl ToolExecutor {
    /// Create a new executor rooted at the given working directory.
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }

    /// Dispatch a tool call by name, deserializing `input` into the appropriate typed input.
    pub async fn execute(&self, tool_name: &str, input: Value) -> Result<ToolResult> {
        debug!(tool = tool_name, "executing built-in tool");

        match tool_name {
            "Read" => {
                let params: FileReadInput = serde_json::from_value(input)?;
                self.execute_read(&params).await
            }
            "Write" => {
                let params: FileWriteInput = serde_json::from_value(input)?;
                self.execute_write(&params).await
            }
            "Edit" => {
                let params: FileEditInput = serde_json::from_value(input)?;
                self.execute_edit(&params).await
            }
            "Bash" => {
                let params: BashInput = serde_json::from_value(input)?;
                self.execute_bash(&params).await
            }
            "Glob" => {
                let params: GlobInput = serde_json::from_value(input)?;
                self.execute_glob(&params).await
            }
            "Grep" => {
                let params: GrepInput = serde_json::from_value(input)?;
                self.execute_grep(&params).await
            }
            _ => Err(AgentError::ToolExecution(format!(
                "unsupported built-in tool: {tool_name}"
            ))),
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    /// Resolve a path that may be relative against the executor's cwd.
    fn resolve_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.cwd.join(p)
        }
    }

    // ── Read ────────────────────────────────────────────────────────────

    /// Read a file's contents, optionally slicing by line offset and limit.
    /// Output uses `cat -n` style (line-number prefixed) formatting.
    async fn execute_read(&self, input: &FileReadInput) -> Result<ToolResult> {
        let path = self.resolve_path(&input.file_path);

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::err(format!(
                    "Failed to read {}: {e}",
                    path.display()
                )));
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        let offset = input.offset.unwrap_or(0) as usize;
        let limit = input.limit.unwrap_or(total as u64) as usize;

        if offset >= total {
            return Ok(ToolResult::ok(String::new()));
        }

        let end = (offset + limit).min(total);
        let selected = &lines[offset..end];

        // Format like `cat -n`: right-aligned line numbers followed by a tab and the line.
        let width = format!("{}", end).len();
        let mut output = String::new();
        for (i, line) in selected.iter().enumerate() {
            let line_no = offset + i + 1; // 1-based
            output.push_str(&format!("{line_no:>width$}\t{line}\n", width = width));
        }

        Ok(ToolResult::ok(output))
    }

    // ── Write ───────────────────────────────────────────────────────────

    /// Write content to a file, creating parent directories as needed.
    async fn execute_write(&self, input: &FileWriteInput) -> Result<ToolResult> {
        let path = self.resolve_path(&input.file_path);

        // Ensure parent directories exist.
        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult::err(format!(
                    "Failed to create directories for {}: {e}",
                    path.display()
                )));
            }
        }

        match tokio::fs::write(&path, &input.content).await {
            Ok(()) => Ok(ToolResult::ok(format!(
                "Successfully wrote to {}",
                path.display()
            ))),
            Err(e) => Ok(ToolResult::err(format!(
                "Failed to write {}: {e}",
                path.display()
            ))),
        }
    }

    // ── Edit ────────────────────────────────────────────────────────────

    /// Perform an exact string replacement in a file.
    ///
    /// - If `old_string` is not found, returns an error result.
    /// - If `old_string` appears more than once and `replace_all` is not set, returns an error
    ///   asking for more context to make the match unique.
    async fn execute_edit(&self, input: &FileEditInput) -> Result<ToolResult> {
        let path = self.resolve_path(&input.file_path);

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::err(format!(
                    "Failed to read {}: {e}",
                    path.display()
                )));
            }
        };

        let replace_all = input.replace_all.unwrap_or(false);
        let count = content.matches(&input.old_string).count();

        if count == 0 {
            return Ok(ToolResult::err(format!(
                "old_string not found in {}. Make sure it matches exactly, including whitespace and indentation.",
                path.display()
            )));
        }

        if count > 1 && !replace_all {
            return Ok(ToolResult::err(format!(
                "old_string found {count} times in {}. Provide more surrounding context to make it unique, or set replace_all to true.",
                path.display()
            )));
        }

        let new_content = if replace_all {
            content.replace(&input.old_string, &input.new_string)
        } else {
            // Replace only the first (and only) occurrence.
            content.replacen(&input.old_string, &input.new_string, 1)
        };

        match tokio::fs::write(&path, &new_content).await {
            Ok(()) => {
                let replacements = if replace_all {
                    format!("{count} replacement(s)")
                } else {
                    "1 replacement".to_string()
                };
                Ok(ToolResult::ok(format!(
                    "Successfully edited {} ({replacements})",
                    path.display()
                )))
            }
            Err(e) => Ok(ToolResult::err(format!(
                "Failed to write {}: {e}",
                path.display()
            ))),
        }
    }

    // ── Bash ────────────────────────────────────────────────────────────

    /// Run a shell command via `/bin/bash -c`, capturing stdout and stderr.
    /// Supports an optional timeout (in milliseconds).
    async fn execute_bash(&self, input: &BashInput) -> Result<ToolResult> {
        let timeout_ms = input.timeout.unwrap_or(120_000);
        let timeout_dur = Duration::from_millis(timeout_ms);

        let child = Command::new("/bin/bash")
            .arg("-c")
            .arg(&input.command)
            .current_dir(&self.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::err(format!("Failed to spawn process: {e}")));
            }
        };

        // Take stdout/stderr handles before waiting so we can still kill on timeout.
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        let wait_result = tokio::time::timeout(timeout_dur, child.wait()).await;

        match wait_result {
            Ok(Ok(status)) => {
                let mut stdout_bytes = Vec::new();
                let mut stderr_bytes = Vec::new();

                if let Some(mut out) = stdout_handle {
                    use tokio::io::AsyncReadExt;
                    let _ = out.read_to_end(&mut stdout_bytes).await;
                }
                if let Some(mut err) = stderr_handle {
                    use tokio::io::AsyncReadExt;
                    let _ = err.read_to_end(&mut stderr_bytes).await;
                }

                let stdout = String::from_utf8_lossy(&stdout_bytes);
                let stderr = String::from_utf8_lossy(&stderr_bytes);

                let mut combined = String::new();
                if !stdout.is_empty() {
                    combined.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(&stderr);
                }

                let is_error = !status.success();
                if is_error && combined.is_empty() {
                    combined = format!(
                        "Process exited with code {}",
                        status.code().unwrap_or(-1)
                    );
                }

                Ok(ToolResult {
                    content: combined,
                    is_error,
                })
            }
            Ok(Err(e)) => Ok(ToolResult::err(format!("Process IO error: {e}"))),
            Err(_) => {
                // Timeout – attempt to kill the child.
                let _ = child.kill().await;
                Ok(ToolResult::err(format!(
                    "Command timed out after {timeout_ms}ms"
                )))
            }
        }
    }

    // ── Glob ────────────────────────────────────────────────────────────

    /// Find files matching a glob pattern. Searches from the provided `path` or the cwd.
    async fn execute_glob(&self, input: &GlobInput) -> Result<ToolResult> {
        let base = match &input.path {
            Some(p) => self.resolve_path(p),
            None => self.cwd.clone(),
        };

        let full_pattern = base.join(&input.pattern);
        let pattern_str = full_pattern.to_string_lossy().to_string();

        // Glob matching is CPU-bound; run on the blocking pool.
        let result = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<String>, String> {
            let entries = glob_match(&pattern_str).map_err(|e| format!("Invalid glob pattern: {e}"))?;

            let mut paths: Vec<String> = Vec::new();
            for entry in entries {
                match entry {
                    Ok(p) => paths.push(p.to_string_lossy().to_string()),
                    Err(e) => {
                        warn!("glob entry error: {e}");
                    }
                }
            }
            paths.sort();
            Ok(paths)
        })
        .await
        .map_err(|e| AgentError::ToolExecution(format!("glob task panicked: {e}")))?;

        match result {
            Ok(paths) => {
                if paths.is_empty() {
                    Ok(ToolResult::ok("No files matched the pattern.".to_string()))
                } else {
                    Ok(ToolResult::ok(paths.join("\n")))
                }
            }
            Err(e) => Ok(ToolResult::err(e)),
        }
    }

    // ── Grep ────────────────────────────────────────────────────────────

    /// Search file contents using regex, with support for multiple output modes,
    /// context lines, case insensitivity, line numbers, head limit, and offset.
    async fn execute_grep(&self, input: &GrepInput) -> Result<ToolResult> {
        let input = input.clone();
        let cwd = self.cwd.clone();

        // Grep is CPU-intensive; run on the blocking pool.
        let result = tokio::task::spawn_blocking(move || grep_sync(&input, &cwd))
            .await
            .map_err(|e| AgentError::ToolExecution(format!("grep task panicked: {e}")))?;

        result
    }
}

// ── Grep implementation (synchronous, for spawn_blocking) ───────────────

/// File extension mapping for the `type` filter (mirrors ripgrep's type system).
fn extensions_for_type(file_type: &str) -> Option<Vec<&'static str>> {
    let map: HashMap<&str, Vec<&str>> = HashMap::from([
        ("rust", vec!["rs"]),
        ("rs", vec!["rs"]),
        ("py", vec!["py", "pyi"]),
        ("python", vec!["py", "pyi"]),
        ("js", vec!["js", "mjs", "cjs"]),
        ("ts", vec!["ts", "tsx", "mts", "cts"]),
        ("go", vec!["go"]),
        ("java", vec!["java"]),
        ("c", vec!["c", "h"]),
        ("cpp", vec!["cpp", "cxx", "cc", "hpp", "hxx", "hh", "h"]),
        ("rb", vec!["rb"]),
        ("ruby", vec!["rb"]),
        ("html", vec!["html", "htm"]),
        ("css", vec!["css"]),
        ("json", vec!["json"]),
        ("yaml", vec!["yaml", "yml"]),
        ("toml", vec!["toml"]),
        ("md", vec!["md", "markdown"]),
        ("sh", vec!["sh", "bash", "zsh"]),
        ("sql", vec!["sql"]),
        ("xml", vec!["xml"]),
        ("swift", vec!["swift"]),
        ("kt", vec!["kt", "kts"]),
        ("scala", vec!["scala"]),
    ]);
    map.get(file_type).cloned()
}

/// Check whether a file path matches the glob filter or type filter.
fn matches_file_filter(path: &Path, glob_filter: &Option<glob::Pattern>, type_exts: &Option<Vec<&str>>) -> bool {
    if let Some(pat) = glob_filter {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !pat.matches(&name) {
            return false;
        }
    }
    if let Some(exts) = type_exts {
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if !exts.contains(&ext.as_str()) {
            return false;
        }
    }
    true
}

/// Collect all regular files under `dir`, respecting hidden-directory skipping.
fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_files_recursive(dir, &mut files);
    files.sort();
    files
}

fn walk_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden directories and common noise.
        if name_str.starts_with('.') || name_str == "node_modules" || name_str == "target" {
            continue;
        }

        if path.is_dir() {
            walk_files_recursive(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Synchronous grep implementation.
fn grep_sync(input: &GrepInput, cwd: &Path) -> Result<ToolResult> {
    let output_mode = input
        .output_mode
        .clone()
        .unwrap_or(GrepOutputMode::FilesWithMatches);
    let case_insensitive = input.case_insensitive.unwrap_or(false);
    let show_line_numbers = input.line_numbers.unwrap_or(true);
    let multiline = input.multiline.unwrap_or(false);

    // Context lines: -C is an alias, prefer `context`, then `-C`, then individual -A/-B.
    let context_lines = input.context.or(input.context_alias);
    let before_context = input.before_context.or(context_lines).unwrap_or(0) as usize;
    let after_context = input.after_context.or(context_lines).unwrap_or(0) as usize;

    let head_limit = input.head_limit.unwrap_or(0) as usize;
    let offset = input.offset.unwrap_or(0) as usize;

    // Build regex.
    let re = RegexBuilder::new(&input.pattern)
        .case_insensitive(case_insensitive)
        .multi_line(multiline)
        .dot_matches_new_line(multiline)
        .build()?;

    // Determine search root.
    let search_path = match &input.path {
        Some(p) => {
            let resolved = if Path::new(p).is_absolute() {
                PathBuf::from(p)
            } else {
                cwd.join(p)
            };
            resolved
        }
        None => cwd.to_path_buf(),
    };

    // Compile optional filters.
    let glob_filter = input.glob.as_ref().map(|g| {
        glob::Pattern::new(g).unwrap_or_else(|_| glob::Pattern::new("*").unwrap())
    });
    let type_exts = input.file_type.as_ref().and_then(|t| {
        extensions_for_type(t).map(|v| v.into_iter().collect::<Vec<_>>())
    });

    // Collect files to search.
    let files = if search_path.is_file() {
        vec![search_path.clone()]
    } else {
        walk_files(&search_path)
    };

    // Filter files.
    let files: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| matches_file_filter(f, &glob_filter, &type_exts))
        .collect();

    match output_mode {
        GrepOutputMode::FilesWithMatches => {
            grep_files_with_matches(&re, &files, offset, head_limit)
        }
        GrepOutputMode::Count => grep_count(&re, &files, offset, head_limit),
        GrepOutputMode::Content => grep_content(
            &re,
            &files,
            before_context,
            after_context,
            show_line_numbers,
            offset,
            head_limit,
        ),
    }
}

fn grep_files_with_matches(
    re: &Regex,
    files: &[PathBuf],
    offset: usize,
    head_limit: usize,
) -> Result<ToolResult> {
    let mut matched: Vec<String> = Vec::new();
    for file in files {
        if let Ok(content) = std::fs::read_to_string(file) {
            if re.is_match(&content) {
                matched.push(file.to_string_lossy().to_string());
            }
        }
    }

    let result = apply_offset_limit(matched, offset, head_limit);
    if result.is_empty() {
        Ok(ToolResult::ok("No matches found.".to_string()))
    } else {
        Ok(ToolResult::ok(result.join("\n")))
    }
}

fn grep_count(
    re: &Regex,
    files: &[PathBuf],
    offset: usize,
    head_limit: usize,
) -> Result<ToolResult> {
    let mut entries: Vec<String> = Vec::new();
    for file in files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let count = re.find_iter(&content).count();
            if count > 0 {
                entries.push(format!("{}:{count}", file.to_string_lossy()));
            }
        }
    }

    let result = apply_offset_limit(entries, offset, head_limit);
    if result.is_empty() {
        Ok(ToolResult::ok("No matches found.".to_string()))
    } else {
        Ok(ToolResult::ok(result.join("\n")))
    }
}

fn grep_content(
    re: &Regex,
    files: &[PathBuf],
    before_context: usize,
    after_context: usize,
    show_line_numbers: bool,
    offset: usize,
    head_limit: usize,
) -> Result<ToolResult> {
    let mut output_lines: Vec<String> = Vec::new();

    for file in files {
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let file_display = file.to_string_lossy();

        // Find which lines match.
        let mut matching_line_indices: Vec<usize> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if re.is_match(line) {
                matching_line_indices.push(i);
            }
        }

        if matching_line_indices.is_empty() {
            continue;
        }

        // Build set of lines to display (matches + context).
        let mut display_set = Vec::new();
        for &idx in &matching_line_indices {
            let start = idx.saturating_sub(before_context);
            let end = (idx + after_context + 1).min(lines.len());
            for i in start..end {
                display_set.push(i);
            }
        }
        display_set.sort();
        display_set.dedup();

        // Emit grouped output with separators between non-contiguous ranges.
        let mut prev: Option<usize> = None;
        for &line_idx in &display_set {
            if let Some(p) = prev {
                if line_idx > p + 1 {
                    output_lines.push("--".to_string());
                }
            }

            let line_content = lines[line_idx];
            if show_line_numbers {
                let sep = if matching_line_indices.contains(&line_idx) {
                    ':'
                } else {
                    '-'
                };
                output_lines.push(format!(
                    "{file_display}{sep}{}{sep}{line_content}",
                    line_idx + 1
                ));
            } else {
                output_lines.push(format!("{file_display}:{line_content}"));
            }

            prev = Some(line_idx);
        }
    }

    let result = apply_offset_limit(output_lines, offset, head_limit);
    if result.is_empty() {
        Ok(ToolResult::ok("No matches found.".to_string()))
    } else {
        Ok(ToolResult::ok(result.join("\n")))
    }
}

/// Apply offset (skip) and head_limit (take) to a list of entries.
fn apply_offset_limit(items: Vec<String>, offset: usize, head_limit: usize) -> Vec<String> {
    let after_offset: Vec<String> = items.into_iter().skip(offset).collect();
    if head_limit > 0 {
        after_offset.into_iter().take(head_limit).collect()
    } else {
        after_offset
    }
}
