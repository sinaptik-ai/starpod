use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use starpod_core::{StarpodError, Result};

// ── AgentSkills-compatible SKILL.md frontmatter ─────────────────────────────

/// YAML frontmatter parsed from a SKILL.md file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    /// Skill identifier (must match directory name per AgentSkills spec).
    pub name: String,
    /// Human-readable description of what the skill does and when to use it.
    pub description: String,
    /// Semantic version (e.g. "0.1.0").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Optional license.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Optional compatibility notes (e.g. "Requires git, docker").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    /// Arbitrary key-value metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
    /// Pre-approved tools (experimental, space-delimited in YAML).
    #[serde(default, rename = "allowed-tools", skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<String>,
}

/// A loaded skill with parsed frontmatter and body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Skill name (directory name / frontmatter name).
    pub name: String,
    /// Human-readable description from frontmatter (or auto-generated).
    pub description: String,
    /// Semantic version (e.g. "0.1.0").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The instruction body (markdown after frontmatter).
    pub body: String,
    /// Full raw content of SKILL.md (frontmatter + body).
    pub raw_content: String,
    /// When the skill was created (ISO 8601).
    pub created_at: String,
    /// Absolute path to the skill directory.
    pub skill_dir: PathBuf,
    /// Optional compatibility notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    /// Arbitrary metadata from frontmatter.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
    /// Pre-approved tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<String>,
}

/// Parse SKILL.md content into (frontmatter, body).
///
/// Handles files with and without YAML frontmatter for backward compatibility.
fn parse_skill_md(raw: &str) -> (Option<SkillFrontmatter>, String) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        // No frontmatter — entire content is the body
        return (None, raw.to_string());
    }

    // Find the closing ---
    let after_open = &trimmed[3..];
    // Skip the rest of the opening --- line
    let after_newline = match after_open.find('\n') {
        Some(pos) => &after_open[pos + 1..],
        None => return (None, raw.to_string()),
    };

    match after_newline.find("\n---") {
        Some(end_pos) => {
            let yaml_str = &after_newline[..end_pos];
            let body_start = end_pos + 4; // "\n---".len()
            let rest = &after_newline[body_start..];
            // Skip the rest of the closing --- line
            let body = match rest.find('\n') {
                Some(pos) => rest[pos + 1..].to_string(),
                None => String::new(),
            };

            match serde_yaml::from_str::<SkillFrontmatter>(yaml_str) {
                Ok(fm) => (Some(fm), body),
                Err(e) => {
                    warn!(error = %e, "Failed to parse SKILL.md frontmatter, treating as plain markdown");
                    (None, raw.to_string())
                }
            }
        }
        None => {
            // No closing --- found
            (None, raw.to_string())
        }
    }
}

/// Format frontmatter + body into a valid SKILL.md file.
fn format_skill_md(name: &str, description: &str, version: Option<&str>, body: &str) -> String {
    let version_line = match version {
        Some(v) => format!("\nversion: {}", v),
        None => String::new(),
    };
    format!(
        "---\nname: {}\ndescription: {}{}\n---\n\n{}",
        name, description, version_line, body
    )
}

/// Manages skills as markdown files on disk.
///
/// Skills live at `<skills_dir>/<name>/SKILL.md` and follow the
/// [AgentSkills](https://agentskills.io) open format — each skill is a
/// directory containing a `SKILL.md` file with YAML frontmatter
/// (`name`, `description`) and a markdown body with instructions.
///
/// Skills always live at `.starpod/skills/` (instance-local). In workspace mode,
/// workspace-level skills are copied into the instance during blueprint application.
///
/// # Progressive disclosure
///
/// Instead of injecting all skill content into every prompt, the store
/// supports a two-tier approach:
///
/// 1. **Catalog** ([`skill_catalog`]) — compact XML with name + description
///    (~50-100 tokens per skill), injected into the system prompt.
/// 2. **Activation** ([`activate_skill`]) — full instructions loaded on
///    demand when the model decides a skill is relevant.
///
/// # Example
///
/// ```
/// # use tempfile::TempDir;
/// # let tmp = TempDir::new().unwrap();
/// use starpod_skills::SkillStore;
///
/// let store = SkillStore::new(tmp.path()).unwrap();
///
/// // Create
/// store.create("code-review", "Review code for bugs.", None, "Check error handling.").unwrap();
///
/// // Catalog for system prompt
/// let catalog = store.skill_catalog().unwrap();
/// assert!(catalog.contains("code-review"));
///
/// // Activate when needed
/// let instructions = store.activate_skill("code-review").unwrap().unwrap();
/// assert!(instructions.contains("Check error handling."));
/// ```
pub struct SkillStore {
    skills_dir: PathBuf,
    /// Optional filter: when set, only skills whose names appear here are visible.
    filter: Option<Vec<String>>,
}

impl SkillStore {
    /// Create a new SkillStore from a skills directory.
    pub fn new(skills_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(skills_dir)?;
        Ok(Self {
            skills_dir: skills_dir.to_path_buf(),
            filter: None,
        })
    }

    /// Set an optional skill filter. When set, `list()`, `skill_catalog()`, and
    /// `skill_names()` only return skills whose names are in `names`.
    /// An empty list means "all skills" (no filtering).
    pub fn with_filter(mut self, names: Vec<String>) -> Self {
        if names.is_empty() {
            self.filter = None;
        } else {
            self.filter = Some(names);
        }
        self
    }

    /// Load a single skill from a directory entry.
    fn load_skill(&self, name: &str, skill_file: &Path) -> Result<Skill> {
        let raw_content = std::fs::read_to_string(skill_file)?;
        let created_at = std::fs::metadata(skill_file)
            .and_then(|m| m.created())
            .map(|t| chrono::DateTime::<Utc>::from(t).to_rfc3339())
            .unwrap_or_else(|_| Utc::now().to_rfc3339());

        let skill_dir = skill_file.parent().unwrap_or(skill_file).to_path_buf();
        let (frontmatter, body) = parse_skill_md(&raw_content);

        match frontmatter {
            Some(fm) => Ok(Skill {
                name: fm.name.clone(),
                description: fm.description,
                version: fm.version,
                body,
                raw_content,
                created_at,
                skill_dir,
                compatibility: fm.compatibility,
                metadata: fm.metadata,
                allowed_tools: fm.allowed_tools,
            }),
            None => {
                // Backward compat: no frontmatter, use directory name and
                // generate description from first line of content.
                let first_line = raw_content.lines().next().unwrap_or("").trim();
                let description = if first_line.len() > 120 {
                    format!("{}...", &first_line[..117])
                } else if first_line.is_empty() {
                    format!("Skill: {}", name)
                } else {
                    first_line.to_string()
                };

                Ok(Skill {
                    name: name.to_string(),
                    description,
                    version: None,
                    body: raw_content.clone(),
                    raw_content,
                    created_at,
                    skill_dir,
                    compatibility: None,
                    metadata: HashMap::new(),
                    allowed_tools: None,
                })
            }
        }
    }

    /// List all available skills.
    pub fn list(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();
        let entries = std::fs::read_dir(&self.skills_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let skill_file = path.join("SKILL.md");
                if skill_file.exists() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    match self.load_skill(&name, &skill_file) {
                        Ok(skill) => skills.push(skill),
                        Err(e) => {
                            warn!(skill = %name, error = %e, "Failed to load skill, skipping");
                        }
                    }
                }
            }
        }

        // Apply filter if set
        if let Some(ref allowed) = self.filter {
            skills.retain(|s| allowed.iter().any(|a| a == &s.name));
        }

        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(skills)
    }

    /// Get a specific skill by name.
    pub fn get(&self, name: &str) -> Result<Option<Skill>> {
        validate_skill_name(name)?;
        let skill_file = self.skills_dir.join(name).join("SKILL.md");
        if !skill_file.exists() {
            return Ok(None);
        }
        self.load_skill(name, &skill_file).map(Some)
    }

    /// Create a new skill with AgentSkills-compatible frontmatter.
    /// Fails if it already exists.
    pub fn create(&self, name: &str, description: &str, version: Option<&str>, body: &str) -> Result<()> {
        validate_skill_name(name)?;
        let skill_dir = self.skills_dir.join(name);
        if skill_dir.exists() {
            return Err(StarpodError::Skill(format!(
                "Skill '{}' already exists. Use update instead.",
                name
            )));
        }
        std::fs::create_dir_all(&skill_dir)?;
        let content = format_skill_md(name, description, version, body);
        std::fs::write(skill_dir.join("SKILL.md"), content)?;
        debug!(skill = %name, "Created skill");
        Ok(())
    }

    /// Update an existing skill's description and/or body.
    pub fn update(&self, name: &str, description: &str, version: Option<&str>, body: &str) -> Result<()> {
        validate_skill_name(name)?;
        let skill_file = self.skills_dir.join(name).join("SKILL.md");
        if !skill_file.exists() {
            return Err(StarpodError::Skill(format!(
                "Skill '{}' does not exist. Use create instead.",
                name
            )));
        }
        let content = format_skill_md(name, description, version, body);
        std::fs::write(&skill_file, content)?;
        debug!(skill = %name, "Updated skill");
        Ok(())
    }

    /// Delete a skill and its directory.
    pub fn delete(&self, name: &str) -> Result<()> {
        validate_skill_name(name)?;
        let skill_dir = self.skills_dir.join(name);
        if !skill_dir.exists() {
            return Err(StarpodError::Skill(format!(
                "Skill '{}' does not exist.",
                name
            )));
        }
        std::fs::remove_dir_all(&skill_dir)?;
        debug!(skill = %name, "Deleted skill");
        Ok(())
    }

    /// Build the skill catalog for system prompt injection (progressive disclosure tier 1).
    ///
    /// Returns an XML catalog of skill names + descriptions (~50-100 tokens per skill).
    /// The model uses this to decide which skills to activate.
    /// Returns an empty string if no skills exist.
    pub fn skill_catalog(&self) -> Result<String> {
        let skills = self.list()?;
        if skills.is_empty() {
            return Ok(String::new());
        }

        let mut xml = String::from("<available_skills>\n");
        for skill in &skills {
            xml.push_str(&format!(
                "  <skill>\n    <name>{}</name>\n    <description>{}</description>\n  </skill>\n",
                xml_escape(&skill.name),
                xml_escape(&skill.description),
            ));
        }
        xml.push_str("</available_skills>");

        Ok(xml)
    }

    /// Activate a skill by name — returns full instructions (progressive disclosure tier 2).
    ///
    /// Returns the skill body wrapped in identifying XML tags, plus a listing
    /// of any bundled resource files.
    pub fn activate_skill(&self, name: &str) -> Result<Option<String>> {
        let skill = match self.get(name)? {
            Some(s) => s,
            None => return Ok(None),
        };

        let mut result = format!(
            "<skill_content name=\"{}\">\n{}\n",
            xml_escape(&skill.name),
            skill.body.trim(),
        );

        // List bundled resources (scripts/, references/, assets/)
        let resources = list_skill_resources(&skill.skill_dir);
        if !resources.is_empty() {
            result.push_str("\n<skill_resources>\n");
            for resource in &resources {
                result.push_str(&format!("  <file>{}</file>\n", resource));
            }
            result.push_str("</skill_resources>\n");
        }

        result.push_str("</skill_content>");

        Ok(Some(result))
    }

    /// List available skill names (convenience for tool enum constraints).
    pub fn skill_names(&self) -> Result<Vec<String>> {
        self.list().map(|skills| skills.into_iter().map(|s| s.name).collect())
    }
}

/// Escape XML special characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// List resource files in a skill directory (scripts/, references/, assets/).
fn list_skill_resources(skill_dir: &Path) -> Vec<String> {
    let mut resources = Vec::new();
    for subdir in &["scripts", "references", "assets"] {
        let dir = skill_dir.join(subdir);
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if entry.path().is_file() {
                        resources.push(format!(
                            "{}/{}",
                            subdir,
                            entry.file_name().to_string_lossy()
                        ));
                    }
                }
            }
        }
    }
    resources.sort();
    resources
}

/// Validate that a skill name is safe for use as a directory name.
/// Follows AgentSkills spec: lowercase alphanumeric + hyphens, no leading/trailing
/// or consecutive hyphens, max 64 chars.
fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(StarpodError::Skill("Skill name cannot be empty".into()));
    }
    if name.len() > 64 {
        return Err(StarpodError::Skill(format!(
            "Skill name '{}' exceeds 64 characters",
            name
        )));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(StarpodError::Skill(format!(
            "Invalid skill name '{}': must not contain path separators or '..'",
            name
        )));
    }
    if name.starts_with('.') {
        return Err(StarpodError::Skill(format!(
            "Invalid skill name '{}': must not start with '.'",
            name
        )));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(StarpodError::Skill(format!(
            "Invalid skill name '{}': must not start or end with a hyphen",
            name
        )));
    }
    if name.contains("--") {
        return Err(StarpodError::Skill(format!(
            "Invalid skill name '{}': must not contain consecutive hyphens",
            name
        )));
    }
    // Check character set: lowercase alphanumeric + hyphens
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(StarpodError::Skill(format!(
            "Invalid skill name '{}': must contain only lowercase letters, digits, and hyphens",
            name
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── Frontmatter parsing ────────────────────────────────────────────────

    #[test]
    fn test_parse_frontmatter_minimal() {
        let content = "---\nname: my-skill\ndescription: Does things.\n---\n\n# Instructions\nDo stuff.";
        let (fm, body) = parse_skill_md(content);
        let fm = fm.unwrap();
        assert_eq!(fm.name, "my-skill");
        assert_eq!(fm.description, "Does things.");
        assert_eq!(body.trim(), "# Instructions\nDo stuff.");
    }

    #[test]
    fn test_parse_frontmatter_all_optional_fields() {
        let content = "---\nname: pdf-tool\ndescription: Process PDFs.\nlicense: MIT\ncompatibility: Requires pdfplumber\nmetadata:\n  author: test\n  version: \"1.0\"\nallowed-tools: Bash(git:*) Read\n---\n\nBody here.";
        let (fm, body) = parse_skill_md(content);
        let fm = fm.unwrap();
        assert_eq!(fm.name, "pdf-tool");
        assert_eq!(fm.license.as_deref(), Some("MIT"));
        assert_eq!(fm.compatibility.as_deref(), Some("Requires pdfplumber"));
        assert_eq!(fm.metadata.get("author").map(|s| s.as_str()), Some("test"));
        assert_eq!(fm.metadata.get("version").map(|s| s.as_str()), Some("1.0"));
        assert_eq!(fm.allowed_tools.as_deref(), Some("Bash(git:*) Read"));
        assert_eq!(body.trim(), "Body here.");
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "Just plain markdown.";
        let (fm, body) = parse_skill_md(content);
        assert!(fm.is_none());
        assert_eq!(body, "Just plain markdown.");
    }

    #[test]
    fn test_parse_malformed_yaml_falls_back() {
        let content = "---\n{not valid yaml [[[[\n---\n\nBody.";
        let (fm, body) = parse_skill_md(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_no_closing_delimiter() {
        let content = "---\nname: broken\ndescription: No closing\n";
        let (fm, body) = parse_skill_md(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_empty_body() {
        let content = "---\nname: empty\ndescription: No body.\n---";
        let (fm, body) = parse_skill_md(content);
        let fm = fm.unwrap();
        assert_eq!(fm.name, "empty");
        assert!(body.is_empty());
    }

    #[test]
    fn test_parse_multiline_body() {
        let content = "---\nname: multi\ndescription: Multi-line body.\n---\n\nLine 1\n\nLine 2\n\n## Section\nMore content.";
        let (fm, body) = parse_skill_md(content);
        assert!(fm.is_some());
        assert!(body.contains("Line 1"));
        assert!(body.contains("Line 2"));
        assert!(body.contains("## Section"));
    }

    #[test]
    fn test_parse_frontmatter_only_required_fields() {
        let content = "---\nname: minimal\ndescription: Just the basics.\n---\n\nBody.";
        let (fm, _body) = parse_skill_md(content);
        let fm = fm.unwrap();
        assert!(fm.license.is_none());
        assert!(fm.compatibility.is_none());
        assert!(fm.metadata.is_empty());
        assert!(fm.allowed_tools.is_none());
    }

    // ── format_skill_md ────────────────────────────────────────────────────

    #[test]
    fn test_format_skill_md() {
        let result = format_skill_md("test", "A test skill.", None, "Do things.");
        assert!(result.starts_with("---\n"));
        assert!(result.contains("name: test"));
        assert!(result.contains("description: A test skill."));
        assert!(!result.contains("version:"));
        assert!(result.contains("---\n\nDo things."));
    }

    #[test]
    fn test_format_skill_md_with_version() {
        let result = format_skill_md("test", "A test skill.", Some("0.1.0"), "Do things.");
        assert!(result.contains("version: 0.1.0"));
    }

    #[test]
    fn test_format_then_parse_roundtrip() {
        let formatted = format_skill_md("roundtrip", "Round-trip test.", Some("1.2.3"), "Instructions here.");
        let (fm, body) = parse_skill_md(&formatted);
        let fm = fm.unwrap();
        assert_eq!(fm.name, "roundtrip");
        assert_eq!(fm.description, "Round-trip test.");
        assert_eq!(fm.version.as_deref(), Some("1.2.3"));
        assert_eq!(body.trim(), "Instructions here.");
    }

    // ── CRUD operations ────────────────────────────────────────────────────

    #[test]
    fn test_create_and_list() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("summarize-pr", "Summarize pull requests.", None, "Read the diff and summarize.").unwrap();
        store.create("code-review", "Review code for issues.", None, "Check for bugs.").unwrap();

        let skills = store.list().unwrap();
        assert_eq!(skills.len(), 2);
        // Sorted alphabetically
        assert_eq!(skills[0].name, "code-review");
        assert_eq!(skills[0].description, "Review code for issues.");
        assert_eq!(skills[1].name, "summarize-pr");
    }

    #[test]
    fn test_create_writes_valid_skill_md() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "A useful skill.", None, "Step 1: Do this.\nStep 2: Do that.").unwrap();

        // Verify the file on disk is a valid AgentSkills SKILL.md
        let path = tmp.path().join("my-skill").join("SKILL.md");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.starts_with("---\n"));
        assert!(raw.contains("name: my-skill"));
        assert!(raw.contains("description: A useful skill."));
        assert!(raw.contains("Step 1: Do this."));
    }

    #[test]
    fn test_get() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "Does useful things.", None, "Do something useful.").unwrap();

        let skill = store.get("my-skill").unwrap().unwrap();
        assert_eq!(skill.name, "my-skill");
        assert_eq!(skill.description, "Does useful things.");
        assert_eq!(skill.body.trim(), "Do something useful.");
        assert!(!skill.created_at.is_empty());
        assert!(skill.skill_dir.ends_with("my-skill"));

        assert!(store.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_update() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "v1 desc", None, "v1 body").unwrap();
        store.update("my-skill", "v2 desc", None, "v2 body").unwrap();

        let skill = store.get("my-skill").unwrap().unwrap();
        assert_eq!(skill.description, "v2 desc");
        assert_eq!(skill.body.trim(), "v2 body");
    }

    #[test]
    fn test_create_with_version() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("versioned", "A versioned skill.", Some("0.1.0"), "body").unwrap();

        let skill = store.get("versioned").unwrap().unwrap();
        assert_eq!(skill.version.as_deref(), Some("0.1.0"));

        // Verify it's on disk
        let raw = std::fs::read_to_string(tmp.path().join("versioned").join("SKILL.md")).unwrap();
        assert!(raw.contains("version: 0.1.0"));
    }

    #[test]
    fn test_create_without_version() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("no-ver", "No version.", None, "body").unwrap();

        let skill = store.get("no-ver").unwrap().unwrap();
        assert!(skill.version.is_none());

        let raw = std::fs::read_to_string(tmp.path().join("no-ver").join("SKILL.md")).unwrap();
        assert!(!raw.contains("version:"));
    }

    #[test]
    fn test_delete() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "desc", None, "content").unwrap();
        store.delete("my-skill").unwrap();

        assert!(store.get("my-skill").unwrap().is_none());
        assert_eq!(store.list().unwrap().len(), 0);
        // Directory should be gone
        assert!(!tmp.path().join("my-skill").exists());
    }

    #[test]
    fn test_create_duplicate_fails() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "desc", None, "v1").unwrap();
        let err = store.create("my-skill", "desc", None, "v2").unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_update_nonexistent_fails() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        let err = store.update("nope", "desc", None, "content").unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn test_delete_nonexistent_fails() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        let err = store.delete("nope").unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn test_list_empty() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();
        assert_eq!(store.list().unwrap().len(), 0);
    }

    #[test]
    fn test_list_ignores_non_skill_dirs() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        // Dir without SKILL.md — should be ignored
        std::fs::create_dir_all(tmp.path().join("not-a-skill")).unwrap();
        std::fs::write(
            tmp.path().join("not-a-skill").join("README.md"),
            "not a skill",
        ).unwrap();

        // Regular file in skills dir — should be ignored
        std::fs::write(tmp.path().join("stray-file.txt"), "stray").unwrap();

        store.create("real-skill", "A real skill.", None, "Content.").unwrap();

        let skills = store.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "real-skill");
    }

    // ── Name validation (AgentSkills spec) ─────────────────────────────────

    #[test]
    fn test_valid_names() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        // All should succeed
        store.create("a", "d", None, "c").unwrap();
        store.create("my-skill", "d", None, "c").unwrap();
        store.create("skill123", "d", None, "c").unwrap();
        store.create("a1b2c3", "d", None, "c").unwrap();
        store.create("x", "d", None, "c").unwrap();
        assert_eq!(store.list().unwrap().len(), 5);
    }

    #[test]
    fn test_invalid_names() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        let cases = vec![
            ("", "empty"),
            ("../escape", "path traversal"),
            ("a/b", "slash"),
            ("a\\b", "backslash"),
            (".hidden", "leading dot"),
            ("-leading", "leading hyphen"),
            ("trailing-", "trailing hyphen"),
            ("double--hyphen", "consecutive hyphens"),
            ("UpperCase", "uppercase"),
            ("has space", "space"),
            ("under_score", "underscore"),
            ("has.dot", "dot"),
        ];

        for (name, reason) in cases {
            assert!(
                store.create(name, "d", None, "c").is_err(),
                "Expected '{}' to be rejected ({})",
                name,
                reason,
            );
        }
    }

    #[test]
    fn test_name_max_length() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        // 64 chars — OK
        let name_64 = "a".repeat(64);
        store.create(&name_64, "d", None, "c").unwrap();

        // 65 chars — too long
        let name_65 = "a".repeat(65);
        assert!(store.create(&name_65, "d", None, "c").is_err());
    }

    // ── Progressive disclosure ─────────────────────────────────────────────

    #[test]
    fn test_skill_catalog_empty() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();
        assert_eq!(store.skill_catalog().unwrap(), "");
    }

    #[test]
    fn test_skill_catalog_format() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("alpha", "Alpha does things.", None, "Alpha body.").unwrap();
        store.create("beta", "Beta does other things.", None, "Beta body.").unwrap();

        let catalog = store.skill_catalog().unwrap();
        assert!(catalog.starts_with("<available_skills>"));
        assert!(catalog.ends_with("</available_skills>"));
        assert!(catalog.contains("<name>alpha</name>"));
        assert!(catalog.contains("<description>Alpha does things.</description>"));
        assert!(catalog.contains("<name>beta</name>"));
        // Must NOT contain skill bodies
        assert!(!catalog.contains("Alpha body"));
        assert!(!catalog.contains("Beta body"));
    }

    #[test]
    fn test_skill_catalog_escapes_xml() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("escaping", "Uses <tags> & \"quotes\".", None, "body").unwrap();

        let catalog = store.skill_catalog().unwrap();
        assert!(catalog.contains("&lt;tags&gt;"));
        assert!(catalog.contains("&amp;"));
        assert!(catalog.contains("&quot;quotes&quot;"));
    }

    #[test]
    fn test_activate_skill() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "A skill.", None, "Do the thing.\nStep by step.").unwrap();

        let activated = store.activate_skill("my-skill").unwrap().unwrap();
        assert!(activated.contains("<skill_content name=\"my-skill\">"));
        assert!(activated.contains("Do the thing."));
        assert!(activated.contains("Step by step."));
        assert!(activated.contains("</skill_content>"));
    }

    #[test]
    fn test_activate_nonexistent_returns_none() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();
        assert!(store.activate_skill("nope").unwrap().is_none());
    }

    #[test]
    fn test_activate_skill_no_resources_omits_tag() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("minimal", "Minimal.", None, "Just instructions.").unwrap();

        let activated = store.activate_skill("minimal").unwrap().unwrap();
        assert!(!activated.contains("<skill_resources>"));
    }

    #[test]
    fn test_activate_skill_with_resources() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "A skill.", None, "Instructions.").unwrap();

        // Create resource files in all three standard directories
        let skill_dir = tmp.path().join("my-skill");
        std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
        std::fs::write(skill_dir.join("scripts").join("run.py"), "print('hi')").unwrap();
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(skill_dir.join("references").join("guide.md"), "# Guide").unwrap();
        std::fs::create_dir_all(skill_dir.join("assets")).unwrap();
        std::fs::write(skill_dir.join("assets").join("template.json"), "{}").unwrap();

        let activated = store.activate_skill("my-skill").unwrap().unwrap();
        assert!(activated.contains("<skill_resources>"));
        assert!(activated.contains("<file>scripts/run.py</file>"));
        assert!(activated.contains("<file>references/guide.md</file>"));
        assert!(activated.contains("<file>assets/template.json</file>"));
        assert!(activated.contains("</skill_resources>"));
    }

    // ── Backward compatibility ─────────────────────────────────────────────

    #[test]
    fn test_backward_compat_no_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        // Manually create a skill without frontmatter (old format)
        let skill_dir = tmp.path().join("old-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Just plain instructions.").unwrap();

        let skill = store.get("old-skill").unwrap().unwrap();
        assert_eq!(skill.name, "old-skill");
        assert_eq!(skill.description, "Just plain instructions.");
        assert_eq!(skill.body, "Just plain instructions.");

        // Should appear in catalog
        let catalog = store.skill_catalog().unwrap();
        assert!(catalog.contains("old-skill"));
    }

    #[test]
    fn test_backward_compat_empty_content() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        let skill_dir = tmp.path().join("empty-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "").unwrap();

        let skill = store.get("empty-skill").unwrap().unwrap();
        assert_eq!(skill.name, "empty-skill");
        // Empty first line → auto-generated description
        assert_eq!(skill.description, "Skill: empty-skill");
    }

    #[test]
    fn test_backward_compat_long_first_line_truncated() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        let long_line = "x".repeat(200);
        let skill_dir = tmp.path().join("long-desc");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), &long_line).unwrap();

        let skill = store.get("long-desc").unwrap().unwrap();
        assert!(skill.description.len() <= 120 + 3); // 117 + "..."
        assert!(skill.description.ends_with("..."));
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    #[test]
    fn test_skill_names() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("beta", "B.", None, "b").unwrap();
        store.create("alpha", "A.", None, "a").unwrap();

        let names = store.skill_names().unwrap();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("a<b>c&d"), "a&lt;b&gt;c&amp;d");
        assert_eq!(xml_escape("\"hello\""), "&quot;hello&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
        assert_eq!(xml_escape("plain"), "plain");
    }

    #[test]
    fn test_list_skill_resources_empty() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();
        store.create("bare", "Bare skill.", None, "No resources.").unwrap();

        let skill_dir = tmp.path().join("bare");
        assert!(list_skill_resources(&skill_dir).is_empty());
    }

    // ── Filter ────────────────────────────────────────────────────────────

    #[test]
    fn test_filter_restricts_list() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("alpha", "A.", None, "a").unwrap();
        store.create("beta", "B.", None, "b").unwrap();
        store.create("gamma", "G.", None, "g").unwrap();

        let filtered = SkillStore::new(tmp.path())
            .unwrap()
            .with_filter(vec!["alpha".into(), "gamma".into()]);

        let skills = filtered.list().unwrap();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[1].name, "gamma");
    }

    #[test]
    fn test_filter_affects_catalog() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("alpha", "A.", None, "a").unwrap();
        store.create("beta", "B.", None, "b").unwrap();

        let filtered = SkillStore::new(tmp.path())
            .unwrap()
            .with_filter(vec!["beta".into()]);

        let catalog = filtered.skill_catalog().unwrap();
        assert!(catalog.contains("beta"));
        assert!(!catalog.contains("alpha"));
    }

    #[test]
    fn test_filter_affects_skill_names() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("alpha", "A.", None, "a").unwrap();
        store.create("beta", "B.", None, "b").unwrap();

        let filtered = SkillStore::new(tmp.path())
            .unwrap()
            .with_filter(vec!["alpha".into()]);

        let names = filtered.skill_names().unwrap();
        assert_eq!(names, vec!["alpha"]);
    }

    #[test]
    fn test_empty_filter_means_all() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path())
            .unwrap()
            .with_filter(vec![]);

        store.create("alpha", "A.", None, "a").unwrap();
        store.create("beta", "B.", None, "b").unwrap();

        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn test_list_skill_resources_sorted() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();
        store.create("res", "Has resources.", None, "Content.").unwrap();

        let skill_dir = tmp.path().join("res");
        std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
        std::fs::write(skill_dir.join("scripts").join("z.sh"), "").unwrap();
        std::fs::write(skill_dir.join("scripts").join("a.py"), "").unwrap();
        std::fs::create_dir_all(skill_dir.join("assets")).unwrap();
        std::fs::write(skill_dir.join("assets").join("data.json"), "").unwrap();

        let resources = list_skill_resources(&skill_dir);
        assert_eq!(resources, vec![
            "assets/data.json",
            "scripts/a.py",
            "scripts/z.sh",
        ]);
    }
}
