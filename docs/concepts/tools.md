# Agent Tools

The agent has access to **built-in tools** from the Claude Agent SDK plus **21 custom tools** provided by Starpod.

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

### Environment

| Tool | Input | Description |
|------|-------|-------------|
| `EnvGet` | `key` | Look up an environment variable by key |

Environment variables from the vault are injected into the process at serve time, making them available both through `EnvGet` and as real env vars in Bash/SSH commands. The system prompt lists which vars are available. System keys (LLM provider keys, service tokens) are blocked by `EnvGet` and stripped from Bash child processes.

### Files

| Tool | Input | Description |
|------|-------|-------------|
| `FileRead` | `path` | Read a file from the agent's filesystem sandbox |
| `FileWrite` | `path`, `content` | Write a file to the agent's filesystem sandbox |
| `FileList` | `path` (optional) | List files and directories in the sandbox |
| `FileDelete` | `path` | Delete a file from the sandbox |
| `Attach` | `path` | Attach a sandbox file for delivery to the user (see below) |

### Attach

The `Attach` tool sends a file to the user through their current channel. After the agent generates or locates a file in its sandbox, calling `Attach` queues it for delivery:

- **Web UI** — delivered as a WebSocket `attachment` message; images render inline, other files show as download links.
- **Telegram** — images sent via `send_photo`, everything else via `send_document`.
- **CLI** — files remain in the sandbox (already accessible on disk).

The tool validates sandbox paths the same way `FileRead` does (no `..` traversal, no absolute paths, no `.starpod/` access). Files must be under 20 MB. MIME type is inferred from the file extension.

Multiple files can be attached in a single turn — they accumulate and are delivered after the agent finishes responding.

### Skills

| Tool | Input | Description |
|------|-------|-------------|
| `SkillActivate` | `name` | Load a skill's full instructions into context |
| `SkillCreate` | `name`, `description`, `body` | Create a new AgentSkills-compatible skill |
| `SkillUpdate` | `name`, `description`, `body` | Update an existing skill's description and instructions |
| `SkillDelete` | `name` | Delete a skill |
| `SkillList` | — | List all active skills |

### Scheduling

| Tool | Input | Description |
|------|-------|-------------|
| `CronAdd` | `name`, `prompt`, `schedule`, `delete_after_run`, `max_retries`, `timeout_secs`, `session_mode` | Create a scheduled job |
| `CronList` | — | List all jobs with next run times |
| `CronRemove` | `name` | Remove a job |
| `CronRuns` | `name`, `limit` | View execution history |
| `CronRun` | `name` | Immediately execute a cron job (manual trigger) |
| `CronUpdate` | `name`, `prompt`, `enabled`, `max_retries`, `timeout_secs`, `session_mode` | Update properties of an existing job |

### Heartbeat

| Tool | Input | Description |
|------|-------|-------------|
| `HeartbeatWake` | `mode` (`"now"` or `"next"`), `message` | Wake the heartbeat system outside its normal cycle |

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
