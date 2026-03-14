# Skills

Skills are **markdown instruction files** injected into the agent's system prompt on every turn. They let you teach the agent reusable behaviors without modifying code.

## How It Works

1. A skill is a markdown file at `.starpod/data/skills/<name>/SKILL.md`
2. On every turn, all active skills are appended to the system prompt
3. The agent can create, update, and delete skills at runtime

## Creating Skills

### Via CLI

```bash
# Inline content
starpod agent skills create "code-review" --content "When reviewing code, always check for:
- Error handling
- Edge cases
- Performance implications
- Security vulnerabilities"

# From a file
starpod agent skills create "code-review" --file code-review-instructions.md
```

### Via the Agent

Ask during a conversation:

> "Create a skill called 'commit-messages' that reminds you to write conventional commit messages with a scope prefix."

The agent uses `SkillCreate` to save it. Takes effect immediately on subsequent turns.

## Managing Skills

```bash
starpod agent skills list              # List all skills
starpod agent skills show code-review  # View a skill
starpod agent skills delete code-review # Delete a skill
```

## Agent Tools

| Tool | Description |
|------|-------------|
| `SkillCreate` | Create a new skill |
| `SkillUpdate` | Update an existing skill |
| `SkillDelete` | Delete a skill |
| `SkillList` | List all active skills |

## Examples

### Daily standup
```markdown
When I ask for a standup summary:
1. Search memory for what was discussed yesterday
2. Check cron job results from overnight tasks
3. Format as: Done / In Progress / Blocked
```

### Code style
```markdown
When writing Rust code:
- Use `thiserror` for error types
- Prefer `impl Into<String>` over `&str` for function parameters
- Always add `#[must_use]` to functions returning `Result`
```

### Response format
```markdown
Always respond in this format:
- Lead with the answer
- Follow with explanation if needed
- Include code examples when relevant
- Keep responses concise
```

## Storage

```
.starpod/data/skills/
├── code-review/
│   └── SKILL.md
├── daily-standup/
│   └── SKILL.md
└── commit-messages/
    └── SKILL.md
```

::: info
Skill names cannot contain path separators, `..`, or leading dots. They're used directly as directory names.
:::
