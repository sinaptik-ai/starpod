pub mod config;
pub mod error;
pub mod instance;
pub mod types;
pub mod workspace;

pub use config::{
    AttachmentsConfig, ChannelsConfig, CompactionConfig, CronConfig, FollowupMode,
    MemoryConfig, StarpodConfig,
    ProviderConfig, ProvidersConfig, ReasoningEffort, TelegramChannelConfig,
};
pub use error::{StarpodError, Result};
pub use instance::{EnvSource, apply_blueprint};
pub use types::{Attachment, ChatMessage, ChatResponse, ChatUsage, MAX_ATTACHMENT_SIZE};
pub use workspace::{
    AgentConfig, Mode, ResolvedPaths, UserContext, WorkspaceConfig,
    detect_mode, detect_mode_from, load_agent_config, load_env, reload_agent_config,
};
