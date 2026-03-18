# Instances

Starpod can manage **remote cloud instances** through a backend API. You can create, list, pause, restart, and kill instances — plus stream logs, open SSH sessions, and monitor health with automatic restart on stale heartbeats.

## Configuration

Set the backend URL as an environment variable:

```bash
export STARPOD_INSTANCE_BACKEND_URL="https://api.starpod.example.com"
```

The API key for authentication is resolved from `providers.anthropic.api_key` or the `ANTHROPIC_API_KEY` environment variable.

## Instance Lifecycle

Each instance has a status that tracks its current state:

| Status | Description |
|--------|-------------|
| `Creating` | Instance is being provisioned |
| `Running` | Instance is active and healthy |
| `Paused` | Instance is suspended (can be restarted) |
| `Stopped` | Instance has been terminated |
| `Error` | Instance encountered a failure |

```
Creating → Running → Paused → Running (restart)
                   → Stopped (kill)
                   → Error
```

## Log Streaming

Logs are streamed as newline-delimited JSON from the backend. Each log entry contains a timestamp, level, and message. The CLI displays logs with colored output:

- **Error** — red
- **Warn** — yellow
- **Info** — green
- **Debug** — dim

```bash
starpod instance logs <id>            # Stream last 50 lines
starpod instance logs <id> --tail 100 # Stream last 100 lines
```

## SSH Access

Starpod fetches SSH connection info from the backend and spawns a native `ssh` process. If the backend provides an ephemeral private key, it is written to a temporary file with `0600` permissions and cleaned up after the session ends.

```bash
starpod instance ssh <id>
```

## Health Monitoring

The `HealthMonitor` polls instance health at a configurable interval (default: 30 seconds). If the heartbeat becomes stale (default timeout: 90 seconds), it automatically triggers a restart.

Health data includes:

| Metric | Description |
|--------|-------------|
| `cpu_percent` | CPU utilization percentage |
| `memory_mb` | Memory usage in MB |
| `disk_mb` | Disk usage in MB |
| `last_heartbeat` | Timestamp of last heartbeat |
| `uptime_secs` | Seconds since instance started |

### Status Change Callbacks

You can register callbacks that fire when an instance's status changes — useful for alerting or logging:

```rust
use starpod_instances::{InstanceClient, HealthMonitor};
use std::sync::Arc;

let client = InstanceClient::new("https://api.example.com", Some("api-key")).unwrap();
let monitor = HealthMonitor::new(client)
    .with_interval(Duration::from_secs(30))
    .with_heartbeat_timeout(Duration::from_secs(90))
    .on_status_change(Arc::new(|id, status, health| {
        println!("Instance {id} changed to {status:?}");
    }));

let shutdown = monitor.start();  // Not async — returns watch::Sender<()> directly
// ... later ...
let _ = shutdown.send(());  // Stop monitoring
```

## CLI

```bash
starpod instance create                         # Create a new instance
starpod instance create --name "my-bot" --region "us-east-1"
starpod instance list                           # List all instances
starpod instance kill <id>                      # Terminate an instance
starpod instance pause <id>                     # Suspend an instance
starpod instance restart <id>                   # Resume a paused instance
starpod instance logs <id> [--tail N]           # Stream logs (default: 50 lines)
starpod instance ssh <id>                       # SSH into an instance
starpod instance health <id>                    # Check instance health
```

## Gateway API

The gateway exposes instance management over HTTP. See the [API Reference](/api-reference/instances) for full details.
