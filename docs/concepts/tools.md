# Agent Tools

The agent has access to **built-in tools** from the Claude Agent SDK plus **13 custom tools** provided by Starpod.

## Built-in Tools

These come from the `agent-sdk` and provide core capabilities:

| Tool | Description |
|------|-------------|
| `Read` | Read a file from disk |
| `Write` | Write content to a file |
| `Edit` | Make targeted edits to a file |
| `Bash` | Execute shell commands |
| `Glob` | Find files by pattern |
| `Grep` | Search file contents with regex |
| `WebSearch` | Search the web |
| `WebFetch` | Fetch a URL |

The `Bash` tool supports `run_in_background: true` for long-running processes (servers, watchers, etc.) that shouldn't block the agent.

## Custom Tools

### Memory

| Tool | Input | Description |
|------|-------|-------------|
| `MemorySearch` | `query`, `limit` | Full-text search across all memory files |
| `MemoryWrite` | `file`, `content` | Write or update a memory/knowledge file |
| `MemoryAppendDaily` | `text` | Append to today's daily log |

### Vault

| Tool | Input | Description |
|------|-------|-------------|
| `VaultGet` | `key` | Decrypt and retrieve a stored credential |
| `VaultSet` | `key`, `value` | Encrypt and store a credential |

### Skills

| Tool | Input | Description |
|------|-------|-------------|
| `SkillCreate` | `name`, `content` | Create a new skill |
| `SkillUpdate` | `name`, `content` | Update an existing skill |
| `SkillDelete` | `name` | Delete a skill |
| `SkillList` | — | List all active skills |

### Scheduling

| Tool | Input | Description |
|------|-------|-------------|
| `CronAdd` | `name`, `prompt`, `schedule`, `delete_after_run` | Create a scheduled job |
| `CronList` | — | List all jobs with next run times |
| `CronRemove` | `name` | Remove a job |
| `CronRuns` | `name`, `limit` | View execution history |

## CronAdd Schedule Format

The `schedule` parameter accepts three formats:

::: code-group
```json [Interval]
{
  "kind": "interval",
  "every_ms": 3600000
}
```

```json [Cron]
{
  "kind": "cron",
  "expr": "0 0 9 * * MON-FRI"
}
```

```json [One-shot]
{
  "kind": "one_shot",
  "at": "2026-03-14T15:00:00Z"
}
```
:::

## Tool Execution

The `agent-sdk` drives the agentic loop:

1. Claude receives the system prompt + conversation history
2. Claude decides to call one or more tools
3. The SDK executes the tools and feeds results back to Claude
4. Claude decides whether to call more tools or respond
5. Repeats up to `max_turns` (default: 30)

Custom tools are handled by an **external tool handler** registered on the `Options` builder. When Claude calls a tool, the handler checks if it matches a custom tool name. If it does, the handler executes it and returns the result. Otherwise, the SDK's built-in handler runs it.
