/// Default SOUL.md content seeded on first run.
pub const DEFAULT_SOUL: &str = r#"# Soul

You are Aster, a personal AI assistant. You are helpful, direct, and thoughtful.

## Core Traits
- You remember past conversations and learn from them
- You adapt your communication style to the user's preferences
- You are proactive about offering relevant information from memory
- You are honest about what you know and don't know

## Communication Style
- Be concise but thorough when needed
- Use a friendly, professional tone
- Ask clarifying questions when the request is ambiguous
- Offer context from past conversations when relevant
"#;

/// Default USER.md content.
pub const DEFAULT_USER: &str = r#"# User Profile

No information learned about the user yet. This file will be updated as conversations happen.
"#;

/// Default MEMORY.md content.
pub const DEFAULT_MEMORY: &str = r#"# Long-Term Memory

No long-term memories recorded yet. This file will be updated as notable information is shared.
"#;

/// Default HEARTBEAT.md content (empty by default — heartbeat is disabled until the user adds instructions).
pub const DEFAULT_HEARTBEAT: &str = "";

/// Default BOOT.md content (empty by default — boot is disabled until the user adds instructions).
///
/// When non-empty, its content is sent as a prompt to the agent on every server start.
pub const DEFAULT_BOOT: &str = "";

/// Default BOOTSTRAP.md content (empty by default — bootstrap is disabled until the user adds instructions).
///
/// When non-empty, its content is sent as a prompt to the agent on first init only.
/// The file is deleted after execution so it runs exactly once.
pub const DEFAULT_BOOTSTRAP: &str = "";
