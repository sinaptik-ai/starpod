use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::debug;

use starpod_core::{StarpodError, Result};

/// A loaded skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Skill name (directory name).
    pub name: String,
    /// Raw markdown content of SKILL.md.
    pub content: String,
    /// When the skill was created (ISO 8601).
    pub created_at: String,
}

/// Manages skills as markdown files on disk.
///
/// Skills live at `<data_dir>/skills/<name>/SKILL.md`.
pub struct SkillStore {
    skills_dir: PathBuf,
}

impl SkillStore {
    /// Create a new SkillStore rooted at `<data_dir>/skills/`.
    pub fn new(data_dir: &Path) -> Result<Self> {
        let skills_dir = data_dir.join("skills");
        std::fs::create_dir_all(&skills_dir)?;
        Ok(Self { skills_dir })
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
                    let content = std::fs::read_to_string(&skill_file)?;
                    let created_at = std::fs::metadata(&skill_file)
                        .and_then(|m| m.created())
                        .map(|t| {
                            chrono::DateTime::<Utc>::from(t).to_rfc3339()
                        })
                        .unwrap_or_else(|_| Utc::now().to_rfc3339());

                    skills.push(Skill {
                        name,
                        content,
                        created_at,
                    });
                }
            }
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
        let content = std::fs::read_to_string(&skill_file)?;
        let created_at = std::fs::metadata(&skill_file)
            .and_then(|m| m.created())
            .map(|t| chrono::DateTime::<Utc>::from(t).to_rfc3339())
            .unwrap_or_else(|_| Utc::now().to_rfc3339());

        Ok(Some(Skill {
            name: name.to_string(),
            content,
            created_at,
        }))
    }

    /// Create a new skill. Fails if it already exists.
    pub fn create(&self, name: &str, content: &str) -> Result<()> {
        validate_skill_name(name)?;
        let skill_dir = self.skills_dir.join(name);
        if skill_dir.exists() {
            return Err(StarpodError::Skill(format!(
                "Skill '{}' already exists. Use update instead.",
                name
            )));
        }
        std::fs::create_dir_all(&skill_dir)?;
        std::fs::write(skill_dir.join("SKILL.md"), content)?;
        debug!(skill = %name, "Created skill");
        Ok(())
    }

    /// Update an existing skill's content.
    pub fn update(&self, name: &str, content: &str) -> Result<()> {
        validate_skill_name(name)?;
        let skill_file = self.skills_dir.join(name).join("SKILL.md");
        if !skill_file.exists() {
            return Err(StarpodError::Skill(format!(
                "Skill '{}' does not exist. Use create instead.",
                name
            )));
        }
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

    /// Load all skills and format them for system prompt injection.
    ///
    /// Returns an empty string if no skills exist.
    pub fn bootstrap_skills(&self) -> Result<String> {
        let skills = self.list()?;
        if skills.is_empty() {
            return Ok(String::new());
        }

        let mut parts = Vec::new();
        parts.push("--- Active Skills ---".to_string());
        for skill in &skills {
            parts.push(format!("### {}\n{}", skill.name, skill.content));
        }

        Ok(parts.join("\n\n"))
    }
}

/// Validate that a skill name is safe for use as a directory name.
fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(StarpodError::Skill("Skill name cannot be empty".into()));
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_and_list() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store
            .create("summarize-pr", "Summarize pull requests by reading the diff.")
            .unwrap();
        store
            .create("code-review", "Review code for bugs and style issues.")
            .unwrap();

        let skills = store.list().unwrap();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "code-review");
        assert_eq!(skills[1].name, "summarize-pr");
    }

    #[test]
    fn test_get() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "Do something useful.").unwrap();

        let skill = store.get("my-skill").unwrap().unwrap();
        assert_eq!(skill.name, "my-skill");
        assert_eq!(skill.content, "Do something useful.");

        assert!(store.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_update() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "v1").unwrap();
        store.update("my-skill", "v2").unwrap();

        let skill = store.get("my-skill").unwrap().unwrap();
        assert_eq!(skill.content, "v2");
    }

    #[test]
    fn test_delete() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "content").unwrap();
        store.delete("my-skill").unwrap();

        assert!(store.get("my-skill").unwrap().is_none());
        assert_eq!(store.list().unwrap().len(), 0);
    }

    #[test]
    fn test_create_duplicate_fails() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        store.create("my-skill", "v1").unwrap();
        assert!(store.create("my-skill", "v2").is_err());
    }

    #[test]
    fn test_update_nonexistent_fails() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        assert!(store.update("nope", "content").is_err());
    }

    #[test]
    fn test_delete_nonexistent_fails() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        assert!(store.delete("nope").is_err());
    }

    #[test]
    fn test_invalid_names() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        assert!(store.create("", "content").is_err());
        assert!(store.create("../escape", "content").is_err());
        assert!(store.create("a/b", "content").is_err());
        assert!(store.create(".hidden", "content").is_err());
    }

    #[test]
    fn test_bootstrap_skills() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::new(tmp.path()).unwrap();

        // Empty
        assert_eq!(store.bootstrap_skills().unwrap(), "");

        store.create("alpha", "Alpha skill content.").unwrap();
        store.create("beta", "Beta skill content.").unwrap();

        let bootstrap = store.bootstrap_skills().unwrap();
        assert!(bootstrap.contains("Active Skills"));
        assert!(bootstrap.contains("### alpha"));
        assert!(bootstrap.contains("Alpha skill content."));
        assert!(bootstrap.contains("### beta"));
    }
}
