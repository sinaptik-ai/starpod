use std::time::Duration;

use futures::StreamExt;
use reqwest::Client;
use tokio_stream::Stream;
use tracing::debug;

use starpod_core::{StarpodError, Result};

use crate::types::*;

/// HTTP client for the Starpod instance backend API.
#[derive(Clone)]
pub struct InstanceClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl InstanceClient {
    /// Create a new client pointing at the given backend URL (30s default timeout).
    pub fn new(base_url: &str, api_key: Option<String>) -> Result<Self> {
        Self::new_with_timeout(base_url, api_key, 30)
    }

    /// Create a new client with a custom HTTP timeout (in seconds).
    pub fn new_with_timeout(base_url: &str, api_key: Option<String>, timeout_secs: u64) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| StarpodError::Config(format!("HTTP client error: {}", e)))?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{}", self.base_url, path)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            req.header("Authorization", format!("Bearer {}", key))
        } else {
            req
        }
    }

    /// Resolve a (possibly short) instance ID to a full UUID.
    ///
    /// If `id` is already a valid full UUID it is returned as-is.  Otherwise we
    /// list all instances and look for a unique prefix match.
    pub async fn resolve_id(&self, id: &str) -> Result<String> {
        // Already a full UUID?
        if uuid::Uuid::try_parse(id).is_ok() {
            return Ok(id.to_string());
        }
        let instances = self.list_instances().await?;
        let matches: Vec<&Instance> = instances
            .iter()
            .filter(|inst| inst.id.starts_with(id))
            .collect();
        match matches.len() {
            0 => Err(StarpodError::Channel(format!(
                "No instance found matching prefix '{}'",
                id
            ))),
            1 => Ok(matches[0].id.clone()),
            n => Err(StarpodError::Channel(format!(
                "Ambiguous prefix '{}' matches {} instances — be more specific",
                id, n
            ))),
        }
    }

    /// Create a new remote instance.
    pub async fn create_instance(&self, req: &CreateInstanceRequest) -> Result<Instance> {
        debug!(url = %self.url("/instances"), "Creating instance");
        let resp = self
            .auth(self.client.post(self.url("/instances")))
            .json(req)
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to create instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Create instance failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Instance>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// List all instances.
    pub async fn list_instances(&self) -> Result<Vec<Instance>> {
        debug!(url = %self.url("/instances"), "Listing instances");
        let resp = self
            .auth(self.client.get(self.url("/instances")))
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to list instances: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "List instances failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Vec<Instance>>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// Get a single instance by ID.
    pub async fn get_instance(&self, id: &str) -> Result<Instance> {
        let resp = self
            .auth(self.client.get(self.url(&format!("/instances/{}", id))))
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to get instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Get instance failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Instance>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// Destroy (permanently delete) an instance and all its runtime state.
    pub async fn destroy_instance(&self, id: &str) -> Result<()> {
        let resp = self
            .auth(
                self.client
                    .delete(self.url(&format!("/instances/{}", id))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to destroy instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Destroy instance failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Stop a running instance (preserves disk).
    pub async fn stop_instance(&self, id: &str) -> Result<()> {
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/instances/{}/stop", id))),
            )
            .header("content-length", "0")
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to stop instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Stop instance failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Start a stopped instance.
    pub async fn start_instance(&self, id: &str) -> Result<()> {
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/instances/{}/start", id))),
            )
            .header("content-length", "0")
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to start instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Start instance failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Restart a running or stopped instance (stop + start).
    pub async fn restart_instance(&self, id: &str) -> Result<()> {
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/instances/{}/restart", id))),
            )
            .header("content-length", "0")
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to restart instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Restart instance failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Stream logs from a running instance.
    ///
    /// The backend returns plain text (one log line per newline). Each line is
    /// returned as a `LogEntry` with `level` and `timestamp` parsed from common
    /// log formats when possible, falling back to raw text.
    pub async fn stream_logs(
        &self,
        id: &str,
        tail: Option<usize>,
    ) -> Result<impl Stream<Item = Result<LogEntry>>> {
        let mut url = self.url(&format!("/instances/{}/logs", id));
        if let Some(n) = tail {
            url.push_str(&format!("?lines={}", n));
        }

        let resp = self
            .auth(self.client.get(&url))
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to stream logs: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Stream logs failed ({}): {}",
                status, body
            )));
        }

        let stream = resp.bytes_stream();
        let mut buffer = String::new();

        let log_stream = stream.filter_map(move |chunk| {
            let entries = match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    let mut results = Vec::new();
                    while let Some(pos) = buffer.find('\n') {
                        let line: String = buffer.drain(..=pos).collect();
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        // Try JSON first, then fall back to plain text parsing
                        let entry = if let Ok(e) = serde_json::from_str::<LogEntry>(line) {
                            e
                        } else {
                            LogEntry::from_plain(line)
                        };
                        results.push(Ok(entry));
                    }
                    results
                }
                Err(e) => {
                    vec![Err(StarpodError::Channel(format!("Stream error: {}", e)))]
                }
            };

            let stream = futures::stream::iter(entries);
            std::future::ready(Some(stream))
        });

        Ok(log_stream.flatten())
    }

    /// Get SSH connection info for an instance.
    pub async fn get_ssh_info(&self, id: &str) -> Result<SshInfo> {
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/instances/{}/ssh", id))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to get SSH info: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Get SSH info failed ({}): {}",
                status, body
            )));
        }

        resp.json::<SshInfo>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// Get health/resource info for an instance.
    pub async fn get_health(&self, id: &str) -> Result<HealthInfo> {
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/instances/{}/health", id))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to get health: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Get health failed ({}): {}",
                status, body
            )));
        }

        resp.json::<HealthInfo>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn setup() -> (MockServer, InstanceClient) {
        let server = MockServer::start().await;
        let client =
            InstanceClient::new(&server.uri(), Some("test-key".to_string())).unwrap();
        (server, client)
    }

    fn sample_instance() -> Instance {
        Instance {
            id: "a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string(),
            status: InstanceStatus::Running,
            agent_id: "f0e1d2c3-b4a5-6789-0fed-cba987654321".to_string(),
            organization_id: None,
            name: None,
            description: None,
            gcp_instance_name: Some("agent-a1b2c3d4".to_string()),
            zone: Some("europe-west4-a".to_string()),
            machine_type: Some("e2-medium".to_string()),
            ip_address: Some("34.90.1.2".to_string()),
            error_message: None,
            email_address: None,
            starpod_api_key: None,
            secret_overrides: None,
            created_at: "2025-03-10T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn test_create_instance() {
        let (server, client) = setup().await;
        let inst = sample_instance();

        Mock::given(method("POST"))
            .and(path("/api/v1/instances"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(201).set_body_json(&inst))
            .mount(&server)
            .await;

        let req = CreateInstanceRequest {
            agent_id: "f0e1d2c3-b4a5-6789-0fed-cba987654321".into(),
            name: None,
            description: None,
            zone: None,
            machine_type: None,
        };
        let result = client.create_instance(&req).await.unwrap();
        assert_eq!(result.id, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
        assert_eq!(result.status, InstanceStatus::Running);
    }

    #[tokio::test]
    async fn test_list_instances() {
        let (server, client) = setup().await;
        let instances = vec![sample_instance()];

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&instances))
            .mount(&server)
            .await;

        let result = client.list_instances().await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    }

    #[tokio::test]
    async fn test_get_instance() {
        let (server, client) = setup().await;
        let inst = sample_instance();

        Mock::given(method("GET"))
            .and(path("/api/v1/instances/a1b2c3d4-e5f6-7890-abcd-ef1234567890"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&inst))
            .mount(&server)
            .await;

        let result = client.get_instance("a1b2c3d4-e5f6-7890-abcd-ef1234567890").await.unwrap();
        assert_eq!(result.id, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
        assert_eq!(result.zone, Some("europe-west4-a".to_string()));
    }

    #[tokio::test]
    async fn test_destroy_instance() {
        let (server, client) = setup().await;

        Mock::given(method("DELETE"))
            .and(path("/api/v1/instances/a1b2c3d4"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        client.destroy_instance("a1b2c3d4").await.unwrap();
    }

    #[tokio::test]
    async fn test_stop_instance() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances/a1b2c3d4/stop"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        client.stop_instance("a1b2c3d4").await.unwrap();
    }

    #[tokio::test]
    async fn test_start_instance() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances/a1b2c3d4/start"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        client.start_instance("a1b2c3d4").await.unwrap();
    }

    #[tokio::test]
    async fn test_restart_instance() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances/a1b2c3d4/restart"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        client.restart_instance("a1b2c3d4").await.unwrap();
    }

    #[tokio::test]
    async fn test_get_ssh_info() {
        let (server, client) = setup().await;
        let ssh = SshInfo {
            host: "10.0.0.1".to_string(),
            port: 22,
            user: "starpod".to_string(),
            private_key: None,
        };

        Mock::given(method("GET"))
            .and(path("/api/v1/instances/a1b2c3d4/ssh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&ssh))
            .mount(&server)
            .await;

        let result = client.get_ssh_info("a1b2c3d4").await.unwrap();
        assert_eq!(result.host, "10.0.0.1");
        assert_eq!(result.user, "starpod");
    }

    #[tokio::test]
    async fn test_get_health() {
        let (server, client) = setup().await;
        let health = HealthInfo {
            cpu_percent: 23.5,
            memory_mb: 512,
            disk_mb: 10240,
            last_heartbeat: 1710003600,
            uptime_secs: 3600,
        };

        Mock::given(method("GET"))
            .and(path("/api/v1/instances/a1b2c3d4/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&health))
            .mount(&server)
            .await;

        let result = client.get_health("a1b2c3d4").await.unwrap();
        assert_eq!(result.memory_mb, 512);
        assert!((result.cpu_percent - 23.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_create_instance_error() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("Internal Server Error"),
            )
            .mount(&server)
            .await;

        let req = CreateInstanceRequest {
            agent_id: "test-agent".into(),
            name: None,
            description: None,
            zone: None,
            machine_type: None,
        };
        let result = client.create_instance(&req).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("500"));
    }

    #[tokio::test]
    async fn test_list_instances_empty() {
        let (server, client) = setup().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&Vec::<Instance>::new()))
            .mount(&server)
            .await;

        let result = client.list_instances().await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_stream_logs() {
        let (server, client) = setup().await;

        let log1 = LogEntry {
            timestamp: 1710000000,
            level: "info".to_string(),
            message: "Server started".to_string(),
        };
        let log2 = LogEntry {
            timestamp: 1710000001,
            level: "debug".to_string(),
            message: "Accepted connection".to_string(),
        };

        let body = format!(
            "{}\n{}\n",
            serde_json::to_string(&log1).unwrap(),
            serde_json::to_string(&log2).unwrap()
        );

        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/instances/a1b2c3d4/logs.*"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let stream = client.stream_logs("a1b2c3d4", Some(100)).await.unwrap();
        let entries: Vec<LogEntry> = stream
            .filter_map(|r| std::future::ready(r.ok()))
            .collect()
            .await;

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "Server started");
        assert_eq!(entries[1].level, "debug");
    }

    #[tokio::test]
    async fn test_new_with_timeout() {
        let server = MockServer::start().await;
        let client =
            InstanceClient::new_with_timeout(&server.uri(), Some("test-key".to_string()), 60);
        assert!(client.is_ok());

        // Verify it works by making a request
        let client = client.unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&Vec::<Instance>::new()))
            .mount(&server)
            .await;

        let result = client.list_instances().await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_no_auth_header_when_no_key() {
        let server = MockServer::start().await;
        let client = InstanceClient::new(&server.uri(), None).unwrap();

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&Vec::<Instance>::new()))
            .mount(&server)
            .await;

        let result = client.list_instances().await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_destroy_instance_error() {
        let (server, client) = setup().await;

        Mock::given(method("DELETE"))
            .and(path("/api/v1/instances/bad-id"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .mount(&server)
            .await;

        let result = client.destroy_instance("bad-id").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("404"));
    }

    #[tokio::test]
    async fn test_stop_instance_error() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances/bad-id/stop"))
            .respond_with(ResponseTemplate::new(409).set_body_string("Cannot stop"))
            .mount(&server)
            .await;

        let result = client.stop_instance("bad-id").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("409"));
    }

    #[tokio::test]
    async fn test_start_instance_error() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances/bad-id/start"))
            .respond_with(ResponseTemplate::new(409).set_body_string("Cannot start"))
            .mount(&server)
            .await;

        let result = client.start_instance("bad-id").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("409"));
    }

    #[tokio::test]
    async fn test_restart_instance_error() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/instances/bad-id/restart"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal error"))
            .mount(&server)
            .await;

        let result = client.restart_instance("bad-id").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resolve_id_full_uuid() {
        let (server, client) = setup().await;

        // Full UUID should be returned as-is without any API calls
        let result = client
            .resolve_id("a1b2c3d4-e5f6-7890-abcd-ef1234567890")
            .await
            .unwrap();
        assert_eq!(result, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    }

    #[tokio::test]
    async fn test_resolve_id_short_prefix() {
        let (server, client) = setup().await;
        let instances = vec![sample_instance()];

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&instances))
            .mount(&server)
            .await;

        let result = client.resolve_id("a1b2c3d4").await.unwrap();
        assert_eq!(result, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    }

    #[tokio::test]
    async fn test_resolve_id_no_match() {
        let (server, client) = setup().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&Vec::<Instance>::new()))
            .mount(&server)
            .await;

        let result = client.resolve_id("nonexist").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No instance found"));
    }
}
