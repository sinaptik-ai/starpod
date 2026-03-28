use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::warn;
use uuid::Uuid;

use crate::error::{AgentError, Result};

/// Information about a past session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique session identifier (UUID).
    pub session_id: String,
    /// Display title: custom title, auto-generated summary, or first prompt.
    pub summary: String,
    /// Last modified time in milliseconds since epoch.
    pub last_modified: u64,
    /// Session file size in bytes.
    pub file_size: u64,
    /// User-set session title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    /// First meaningful user prompt in the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_prompt: Option<String>,
    /// Git branch at the end of the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Working directory for the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// A session message from a transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub uuid: String,
    pub session_id: String,
    pub message: serde_json::Value,
    pub parent_tool_use_id: Option<String>,
}

/// Manages session state and persistence.
#[derive(Debug)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub messages: Vec<serde_json::Value>,
    /// Optional override for the home directory (used in tests).
    home_override: Option<PathBuf>,
}

impl Session {
    /// Create a new session with a generated ID.
    pub fn new(cwd: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            cwd: cwd.into(),
            messages: Vec::new(),
            home_override: None,
        }
    }

    /// Create a session with a specific ID.
    pub fn with_id(id: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            cwd: cwd.into(),
            messages: Vec::new(),
            home_override: None,
        }
    }

    /// Set an explicit home directory override (useful for testing).
    pub fn with_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.home_override = Some(home.into());
        self
    }

    /// Get the path where sessions are stored for a given working directory.
    pub fn sessions_dir(cwd: &str) -> PathBuf {
        Self::sessions_dir_with_home(cwd, home_dir_or_tmp())
    }

    /// Get the sessions directory using an explicit home path.
    pub fn sessions_dir_with_home(cwd: &str, home: PathBuf) -> PathBuf {
        let encoded_cwd = encode_path(cwd);
        home.join(".claude").join("projects").join(encoded_cwd)
    }

    /// Get the file path for this session's transcript.
    pub fn transcript_path(&self) -> PathBuf {
        let home = self.home_override.clone().unwrap_or_else(home_dir_or_tmp);
        Self::sessions_dir_with_home(&self.cwd, home).join(format!("{}.jsonl", self.id))
    }

    /// Append a JSON message to the transcript file on disk.
    ///
    /// Creates the sessions directory and file if they don't exist.
    /// The message is serialized as a single JSON line followed by a newline.
    pub async fn append_message(&self, message: &serde_json::Value) -> Result<()> {
        let path = self.transcript_path();

        // Ensure the parent directory exists.
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let mut line = serde_json::to_string(message)?;
        line.push('\n');

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        // Restrict session files to owner-only on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            fs::set_permissions(&path, perms).await?;
        }

        file.write_all(line.as_bytes()).await?;
        file.flush().await?;

        Ok(())
    }

    /// Load all messages from the transcript file.
    ///
    /// Returns an empty vec if the file does not exist.
    /// Skips any lines that fail to parse as JSON, logging a warning.
    pub async fn load_messages(&self) -> Result<Vec<serde_json::Value>> {
        let path = self.transcript_path();

        if !path.exists() {
            return Ok(Vec::new());
        }

        let contents = fs::read_to_string(&path).await?;
        let mut messages = Vec::new();

        for (i, line) in contents.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(value) => messages.push(value),
                Err(e) => {
                    warn!(
                        "Skipping malformed JSON on line {} of {}: {}",
                        i + 1,
                        path.display(),
                        e
                    );
                }
            }
        }

        Ok(messages)
    }
}

/// List sessions for a given directory, sorted by last_modified descending.
///
/// Scans `~/.claude/projects/<encoded-dir>/` for `.jsonl` files. For each file,
/// reads the first few lines to extract the first user prompt and any custom title.
///
/// * `dir` - working directory whose sessions to list; if `None`, uses the
///   current working directory.
/// * `limit` - maximum number of sessions to return; if `None`, returns all.
pub async fn list_sessions(dir: Option<&str>, limit: Option<usize>) -> Result<Vec<SessionInfo>> {
    list_sessions_with_home(dir, limit, home_dir_or_tmp()).await
}

/// Like [`list_sessions`] but with an explicit home directory.
pub async fn list_sessions_with_home(
    dir: Option<&str>,
    limit: Option<usize>,
    home: PathBuf,
) -> Result<Vec<SessionInfo>> {
    let cwd = resolve_cwd(dir)?;
    let sessions_dir = Session::sessions_dir_with_home(&cwd, home);

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(&sessions_dir).await?;
    let mut infos: Vec<SessionInfo> = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        // Only consider .jsonl files.
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("jsonl") {
            continue;
        }

        let session_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(stem) => stem.to_string(),
            None => continue,
        };

        let metadata = match fs::metadata(&path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        let last_modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let file_size = metadata.len();

        // Read the file to extract the first user prompt and any custom title.
        let (first_prompt, custom_title) = extract_session_metadata(&path).await;

        let summary = custom_title
            .clone()
            .or_else(|| first_prompt.clone())
            .unwrap_or_else(|| "(empty session)".to_string());

        infos.push(SessionInfo {
            session_id,
            summary,
            last_modified,
            file_size,
            custom_title,
            first_prompt,
            git_branch: None,
            cwd: Some(cwd.clone()),
        });
    }

    // Sort by last_modified descending (newest first).
    infos.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));

    if let Some(limit) = limit {
        infos.truncate(limit);
    }

    Ok(infos)
}

/// Get messages from a specific session, with optional pagination.
///
/// * `session_id` - the UUID of the session to read.
/// * `dir` - working directory; if `None`, uses the current working directory.
/// * `limit` - max number of messages to return; if `None`, returns all (after offset).
/// * `offset` - number of messages to skip from the start; if `None`, starts at 0.
pub async fn get_session_messages(
    session_id: &str,
    dir: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<SessionMessage>> {
    get_session_messages_with_home(session_id, dir, limit, offset, home_dir_or_tmp()).await
}

/// Like [`get_session_messages`] but with an explicit home directory.
pub async fn get_session_messages_with_home(
    session_id: &str,
    dir: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
    home: PathBuf,
) -> Result<Vec<SessionMessage>> {
    let cwd = resolve_cwd(dir)?;
    let session = Session::with_id(session_id, &cwd).with_home(&home);
    let path = session.transcript_path();

    if !path.exists() {
        return Err(AgentError::SessionNotFound(session_id.to_string()));
    }

    let contents = fs::read_to_string(&path).await?;
    let offset = offset.unwrap_or(0);

    let mut messages: Vec<SessionMessage> = Vec::new();

    for (i, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Apply offset: skip the first `offset` non-empty lines.
        if i < offset {
            continue;
        }

        // Apply limit.
        if let Some(limit) = limit {
            if messages.len() >= limit {
                break;
            }
        }

        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(value) => {
                let msg = SessionMessage {
                    message_type: value
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    uuid: value
                        .get("uuid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    session_id: value
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(session_id)
                        .to_string(),
                    message: value,
                    parent_tool_use_id: None,
                };
                messages.push(msg);
            }
            Err(e) => {
                warn!(
                    "Skipping malformed JSON on line {} of {}: {}",
                    i + 1,
                    path.display(),
                    e
                );
            }
        }
    }

    Ok(messages)
}

/// Find the most recently modified session in the given directory.
///
/// Useful for the "continue" option: resumes the last active session.
/// Returns `None` if no sessions exist.
pub async fn find_most_recent_session(dir: Option<&str>) -> Result<Option<SessionInfo>> {
    let sessions = list_sessions(dir, Some(1)).await?;
    Ok(sessions.into_iter().next())
}

/// Like [`find_most_recent_session`] but with an explicit home directory.
pub async fn find_most_recent_session_with_home(
    dir: Option<&str>,
    home: PathBuf,
) -> Result<Option<SessionInfo>> {
    let sessions = list_sessions_with_home(dir, Some(1), home).await?;
    Ok(sessions.into_iter().next())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Encode a filesystem path for use as a directory name.
/// Every non-alphanumeric character is replaced with `-`.
fn encode_path(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// Return the user's home directory, falling back to `/tmp` if `HOME` is unset.
fn home_dir_or_tmp() -> PathBuf {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Resolve the working directory: use the provided `dir` or fall back to the
/// current working directory.
fn resolve_cwd(dir: Option<&str>) -> Result<String> {
    match dir {
        Some(d) => Ok(d.to_string()),
        None => std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| AgentError::Io(e)),
    }
}

/// Read a JSONL transcript file and extract the first user prompt text and
/// any custom session title.
///
/// We only read up to 50 lines to keep this fast for large transcripts.
async fn extract_session_metadata(path: &PathBuf) -> (Option<String>, Option<String>) {
    let contents = match fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return (None, None),
    };

    let mut first_prompt: Option<String> = None;
    let mut custom_title: Option<String> = None;

    for line in contents.lines().take(50) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Look for a custom title field (set by the user or system).
        if let Some(title) = value.get("customTitle").and_then(|v| v.as_str()) {
            if !title.is_empty() {
                custom_title = Some(title.to_string());
            }
        }
        if let Some(title) = value.get("custom_title").and_then(|v| v.as_str()) {
            if !title.is_empty() {
                custom_title = Some(title.to_string());
            }
        }

        // Extract first user prompt if we haven't found one yet.
        if first_prompt.is_none() {
            if let Some("user") = value.get("type").and_then(|v| v.as_str()) {
                if let Some(content) = value.get("content") {
                    let text = extract_text_from_content(content);
                    if !text.is_empty() {
                        // Truncate long prompts for display.
                        let truncated = if text.len() > 200 {
                            format!("{}...", &text[..200])
                        } else {
                            text
                        };
                        first_prompt = Some(truncated);
                    }
                }
            }
        }

        // If we have both, stop early.
        if first_prompt.is_some() && custom_title.is_some() {
            break;
        }
    }

    (first_prompt, custom_title)
}

/// Extract plain text from a message's `content` field.
///
/// Content may be a string, or an array of content blocks (each with a `type`
/// and `text` field).
fn extract_text_from_content(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }

    if let Some(blocks) = content.as_array() {
        let texts: Vec<&str> = blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    block.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect();
        return texts.join(" ");
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    /// Helper: create a Session whose sessions dir lives inside a temp directory
    /// using the home_override mechanism (no env var mutation needed).
    fn session_in_tmp(tmp: &TempDir) -> Session {
        Session::new("/test/project").with_home(tmp.path())
    }

    #[tokio::test]
    async fn test_append_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let session = session_in_tmp(&tmp);

        let msg1 = json!({"type": "user", "content": "hello"});
        let msg2 = json!({"type": "assistant", "content": "world"});

        session.append_message(&msg1).await.unwrap();
        session.append_message(&msg2).await.unwrap();

        let loaded = session.load_messages().await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0]["content"], "hello");
        assert_eq!(loaded[1]["content"], "world");
    }

    #[tokio::test]
    async fn test_load_messages_empty_file() {
        let tmp = TempDir::new().unwrap();
        let session = session_in_tmp(&tmp);

        // No file written yet.
        let loaded = session.load_messages().await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_transcript_path_encoding() {
        let session = Session::with_id("abc-123", "/home/user/my project");
        let path = session.transcript_path();
        let path_str = path.to_string_lossy();

        // The cwd should be encoded: slashes and spaces become dashes.
        assert!(path_str.contains("-home-user-my-project"));
        assert!(path_str.ends_with("abc-123.jsonl"));
    }

    #[tokio::test]
    async fn test_list_sessions_and_find_most_recent() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();

        let cwd = "/test/project";

        // Create two sessions with a small delay between them.
        let s1 = Session::with_id("session-1", cwd).with_home(&home);
        let s2 = Session::with_id("session-2", cwd).with_home(&home);

        s1.append_message(
            &json!({"type": "user", "content": [{"type": "text", "text": "first prompt"}]}),
        )
        .await
        .unwrap();

        // Small delay so modified times differ.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        s2.append_message(&json!({"type": "user", "content": "second session prompt"}))
            .await
            .unwrap();

        let sessions = list_sessions_with_home(Some(cwd), None, home.clone())
            .await
            .unwrap();
        assert_eq!(sessions.len(), 2);

        // Newest first.
        assert_eq!(sessions[0].session_id, "session-2");
        assert_eq!(sessions[1].session_id, "session-1");

        // Test limit.
        let sessions = list_sessions_with_home(Some(cwd), Some(1), home.clone())
            .await
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "session-2");

        // Test find_most_recent_session.
        let recent = find_most_recent_session_with_home(Some(cwd), home.clone())
            .await
            .unwrap();
        assert!(recent.is_some());
        assert_eq!(recent.unwrap().session_id, "session-2");
    }

    #[tokio::test]
    async fn test_get_session_messages_pagination() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();

        let cwd = "/test/project";
        let session = Session::with_id("paginated", cwd).with_home(&home);

        for i in 0..10 {
            session
                .append_message(&json!({"type": "user", "content": format!("msg {}", i)}))
                .await
                .unwrap();
        }

        // Read all.
        let all = get_session_messages_with_home("paginated", Some(cwd), None, None, home.clone())
            .await
            .unwrap();
        assert_eq!(all.len(), 10);

        // With offset and limit.
        let page =
            get_session_messages_with_home("paginated", Some(cwd), Some(3), Some(2), home.clone())
                .await
                .unwrap();
        assert_eq!(page.len(), 3);
        assert_eq!(page[0].message["content"], "msg 2");

        // Non-existent session.
        let err =
            get_session_messages_with_home("nonexistent", Some(cwd), None, None, home.clone())
                .await;
        assert!(err.is_err());
    }
}
