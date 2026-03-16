pub mod config;
pub mod error;
pub mod types;
pub mod workspace;

pub use config::{
    AttachmentsConfig, ChannelsConfig, CompactionConfig, CronConfig, FollowupMode,
    MemoryConfig, StarpodConfig,
    ProviderConfig, ProvidersConfig, ReasoningEffort, TelegramChannelConfig,
};
pub use error::{StarpodError, Result};
pub use types::{Attachment, ChatMessage, ChatResponse, ChatUsage, MAX_ATTACHMENT_SIZE};
pub use workspace::{
    AgentConfig, Mode, ResolvedPaths, WorkspaceConfig,
    detect_mode, detect_mode_from, load_agent_config, reload_agent_config,
};
