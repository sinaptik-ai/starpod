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
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
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
    pub web_url: Option<String>,
    #[serde(default)]
    pub direct_url: Option<String>,
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

impl LogEntry {
    /// Parse a plain-text log line into a LogEntry.
    ///
    /// Recognises common patterns:
    /// - `2026-03-24T09:00:00Z INFO some message`
    /// - `[INFO] some message`
    /// - `INFO some message`
    /// - plain text (no level detected → level = "info")
    pub fn from_plain(line: &str) -> Self {
        // Try: `<timestamp> <LEVEL> <message>`  (e.g. tracing format)
        // Timestamp could be ISO-8601 or epoch.
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() >= 3 {
            if let Some(ts) = try_parse_timestamp(parts[0]) {
                let level = normalise_level(parts[1]);
                return Self {
                    timestamp: ts,
                    level,
                    message: parts[2].to_string(),
                };
            }
        }

        // Try: `[LEVEL] message`
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                let candidate = &line[1..end];
                let level = normalise_level(candidate);
                if level != "info" || candidate.eq_ignore_ascii_case("info") {
                    return Self {
                        timestamp: 0,
                        level,
                        message: line[end + 1..].trim().to_string(),
                    };
                }
            }
        }

        // Try: `LEVEL message` (just a bare level keyword at the start)
        if let Some((first, rest)) = line.split_once(' ') {
            let level = normalise_level(first);
            if level != "info" || first.eq_ignore_ascii_case("info") {
                return Self {
                    timestamp: 0,
                    level,
                    message: rest.to_string(),
                };
            }
        }

        // Fallback: treat whole line as info message
        Self {
            timestamp: 0,
            level: "info".to_string(),
            message: line.to_string(),
        }
    }
}

fn normalise_level(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "error" | "err" | "fatal" => "error".to_string(),
        "warn" | "warning" => "warn".to_string(),
        "info" => "info".to_string(),
        "debug" | "trace" => "debug".to_string(),
        _ => "info".to_string(),
    }
}

fn try_parse_timestamp(s: &str) -> Option<i64> {
    // Try epoch seconds
    if let Ok(ts) = s.parse::<i64>() {
        return Some(ts);
    }
    // Try ISO-8601 / RFC-3339
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    // Try common format without timezone (assume UTC)
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(dt.and_utc().timestamp());
    }
    None
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_type: Option<String>,
}
