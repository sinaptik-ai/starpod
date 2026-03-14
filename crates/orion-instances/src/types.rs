use serde::{Deserialize, Serialize};

/// Status of a remote instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceStatus {
    Creating,
    Running,
    Paused,
    Stopped,
    Error,
}

impl std::fmt::Display for InstanceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Creating => write!(f, "creating"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Stopped => write!(f, "stopped"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Metadata for a remote instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub name: Option<String>,
    pub status: InstanceStatus,
    pub region: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub health: Option<HealthInfo>,
}

/// Health / resource usage information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthInfo {
    pub cpu_percent: f64,
    pub memory_mb: u64,
    pub disk_mb: u64,
    pub last_heartbeat: i64,
    pub uptime_secs: u64,
}

/// A single log entry from a running instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: i64,
    pub level: String,
    pub message: String,
}

/// SSH connection details returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshInfo {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Optional private key content (PEM) for ephemeral access.
    pub private_key: Option<String>,
}

/// Request body for creating an instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInstanceRequest {
    pub name: Option<String>,
    pub region: Option<String>,
}
