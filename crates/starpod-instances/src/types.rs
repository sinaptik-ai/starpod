use serde::{Deserialize, Serialize};

/// Status of a remote instance — mirrors Spawner's `InstanceStatus` enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceStatus {
    Pending,
    Provisioning,
    Running,
    Stopping,
    Stopped,
    Starting,
    Restarting,
    Deleting,
    Deleted,
    Error,
}

impl std::fmt::Display for InstanceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Provisioning => write!(f, "provisioning"),
            Self::Running => write!(f, "running"),
            Self::Stopping => write!(f, "stopping"),
            Self::Stopped => write!(f, "stopped"),
            Self::Starting => write!(f, "starting"),
            Self::Restarting => write!(f, "restarting"),
            Self::Deleting => write!(f, "deleting"),
            Self::Deleted => write!(f, "deleted"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Metadata for a remote instance — mirrors Spawner's `InstanceResponse`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub status: InstanceStatus,
    pub agent_id: String,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub gcp_instance_name: Option<String>,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub machine_type: Option<String>,
    #[serde(default)]
    pub ip_address: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub email_address: Option<String>,
    #[serde(default)]
    pub starpod_api_key: Option<String>,
    #[serde(default)]
    pub secret_overrides: Option<serde_json::Value>,
    pub created_at: String,
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
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_type: Option<String>,
}
