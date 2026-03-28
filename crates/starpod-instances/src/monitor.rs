use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::client::InstanceClient;
use crate::types::{HealthInfo, InstanceStatus};

/// Callback invoked when an instance's health status changes.
pub type HealthCallback = Arc<dyn Fn(String, InstanceStatus, Option<HealthInfo>) + Send + Sync>;

/// Background health monitor that periodically checks all instances.
pub struct HealthMonitor {
    client: InstanceClient,
    interval: Duration,
    heartbeat_timeout: Duration,
    on_status_change: Option<HealthCallback>,
}

impl HealthMonitor {
    pub fn new(client: InstanceClient) -> Self {
        Self {
            client,
            interval: Duration::from_secs(30),
            heartbeat_timeout: Duration::from_secs(90),
            on_status_change: None,
        }
    }

    /// Set the polling interval (default: 30s).
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Set the heartbeat timeout (default: 90s).
    /// If an instance's last heartbeat is older than this, it's considered unhealthy.
    pub fn with_heartbeat_timeout(mut self, timeout: Duration) -> Self {
        self.heartbeat_timeout = timeout;
        self
    }

    /// Set a callback for status changes.
    pub fn on_status_change(mut self, cb: HealthCallback) -> Self {
        self.on_status_change = Some(cb);
        self
    }

    /// Start the monitor loop. Returns a shutdown sender — drop it to stop.
    pub fn start(self) -> watch::Sender<()> {
        let (tx, mut rx) = watch::channel(());

        tokio::spawn(async move {
            info!(
                interval_secs = self.interval.as_secs(),
                "Health monitor started"
            );

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(self.interval) => {}
                    _ = rx.changed() => {
                        info!("Health monitor shutting down");
                        return;
                    }
                }

                self.check_all().await;
            }
        });

        tx
    }

    async fn check_all(&self) {
        let instances = match self.client.list_instances().await {
            Ok(list) => list,
            Err(e) => {
                warn!(error = %e, "Health monitor: failed to list instances");
                return;
            }
        };

        let now = chrono::Utc::now().timestamp();

        for inst in &instances {
            if inst.status != InstanceStatus::Running {
                continue;
            }

            match self.client.get_health(&inst.id).await {
                Ok(health) => {
                    let stale =
                        (now - health.last_heartbeat) > self.heartbeat_timeout.as_secs() as i64;
                    if stale {
                        warn!(
                            id = %inst.id,
                            last_heartbeat = health.last_heartbeat,
                            "Instance heartbeat stale — attempting restart"
                        );

                        if let Some(ref cb) = self.on_status_change {
                            cb(inst.id.clone(), InstanceStatus::Error, Some(health.clone()));
                        }

                        if let Err(e) = self.client.restart_instance(&inst.id).await {
                            error!(id = %inst.id, error = %e, "Auto-restart failed");
                        } else {
                            info!(id = %inst.id, "Auto-restart triggered");
                        }
                    }
                }
                Err(e) => {
                    warn!(id = %inst.id, error = %e, "Failed to get health");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Instance;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn running_instance(id: &str) -> Instance {
        Instance {
            id: id.to_string(),
            status: InstanceStatus::Running,
            agent_id: "test-agent".to_string(),
            organization_id: None,
            name: None,
            description: None,
            gcp_instance_name: None,
            zone: None,
            machine_type: None,
            ip_address: None,
            error_message: None,
            email_address: None,
            starpod_api_key: None,
            web_url: None,
            direct_url: None,
            secret_overrides: None,
            created_at: "2025-03-10T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn test_monitor_triggers_restart_on_stale_heartbeat() {
        let server = MockServer::start().await;
        let client = InstanceClient::new(&server.uri(), None).unwrap();

        let instances = vec![running_instance("inst-stale")];

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&instances))
            .mount(&server)
            .await;

        // Return a stale heartbeat (2 minutes ago)
        let stale_health = HealthInfo {
            cpu_percent: 10.0,
            memory_mb: 256,
            disk_mb: 5000,
            last_heartbeat: chrono::Utc::now().timestamp() - 120,
            uptime_secs: 600,
        };

        Mock::given(method("GET"))
            .and(path("/api/v1/instances/inst-stale/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&stale_health))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances/inst-stale/restart"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let monitor = HealthMonitor::new(client).with_heartbeat_timeout(Duration::from_secs(60));

        monitor.check_all().await;
        // The mock expectation verifies restart was called
    }

    #[tokio::test]
    async fn test_monitor_skips_healthy_instance() {
        let server = MockServer::start().await;
        let client = InstanceClient::new(&server.uri(), None).unwrap();

        let instances = vec![running_instance("inst-ok")];

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&instances))
            .mount(&server)
            .await;

        // Fresh heartbeat
        let fresh_health = HealthInfo {
            cpu_percent: 5.0,
            memory_mb: 128,
            disk_mb: 3000,
            last_heartbeat: chrono::Utc::now().timestamp(),
            uptime_secs: 300,
        };

        Mock::given(method("GET"))
            .and(path("/api/v1/instances/inst-ok/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&fresh_health))
            .mount(&server)
            .await;

        // No restart should be called
        Mock::given(method("POST"))
            .and(path_regex(r"/api/v1/instances/.*/restart"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let monitor = HealthMonitor::new(client).with_heartbeat_timeout(Duration::from_secs(60));

        monitor.check_all().await;
    }

    #[tokio::test]
    async fn test_monitor_skips_non_running_instances() {
        let server = MockServer::start().await;
        let client = InstanceClient::new(&server.uri(), None).unwrap();

        let instances = vec![Instance {
            id: "inst-paused".to_string(),
            status: InstanceStatus::Stopped,
            agent_id: "test-agent".to_string(),
            organization_id: None,
            name: None,
            description: None,
            gcp_instance_name: None,
            zone: None,
            machine_type: None,
            ip_address: None,
            error_message: None,
            email_address: None,
            starpod_api_key: None,
            web_url: None,
            direct_url: None,
            secret_overrides: None,
            created_at: "2025-03-10T00:00:00Z".to_string(),
        }];

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&instances))
            .mount(&server)
            .await;

        // Health should never be checked for paused instances
        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/instances/.*/health"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let monitor = HealthMonitor::new(client);
        monitor.check_all().await;
    }

    #[tokio::test]
    async fn test_monitor_callback_on_stale() {
        let server = MockServer::start().await;
        let client = InstanceClient::new(&server.uri(), None).unwrap();

        let instances = vec![running_instance("inst-cb")];

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&instances))
            .mount(&server)
            .await;

        let stale_health = HealthInfo {
            cpu_percent: 10.0,
            memory_mb: 256,
            disk_mb: 5000,
            last_heartbeat: chrono::Utc::now().timestamp() - 200,
            uptime_secs: 600,
        };

        Mock::given(method("GET"))
            .and(path("/api/v1/instances/inst-cb/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&stale_health))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances/inst-cb/restart"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();

        let monitor = HealthMonitor::new(client)
            .with_heartbeat_timeout(Duration::from_secs(60))
            .on_status_change(Arc::new(move |id, status, _health| {
                assert_eq!(id, "inst-cb");
                assert_eq!(status, InstanceStatus::Error);
                called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            }));

        monitor.check_all().await;
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
