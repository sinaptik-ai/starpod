//! File-based hook discovery — scans directories for HOOK.md manifests.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::callback::{HookCallback, HookCallbackMatcher};
use crate::eligibility::HookRequirements;
use crate::error::HookError;
use crate::event::HookEvent;
use crate::input::HookInput;
use crate::output::HookOutput;
use crate::runner::HookRegistry;

/// Parsed contents of a HOOK.md manifest file.
///
/// The manifest uses TOML frontmatter delimited by `+++`:
///
/// ```text
/// +++
/// name = "lint-on-write"
/// event = "PostToolUse"
/// matcher = "Write|Edit"
/// timeout = 30
/// command = "./handler.sh"
///
/// [requires]
/// bins = ["eslint"]
/// os = ["macos", "linux"]
/// +++
///
/// # Lint on Write
///
/// Runs eslint after file writes.
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookManifest {
    /// Human-readable name for this hook.
    pub name: String,

    /// Which event this hook fires on.
    pub event: HookEvent,

    /// Optional regex pattern for matcher filtering (e.g., tool name).
    #[serde(default)]
    pub matcher: Option<String>,

    /// Timeout in seconds for the hook's execution.
    #[serde(default)]
    pub timeout: Option<u64>,

    /// Eligibility requirements (binaries, env vars, OS).
    #[serde(default)]
    pub requires: Option<HookRequirements>,

    /// Shell command to execute. The hook input is piped as JSON on stdin,
    /// and the hook output is read from stdout as JSON.
    #[serde(default)]
    pub command: Option<String>,

    /// Human-readable description (extracted from markdown body).
    #[serde(skip)]
    pub description: Option<String>,
}

/// Discovers hooks from HOOK.md files on disk.
///
/// Each hook lives in its own subdirectory as a `HOOK.md` file with TOML
/// frontmatter. The directory structure looks like:
///
/// ```text
/// hooks/
/// ├── lint-on-write/
/// │   ├── HOOK.md
/// │   └── handler.sh
/// └── format-check/
///     ├── HOOK.md
///     └── check.py
/// ```
///
/// # Example
///
/// ```no_run
/// use starpod_hooks::HookDiscovery;
/// use std::path::Path;
///
/// let registry = HookDiscovery::discover(&[
///     Path::new(".starpod/hooks"),
/// ]).unwrap();
/// ```
pub struct HookDiscovery;

impl HookDiscovery {
    /// Scan a directory for hook manifests.
    ///
    /// Looks for `<dir>/*/HOOK.md` files, parses each one, and returns
    /// `(HookEvent, HookCallbackMatcher)` pairs for hooks that have a command.
    pub fn scan_dir(dir: &Path) -> crate::error::Result<Vec<(HookEvent, HookCallbackMatcher)>> {
        let pattern = dir.join("*/HOOK.md");
        let pattern_str = pattern.to_string_lossy();

        let mut results = Vec::new();

        let entries = glob::glob(&pattern_str).map_err(|e| {
            HookError::Discovery(format!("Invalid glob pattern '{}': {}", pattern_str, e))
        })?;

        for entry in entries {
            let path = entry
                .map_err(|e| HookError::Discovery(format!("Failed to read glob entry: {}", e)))?;

            let content = std::fs::read_to_string(&path).map_err(|e| HookError::ManifestParse {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;

            let manifest = parse_manifest(&content, &path)?;

            if let Some(ref command) = manifest.command {
                let hook_dir = path.parent().unwrap_or(dir).to_path_buf();
                let callback =
                    build_command_callback(manifest.name.clone(), command.clone(), hook_dir);

                let mut matcher =
                    HookCallbackMatcher::new(vec![callback]).with_name(&manifest.name);

                if let Some(ref m) = manifest.matcher {
                    matcher = matcher.with_matcher(m);
                }
                if let Some(t) = manifest.timeout {
                    matcher = matcher.with_timeout(t);
                }
                if let Some(req) = manifest.requires {
                    matcher = matcher.with_requirements(req);
                }

                results.push((manifest.event, matcher));
            }
        }

        Ok(results)
    }

    /// Discover hooks from multiple directories and build a registry.
    ///
    /// Scans each directory, collects all manifests, and groups them by event.
    pub fn discover(dirs: &[&Path]) -> crate::error::Result<HookRegistry> {
        let mut registry = HookRegistry::new();

        for dir in dirs {
            if !dir.exists() {
                continue;
            }

            let pairs = Self::scan_dir(dir)?;
            for (event, matcher) in pairs {
                let mut single = HookRegistry::new();
                single.register(event, vec![matcher]);
                registry.merge(single);
            }
        }

        Ok(registry)
    }
}

/// Parse TOML frontmatter from a HOOK.md file.
///
/// Frontmatter is delimited by `+++` on its own line.
fn parse_manifest(content: &str, path: &Path) -> crate::error::Result<HookManifest> {
    let trimmed = content.trim();

    if !trimmed.starts_with("+++") {
        return Err(HookError::ManifestParse {
            path: path.display().to_string(),
            reason: "Missing opening +++ frontmatter delimiter".to_string(),
        });
    }

    // Find the closing +++
    let after_open = &trimmed[3..];
    let close_pos = after_open
        .find("+++")
        .ok_or_else(|| HookError::ManifestParse {
            path: path.display().to_string(),
            reason: "Missing closing +++ frontmatter delimiter".to_string(),
        })?;

    let toml_str = &after_open[..close_pos].trim();
    let body = after_open[close_pos + 3..].trim();

    let mut manifest: HookManifest =
        toml::from_str(toml_str).map_err(|e| HookError::ManifestParse {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

    if !body.is_empty() {
        manifest.description = Some(body.to_string());
    }

    Ok(manifest)
}

/// Build a HookCallback that executes a shell command as a subprocess.
///
/// The callback serializes `HookInput` to JSON, pipes it on stdin,
/// reads stdout, and parses the output as `HookOutput` JSON.
fn build_command_callback(
    hook_name: String,
    command: String,
    work_dir: std::path::PathBuf,
) -> HookCallback {
    Arc::new(
        move |input: HookInput, _tool_use_id: Option<String>, _cancel| {
            let hook_name = hook_name.clone();
            let command = command.clone();
            let work_dir = work_dir.clone();

            Box::pin(async move {
                let input_json = serde_json::to_string(&input)?;

                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&work_dir)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| HookError::CommandExecution {
                        hook_name: hook_name.clone(),
                        reason: e.to_string(),
                    })?;

                use tokio::io::AsyncWriteExt;

                let mut child = output;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(input_json.as_bytes()).await.map_err(|e| {
                        HookError::CommandExecution {
                            hook_name: hook_name.clone(),
                            reason: format!("Failed to write to stdin: {}", e),
                        }
                    })?;
                    // Drop stdin to close the pipe and signal EOF
                }

                let result =
                    child
                        .wait_with_output()
                        .await
                        .map_err(|e| HookError::CommandExecution {
                            hook_name: hook_name.clone(),
                            reason: e.to_string(),
                        })?;

                let stdout = String::from_utf8_lossy(&result.stdout);
                let stdout_trimmed = stdout.trim();

                if stdout_trimmed.is_empty() {
                    // No output — return default (continue)
                    return Ok(HookOutput::default());
                }

                let hook_output: HookOutput =
                    serde_json::from_str(stdout_trimmed).map_err(|e| {
                        HookError::CommandExecution {
                            hook_name: hook_name.clone(),
                            reason: format!(
                                "Failed to parse stdout as HookOutput: {}. stdout: {}",
                                e, stdout_trimmed
                            ),
                        }
                    })?;

                Ok(hook_output)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_hook_dir(tmp: &Path, name: &str, content: &str) {
        let hook_dir = tmp.join(name);
        fs::create_dir_all(&hook_dir).unwrap();
        fs::write(hook_dir.join("HOOK.md"), content).unwrap();
    }

    #[test]
    fn parse_valid_manifest() {
        let content = r#"+++
name = "test-hook"
event = "PostToolUse"
matcher = "Bash"
timeout = 30
command = "./run.sh"

[requires]
bins = ["sh"]
os = ["macos", "linux"]
+++

# Test Hook

A test hook for unit tests.
"#;

        let manifest = parse_manifest(content, Path::new("test/HOOK.md")).unwrap();
        assert_eq!(manifest.name, "test-hook");
        assert_eq!(manifest.event, HookEvent::PostToolUse);
        assert_eq!(manifest.matcher.as_deref(), Some("Bash"));
        assert_eq!(manifest.timeout, Some(30));
        assert_eq!(manifest.command.as_deref(), Some("./run.sh"));
        assert!(manifest.description.unwrap().contains("A test hook"));

        let req = manifest.requires.unwrap();
        assert_eq!(req.bins, vec!["sh"]);
        assert_eq!(req.os, vec!["macos", "linux"]);
    }

    #[test]
    fn parse_minimal_manifest() {
        let content = r#"+++
name = "minimal"
event = "PreToolUse"
+++
"#;

        let manifest = parse_manifest(content, Path::new("test/HOOK.md")).unwrap();
        assert_eq!(manifest.name, "minimal");
        assert_eq!(manifest.event, HookEvent::PreToolUse);
        assert!(manifest.matcher.is_none());
        assert!(manifest.timeout.is_none());
        assert!(manifest.command.is_none());
        assert!(manifest.requires.is_none());
    }

    #[test]
    fn parse_missing_frontmatter_delimiters() {
        let content = "name = \"test\"\nevent = \"PostToolUse\"";
        let err = parse_manifest(content, Path::new("test/HOOK.md")).unwrap_err();
        assert!(err.to_string().contains("Missing opening +++"));
    }

    #[test]
    fn parse_missing_closing_delimiter() {
        let content = "+++\nname = \"test\"\nevent = \"PostToolUse\"\n";
        let err = parse_manifest(content, Path::new("test/HOOK.md")).unwrap_err();
        assert!(err.to_string().contains("Missing closing +++"));
    }

    #[test]
    fn scan_dir_finds_hooks() {
        let tmp = tempdir();
        create_hook_dir(
            &tmp,
            "my-hook",
            r#"+++
name = "my-hook"
event = "PostToolUse"
matcher = "Bash"
command = "echo '{}'"
+++
"#,
        );

        let results = HookDiscovery::scan_dir(&tmp).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, HookEvent::PostToolUse);
        assert_eq!(results[0].1.name.as_deref(), Some("my-hook"));
        assert_eq!(results[0].1.matcher.as_deref(), Some("Bash"));
    }

    #[test]
    fn scan_dir_skips_hooks_without_command() {
        let tmp = tempdir();
        create_hook_dir(
            &tmp,
            "no-cmd",
            r#"+++
name = "no-cmd"
event = "PostToolUse"
+++
"#,
        );

        let results = HookDiscovery::scan_dir(&tmp).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn scan_empty_dir() {
        let tmp = tempdir();
        let results = HookDiscovery::scan_dir(&tmp).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn discover_nonexistent_dir_is_ok() {
        let result = HookDiscovery::discover(&[Path::new("/nonexistent/path/xyz")]);
        assert!(result.is_ok());
    }

    #[test]
    fn discover_builds_registry() {
        let tmp = tempdir();
        create_hook_dir(
            &tmp,
            "hook-a",
            r#"+++
name = "hook-a"
event = "PostToolUse"
command = "echo '{}'"
+++
"#,
        );
        create_hook_dir(
            &tmp,
            "hook-b",
            r#"+++
name = "hook-b"
event = "PreToolUse"
command = "echo '{}'"
+++
"#,
        );

        let registry = HookDiscovery::discover(&[&tmp]).unwrap();
        assert!(registry.has_hooks(&HookEvent::PostToolUse));
        assert!(registry.has_hooks(&HookEvent::PreToolUse));
        assert!(!registry.has_hooks(&HookEvent::SessionStart));
    }

    #[tokio::test]
    async fn command_callback_executes_subprocess() {
        let tmp = tempdir();

        // Create a handler script that echoes valid HookOutput JSON
        let script_path = tmp.join("handler.sh");
        fs::write(
            &script_path,
            r#"#!/bin/sh
cat - > /dev/null
echo '{"continue": true}'
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let callback = build_command_callback(
            "test-cmd".to_string(),
            "./handler.sh".to_string(),
            tmp.clone(),
        );

        let input = HookInput::UserPromptSubmit {
            base: crate::input::BaseHookInput {
                session_id: "test".into(),
                transcript_path: String::new(),
                cwd: "/tmp".into(),
                permission_mode: None,
                agent_id: None,
                agent_type: None,
            },
            prompt: "hello".into(),
        };

        let cancel = tokio_util::sync::CancellationToken::new();
        let result = callback(input, None, cancel).await;
        assert!(result.is_ok());
    }

    #[test]
    fn parse_invalid_toml_returns_error() {
        let content = "+++\nthis is not valid toml {{{\n+++\n";
        let err = parse_manifest(content, Path::new("test/HOOK.md")).unwrap_err();
        assert!(err.to_string().contains("Failed to parse hook manifest"));
    }

    #[test]
    fn parse_missing_required_fields() {
        // Missing event field
        let content = "+++\nname = \"test\"\n+++\n";
        let err = parse_manifest(content, Path::new("test/HOOK.md")).unwrap_err();
        assert!(err.to_string().contains("Failed to parse hook manifest"));
    }

    #[test]
    fn scan_dir_propagates_timeout_and_requires() {
        let tmp = tempdir();
        create_hook_dir(
            &tmp,
            "full-hook",
            r#"+++
name = "full-hook"
event = "PostToolUse"
matcher = "Write"
timeout = 15
command = "echo '{}'"

[requires]
bins = ["sh"]
os = ["macos", "linux"]
+++
"#,
        );

        let results = HookDiscovery::scan_dir(&tmp).unwrap();
        assert_eq!(results.len(), 1);
        let (_, ref matcher) = results[0];
        assert_eq!(matcher.timeout, Some(15));
        assert!(matcher.requires.is_some());
        let req = matcher.requires.as_ref().unwrap();
        assert_eq!(req.bins, vec!["sh"]);
        assert_eq!(req.os, vec!["macos", "linux"]);
    }

    #[test]
    fn discover_multiple_hooks_same_event() {
        let tmp = tempdir();
        create_hook_dir(
            &tmp,
            "hook-1",
            r#"+++
name = "hook-1"
event = "PostToolUse"
command = "echo '{}'"
+++
"#,
        );
        create_hook_dir(
            &tmp,
            "hook-2",
            r#"+++
name = "hook-2"
event = "PostToolUse"
command = "echo '{}'"
+++
"#,
        );

        let registry = HookDiscovery::discover(&[&tmp]).unwrap();
        let matchers = registry.get(&HookEvent::PostToolUse).unwrap();
        assert_eq!(matchers.len(), 2);
    }

    #[tokio::test]
    async fn command_callback_invalid_json_returns_error() {
        let tmp = tempdir();

        let script_path = tmp.join("bad.sh");
        fs::write(
            &script_path,
            "#!/bin/sh\ncat - > /dev/null\necho 'not json'\n",
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let callback =
            build_command_callback("bad-cmd".to_string(), "./bad.sh".to_string(), tmp.clone());

        let input = HookInput::UserPromptSubmit {
            base: crate::input::BaseHookInput {
                session_id: "test".into(),
                transcript_path: String::new(),
                cwd: "/tmp".into(),
                permission_mode: None,
                agent_id: None,
                agent_type: None,
            },
            prompt: "hello".into(),
        };

        let cancel = tokio_util::sync::CancellationToken::new();
        let result = callback(input, None, cancel).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Failed to parse stdout"));
    }

    #[tokio::test]
    async fn command_callback_empty_stdout_returns_default() {
        let tmp = tempdir();

        let script_path = tmp.join("empty.sh");
        fs::write(&script_path, "#!/bin/sh\ncat - > /dev/null\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let callback = build_command_callback(
            "empty-cmd".to_string(),
            "./empty.sh".to_string(),
            tmp.clone(),
        );

        let input = HookInput::UserPromptSubmit {
            base: crate::input::BaseHookInput {
                session_id: "test".into(),
                transcript_path: String::new(),
                cwd: "/tmp".into(),
                permission_mode: None,
                agent_id: None,
                agent_type: None,
            },
            prompt: "hello".into(),
        };

        let cancel = tokio_util::sync::CancellationToken::new();
        let result = callback(input, None, cancel).await.unwrap();
        // Default HookOutput is Sync with all None fields
        assert!(matches!(result, HookOutput::Sync(_)));
    }

    /// Helper to create a temporary directory for tests.
    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "starpod-hooks-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
