# starpod-skills

Markdown-based skill system for self-extending agent behavior.

## API

```rust
let store = SkillStore::new(&data_dir)?;

// CRUD
store.create("code-review", "Always check for...")?;
store.update("code-review", "Updated instructions")?;
store.delete("code-review")?;

// Read
let skill = store.get("code-review")?;   // Option<Skill>
let skills = store.list()?;              // Vec<Skill>

// System prompt injection
let prompt_text = store.bootstrap_skills()?;
```

## Skill Type

```rust
pub struct Skill {
    pub name: String,
    pub content: String,      // Raw markdown
    pub created_at: String,   // ISO 8601
}
```

## Storage

```
.starpod/data/skills/
└── <name>/
    └── SKILL.md
```

## Name Validation

- No empty names
- No path separators (`/`, `\`)
- No `..` or leading `.`
- Used directly as directory names

## Tests

9 unit tests.
