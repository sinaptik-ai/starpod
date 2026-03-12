use thiserror::Error;

/// Errors that can occur during agent execution.
#[derive(Error, Debug)]
pub enum AgentError {
    #[error("API error: {0}")]
    Api(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Billing error: {0}")]
    BillingError(String),

    #[error("Rate limited: {0}")]
    RateLimited(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Process error: {0}")]
    Process(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Hook error: {0}")]
    Hook(String),

    #[error("MCP server error: {0}")]
    McpServer(String),

    #[error("Tool execution error: {0}")]
    ToolExecution(String),

    #[error("Permission denied: tool={tool}, reason={reason}")]
    PermissionDenied { tool: String, reason: String },

    #[error("Max turns exceeded: {0}")]
    MaxTurnsExceeded(u32),

    #[error("Max budget exceeded: ${0:.4}")]
    MaxBudgetExceeded(f64),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("Cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, AgentError>;
