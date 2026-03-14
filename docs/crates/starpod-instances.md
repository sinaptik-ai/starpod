# starpod-instances

HTTP client for remote instance management, log streaming, SSH access, and health monitoring.

## InstanceClient

```rust
use starpod_instances::InstanceClient;

let client = InstanceClient::new("https://api.example.com", Some("api-key"));

// CRUD
let instance = client.create_instance(CreateInstanceRequest {
    name: Some("my-bot".into()),
    region: Some("us-east-1".into()),
}).await?;
let instances = client.list_instances().await?;
let instance = client.get_instance("inst_abc123").await?;
client.kill_instance("inst_abc123").await?;

// Lifecycle
client.pause_instance("inst_abc123").await?;
client.restart_instance("inst_abc123").await?;

// Logs (streaming NDJSON)
let mut stream = client.stream_logs("inst_abc123", Some(50)).await?;
while let Some(entry) = stream.next().await {
    println!("{}: {}", entry.level, entry.message);
}

// SSH
let ssh = client.get_ssh_info("inst_abc123").await?;
println!("ssh {}@{} -p {}", ssh.user, ssh.host, ssh.port);

// Health
let health = client.get_health("inst_abc123").await?;
println!("CPU: {}%, Memory: {} MB", health.cpu_percent, health.memory_mb);
```

## HealthMonitor

Background health polling with auto-restart on stale heartbeats.

```rust
use starpod_instances::HealthMonitor;
use std::time::Duration;

let monitor = HealthMonitor::new(client, "inst_abc123")
    .with_interval(Duration::from_secs(30))       // Poll every 30s (default)
    .with_heartbeat_timeout(Duration::from_secs(90)) // Stale after 90s (default)
    .on_status_change(|id, old, new| {
        println!("{id}: {old:?} → {new:?}");
    });

let shutdown = monitor.start().await;
// Send to stop: shutdown.send(()).ok();
```

## Types

```rust
pub enum InstanceStatus {
    Creating,
    Running,
    Paused,
    Stopped,
    Error,
}

pub struct Instance {
    pub id: String,
    pub name: Option<String>,
    pub status: InstanceStatus,
    pub region: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub health: Option<HealthInfo>,
}

pub struct HealthInfo {
    pub cpu_percent: f64,
    pub memory_mb: u64,
    pub disk_mb: u64,
    pub last_heartbeat: String,
    pub uptime_secs: u64,
}

pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

pub struct SshInfo {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub private_key: Option<String>,
}

pub struct CreateInstanceRequest {
    pub name: Option<String>,
    pub region: Option<String>,
}
```

## Tests

16 unit tests.
