# Skills

Skills are [AgentSkills](https://agentskills.io)-compatible instruction packages that extend the agent's capabilities. Each skill is a `SKILL.md` file with YAML frontmatter and markdown instructions, stored in its own directory.

## How It Works

Starpod implements **progressive disclosure** to keep prompts efficient:

1. **Catalog** — at startup, only skill names and descriptions are injected into the system prompt (~50-100 tokens per skill)
2. **Activation** — when a task matches a skill's description, the agent calls `SkillActivate` to load the full instructions
3. **Resources** — scripts, references, and assets bundled with a skill are loaded on demand

This means you can have many skills installed without paying the token cost of all their content on every turn.

## SKILL.md Format

Each skill is a directory containing a `SKILL.md` file with YAML frontmatter:

```markdown
---
name: code-review
description: Review code for bugs, security issues, and style. Use when asked to review code or PRs.
---

# Code Review

When reviewing code, always check for:
- Error handling and edge cases
- Security vulnerabilities (injection, XSS, etc.)
- Performance implications
- Consistent code style
```

### Required Fields

| Field | Constraints |
|-------|-------------|
| `name` | 1-64 chars, lowercase letters + digits + hyphens. Must match directory name. |
| `description` | 1-1024 chars. Describes what the skill does and when to use it. |

### Optional Fields

| Field | Description |
|-------|-------------|
| `license` | License name or reference to bundled LICENSE file |
| `compatibility` | Environment requirements (e.g. "Requires git, docker") |
| `metadata` | Arbitrary key-value pairs |
| `allowed-tools` | Space-delimited list of pre-approved tools (experimental) |

## Creating Skills

### Via CLI

```bash
# With inline instructions
starpod agent skills new "code-review" \
  --description "Review code for bugs and style issues." \
  --body "Check for error handling, edge cases, and security."

# From a file
starpod agent skills new "code-review" \
  --description "Review code for bugs and style issues." \
  --file code-review-instructions.md
```

### Via the Agent

Ask during a conversation:

> "Create a skill called 'commit-messages' that reminds you to write conventional commit messages with a scope prefix."

The agent uses `SkillCreate` to save it. The skill appears in the catalog immediately.

### Manually

Create the directory and file directly:

```bash
mkdir -p .starpod/data/skills/code-review
cat > .starpod/data/skills/code-review/SKILL.md << 'EOF'
---
name: code-review
description: Review code for bugs and style issues.
---

Check for error handling, edge cases, and security.
EOF
```

## Managing Skills

```bash
starpod agent skills list              # List all skills with descriptions
starpod agent skills show code-review  # View a skill's full content
starpod agent skills delete code-review # Delete a skill
```

## Agent Tools

| Tool | Description |
|------|-------------|
| `SkillActivate` | Load a skill's full instructions into context |
| `SkillCreate` | Create a new skill |
| `SkillUpdate` | Update an existing skill's description and instructions |
| `SkillDelete` | Delete a skill |
| `SkillList` | List all skills with descriptions |

## Bundled Resources

Skills can include supporting files that the agent loads on demand:

```
code-review/
├── SKILL.md              # Required: metadata + instructions
├── scripts/              # Optional: executable code
│   └── lint-check.sh
├── references/           # Optional: documentation
│   └── style-guide.md
└── assets/               # Optional: templates, data files
    └── checklist.json
```

When a skill is activated, the resource listing is included so the agent knows what's available.

## Examples

### Daily Standup
```markdown
---
name: daily-standup
description: Generate a standup summary from memory and recent activity.
---

When I ask for a standup summary:
1. Search memory for what was discussed yesterday
2. Check cron job results from overnight tasks
3. Format as: Done / In Progress / Blocked
```

### Commit Messages
```markdown
---
name: commit-messages
description: Write conventional commit messages with scope and issue references.
---

When writing commit messages:
- Use conventional commits (feat:, fix:, docs:, etc.)
- Include scope prefix (e.g., fix(core): ...)
- Keep first line under 50 characters
- Reference issue numbers if applicable
```

### Response Format
```markdown
---
name: response-format
description: Format responses concisely with answers first.
---

Always respond in this format:
- Lead with the answer
- Follow with explanation if needed
- Include code examples when relevant
- Keep responses concise
```

## Backward Compatibility

Skills without YAML frontmatter (plain markdown) continue to work. The directory name is used as the skill name, and the first line of content becomes the description.

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

## AgentSkills Compatibility

Starpod's skill format is compatible with the [AgentSkills](https://agentskills.io) open standard, used by Claude Code, Cursor, VS Code Copilot, Gemini CLI, and many other tools. Skills created for those tools can be dropped into `.starpod/data/skills/` and will work automatically.
