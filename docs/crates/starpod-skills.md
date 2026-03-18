# starpod-skills

[AgentSkills](https://agentskills.io)-compatible skill system with progressive disclosure.

## Overview

Skills are `SKILL.md` files with YAML frontmatter (`name`, `description`) and a markdown body containing instructions. The store implements a two-tier loading strategy:

1. **Catalog** — name + description only (~50-100 tokens/skill) for the system prompt
2. **Activation** — full instructions loaded on demand via `activate_skill()`

## API

```rust
use starpod_skills::SkillStore;

let store = SkillStore::new(&data_dir)?;

// Create with frontmatter fields
store.create("code-review", "Review code for issues.", "Check for bugs.")?;

// Update description and body
store.update("code-review", "Review code thoroughly.", "New instructions.")?;

// Delete
store.delete("code-review")?;

// Read
let skill = store.get("code-review")?;         // Option<Skill>
let skills = store.list()?;                     // Vec<Skill>
let names = store.skill_names()?;               // Vec<String>

// Progressive disclosure
let catalog_xml = store.skill_catalog()?;       // XML for system prompt
let activated = store.activate_skill("code-review")?; // Full instructions
```

## Types

### Skill

```rust
pub struct Skill {
    pub name: String,              // Skill identifier (directory name)
    pub description: String,       // What the skill does (from frontmatter)
    pub body: String,              // Markdown instructions (after frontmatter)
    pub raw_content: String,       // Full SKILL.md content
    pub created_at: String,        // ISO 8601 timestamp
    pub skill_dir: PathBuf,        // Absolute path to skill directory
    pub compatibility: Option<String>,  // Environment requirements
    pub metadata: HashMap<String, String>, // Arbitrary key-value pairs
    pub allowed_tools: Option<String>,  // Pre-approved tools (experimental)
}
```

### SkillFrontmatter

```rust
pub struct SkillFrontmatter {
    pub name: String,              // Required
    pub description: String,       // Required
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: HashMap<String, String>,
    pub allowed_tools: Option<String>,  // YAML key: "allowed-tools"
}
```

## SKILL.md Format

```markdown
---
name: skill-name
description: What this skill does and when to use it.
license: MIT
compatibility: Requires git, docker
metadata:
  author: example-org
  version: "1.0"
allowed-tools: Bash(git:*) Read
---

# Instructions

Step-by-step instructions for the agent...
```

## Storage

```
.starpod/skills/
└── <name>/
    ├── SKILL.md        # Required
    ├── scripts/        # Optional: executable code
    ├── references/     # Optional: documentation
    └── assets/         # Optional: templates, data
```

## Name Validation

Follows the AgentSkills spec:

- 1-64 characters
- Lowercase letters, digits, and hyphens only
- Must not start or end with a hyphen
- Must not contain consecutive hyphens (`--`)
- Must not contain path separators, `..`, or leading dots
- Must match the parent directory name

## Progressive Disclosure

### Catalog (`skill_catalog()`)

Returns compact XML injected into the system prompt:

```xml
<available_skills>
  <skill>
    <name>code-review</name>
    <description>Review code for bugs and style issues.</description>
  </skill>
</available_skills>
```

### Activation (`activate_skill()`)

Returns full instructions with resource listing:

```xml
<skill_content name="code-review">
Check for error handling, edge cases, and security.

<skill_resources>
  <file>scripts/lint.sh</file>
  <file>references/style-guide.md</file>
</skill_resources>
</skill_content>
```

## Backward Compatibility

Skills without YAML frontmatter are loaded with:
- `name` = directory name
- `description` = first line of content (truncated to 120 chars)
- `body` = full file content

## Tests

31 unit tests + 1 doc-test covering:
- Frontmatter parsing (minimal, all fields, no frontmatter, malformed YAML, edge cases)
- CRUD operations (create, get, list, update, delete, duplicates, errors)
- Name validation (all AgentSkills spec rules, max length)
- Progressive disclosure (catalog format, XML escaping, activation with/without resources)
- Backward compatibility (no frontmatter, empty content, long descriptions)
- Helper functions (xml_escape, list_skill_resources)
