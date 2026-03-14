use std::time::Duration;

use futures::StreamExt;
use reqwest::Client;
use tokio_stream::Stream;
use tracing::debug;

use orion_core::{OrionError, Result};

use crate::types::*;

/// HTTP client for the Orion instance backend API.
#[derive(Clone)]
pub struct InstanceClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl InstanceClient {
    /// Create a new client pointing at the given backend URL.
    pub fn new(base_url: &str, api_key: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| OrionError::Config(format!("HTTP client error: {}", e)))?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            req.header("Authorization", format!("Bearer {}", key))
        } else {
            req
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
            .map_err(|e| OrionError::Channel(format!("Failed to create instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
                "Create instance failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Instance>()
            .await
            .map_err(|e| OrionError::Channel(format!("Invalid response: {}", e)))
    }

    /// List all instances.
    pub async fn list_instances(&self) -> Result<Vec<Instance>> {
        debug!(url = %self.url("/instances"), "Listing instances");
        let resp = self
            .auth(self.client.get(self.url("/instances")))
            .send()
            .await
            .map_err(|e| OrionError::Channel(format!("Failed to list instances: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
                "List instances failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Vec<Instance>>()
            .await
            .map_err(|e| OrionError::Channel(format!("Invalid response: {}", e)))
    }

    /// Get a single instance by ID.
    pub async fn get_instance(&self, id: &str) -> Result<Instance> {
        let resp = self
            .auth(self.client.get(self.url(&format!("/instances/{}", id))))
            .send()
            .await
            .map_err(|e| OrionError::Channel(format!("Failed to get instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
                "Get instance failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Instance>()
            .await
            .map_err(|e| OrionError::Channel(format!("Invalid response: {}", e)))
    }

    /// Kill (terminate) an instance.
    pub async fn kill_instance(&self, id: &str) -> Result<()> {
        let resp = self
            .auth(
                self.client
                    .delete(self.url(&format!("/instances/{}", id))),
            )
            .send()
            .await
            .map_err(|e| OrionError::Channel(format!("Failed to kill instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
                "Kill instance failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Pause a running instance.
    pub async fn pause_instance(&self, id: &str) -> Result<()> {
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/instances/{}/pause", id))),
            )
            .send()
            .await
            .map_err(|e| OrionError::Channel(format!("Failed to pause instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
                "Pause instance failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Restart a paused or running instance.
    pub async fn restart_instance(&self, id: &str) -> Result<()> {
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/instances/{}/restart", id))),
            )
            .send()
            .await
            .map_err(|e| OrionError::Channel(format!("Failed to restart instance: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
                "Restart instance failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Stream logs from a running instance (newline-delimited JSON).
    pub async fn stream_logs(
        &self,
        id: &str,
        tail: Option<usize>,
    ) -> Result<impl Stream<Item = Result<LogEntry>>> {
        let mut url = self.url(&format!("/instances/{}/logs", id));
        if let Some(n) = tail {
            url.push_str(&format!("?tail={}", n));
        }

        let resp = self
            .auth(self.client.get(&url))
            .send()
            .await
            .map_err(|e| OrionError::Channel(format!("Failed to stream logs: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
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
                        match serde_json::from_str::<LogEntry>(line) {
                            Ok(entry) => results.push(Ok(entry)),
                            Err(e) => results.push(Err(OrionError::Channel(format!(
                                "Invalid log entry: {}",
                                e
                            )))),
                        }
                    }
                    results
                }
                Err(e) => {
                    vec![Err(OrionError::Channel(format!("Stream error: {}", e)))]
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
            .map_err(|e| OrionError::Channel(format!("Failed to get SSH info: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
                "Get SSH info failed ({}): {}",
                status, body
            )));
        }

        resp.json::<SshInfo>()
            .await
            .map_err(|e| OrionError::Channel(format!("Invalid response: {}", e)))
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
            .map_err(|e| OrionError::Channel(format!("Failed to get health: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OrionError::Channel(format!(
                "Get health failed ({}): {}",
                status, body
            )));
        }

        resp.json::<HealthInfo>()
            .await
            .map_err(|e| OrionError::Channel(format!("Invalid response: {}", e)))
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
            id: "inst-001".to_string(),
            name: Some("my-agent".to_string()),
            status: InstanceStatus::Running,
            region: Some("us-east-1".to_string()),
            created_at: 1710000000,
            updated_at: 1710003600,
            health: None,
        }
    }

    #[tokio::test]
    async fn test_create_instance() {
        let (server, client) = setup().await;
        let inst = sample_instance();

        Mock::given(method("POST"))
            .and(path("/instances"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(201).set_body_json(&inst))
            .mount(&server)
            .await;

        let req = CreateInstanceRequest {
            name: Some("my-agent".into()),
            region: Some("us-east-1".into()),
        };
        let result = client.create_instance(&req).await.unwrap();
        assert_eq!(result.id, "inst-001");
        assert_eq!(result.status, InstanceStatus::Running);
    }

    #[tokio::test]
    async fn test_list_instances() {
        let (server, client) = setup().await;
        let instances = vec![sample_instance()];

        Mock::given(method("GET"))
            .and(path("/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&instances))
            .mount(&server)
            .await;

        let result = client.list_instances().await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "inst-001");
    }

    #[tokio::test]
    async fn test_get_instance() {
        let (server, client) = setup().await;
        let inst = sample_instance();

        Mock::given(method("GET"))
            .and(path("/instances/inst-001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&inst))
            .mount(&server)
            .await;

        let result = client.get_instance("inst-001").await.unwrap();
        assert_eq!(result.id, "inst-001");
        assert_eq!(result.name, Some("my-agent".to_string()));
    }

    #[tokio::test]
    async fn test_kill_instance() {
        let (server, client) = setup().await;

        Mock::given(method("DELETE"))
            .and(path("/instances/inst-001"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        client.kill_instance("inst-001").await.unwrap();
    }

    #[tokio::test]
    async fn test_pause_instance() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/instances/inst-001/pause"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        client.pause_instance("inst-001").await.unwrap();
    }

    #[tokio::test]
    async fn test_restart_instance() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/instances/inst-001/restart"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        client.restart_instance("inst-001").await.unwrap();
    }

    #[tokio::test]
    async fn test_get_ssh_info() {
        let (server, client) = setup().await;
        let ssh = SshInfo {
            host: "10.0.0.1".to_string(),
            port: 22,
            user: "orion".to_string(),
            private_key: None,
        };

        Mock::given(method("GET"))
            .and(path("/instances/inst-001/ssh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&ssh))
            .mount(&server)
            .await;

        let result = client.get_ssh_info("inst-001").await.unwrap();
        assert_eq!(result.host, "10.0.0.1");
        assert_eq!(result.user, "orion");
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
            .and(path("/instances/inst-001/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&health))
            .mount(&server)
            .await;

        let result = client.get_health("inst-001").await.unwrap();
        assert_eq!(result.memory_mb, 512);
        assert!((result.cpu_percent - 23.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_create_instance_error() {
        let (server, client) = setup().await;

        Mock::given(method("POST"))
            .and(path("/instances"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("Internal Server Error"),
            )
            .mount(&server)
            .await;

        let req = CreateInstanceRequest {
            name: None,
            region: None,
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
            .and(path("/instances"))
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
            .and(path_regex(r"/instances/inst-001/logs.*"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let stream = client.stream_logs("inst-001", Some(100)).await.unwrap();
        let entries: Vec<LogEntry> = stream
            .filter_map(|r| std::future::ready(r.ok()))
            .collect()
            .await;

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "Server started");
        assert_eq!(entries[1].level, "debug");
    }

    #[tokio::test]
    async fn test_no_auth_header_when_no_key() {
        let server = MockServer::start().await;
        let client = InstanceClient::new(&server.uri(), None).unwrap();

        Mock::given(method("GET"))
            .and(path("/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&Vec::<Instance>::new()))
            .mount(&server)
            .await;

        let result = client.list_instances().await.unwrap();
        assert!(result.is_empty());
    }
}
