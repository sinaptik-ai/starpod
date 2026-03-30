pub mod config;
pub mod config_migrate;
pub mod deploy_manifest;
pub mod error;
pub mod instance;
pub mod types;
pub mod workspace;

pub use config::{
    parse_model_spec, AttachmentsConfig, AuthConfig, BrowserConfig, ChannelsConfig,
    CompactionConfig, CronConfig, FollowupMode, FrontendConfig, InternetConfig, MemoryConfig,
    ProviderConfig, ProvidersConfig, ReasoningEffort, StarpodConfig, TelegramChannelConfig,
};
pub use error::{Result, StarpodError};
pub use instance::{apply_blueprint, build_standalone, create_ephemeral_instance, EnvSource};
pub use types::{Attachment, ChatMessage, ChatResponse, ChatUsage, MAX_ATTACHMENT_SIZE};
pub use workspace::{
    detect_mode, detect_mode_from, load_agent_config, load_env, reload_agent_config, AgentConfig,
    Mode, ResolvedPaths, UserContext, WorkspaceConfig,
};
