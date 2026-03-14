use thiserror::Error;

/// Unified error type for the Starpod platform.
#[derive(Debug, Error)]
pub enum StarpodError {
    /// Configuration errors.
    #[error("Config error: {0}")]
    Config(String),

    /// Database / SQLite errors.
    #[error("Database error: {0}")]
    Database(String),

    /// File I/O errors.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Vault / encryption errors.
    #[error("Vault error: {0}")]
    Vault(String),

    /// Session management errors.
    #[error("Session error: {0}")]
    Session(String),

    /// Agent SDK errors.
    #[error("Agent error: {0}")]
    Agent(String),

    /// Skill errors.
    #[error("Skill error: {0}")]
    Skill(String),

    /// Cron / scheduling errors.
    #[error("Cron error: {0}")]
    Cron(String),

    /// Instance management errors.
    #[error("Instance error: {0}")]
    Instance(String),

    /// Channel / communication errors.
    #[error("Channel error: {0}")]
    Channel(String),

    /// Serialization errors.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Convenience type alias.
pub type Result<T> = std::result::Result<T, StarpodError>;
