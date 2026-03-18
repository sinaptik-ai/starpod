use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use reqwest::multipart;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use starpod_core::{Result, StarpodError};

/// Client for the Spawner deploy API (agents, files, secrets, instances).
#[derive(Clone)]
pub struct DeployClient {
    client: Client,
    base_url: String,
    api_key: String,
}

// ── API response types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub id: String,
    pub name: String,
    pub gcs_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadFilesResponse {
    pub uploaded: Vec<UploadedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretResponse {
    pub id: String,
    pub key: String,
    pub hint: Option<String>,
    pub agent_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceResponse {
    pub id: String,
    pub agent_id: String,
    pub status: String,
    pub zone: Option<String>,
    pub machine_type: Option<String>,
    pub ip_address: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSecretRequest {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInstanceRequest {
    pub agent_id: String,
    pub zone: Option<String>,
    pub machine_type: Option<String>,
}

/// Summary of what was deployed.
#[derive(Debug)]
pub struct DeploySummary {
    pub agent_id: String,
    pub agent_name: String,
    pub files_uploaded: usize,
    pub secrets_set: usize,
    pub instance: Option<InstanceResponse>,
}

/// Options for a deploy operation.
pub struct DeployOpts<'a> {
    pub agent_name: &'a str,
    pub agent_dir: &'a Path,
    pub skills_dir: Option<&'a Path>,
    pub env_vars: HashMap<String, String>,
    pub create_instance: bool,
    pub zone: Option<&'a str>,
    pub machine_type: Option<&'a str>,
    /// Callback invoked during instance provisioning polling (status updates).
    pub on_instance_poll: Option<Box<dyn FnMut(&InstanceResponse) + Send>>,
}

impl DeployClient {
    /// Create a new deploy client.
    pub fn new(base_url: &str, api_key: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| StarpodError::Config(format!("HTTP client error: {}", e)))?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{}", self.base_url, path)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("Authorization", format!("Bearer {}", self.api_key))
    }

    // ── Agent CRUD ────────────────────────────────────────────────────

    /// Create a new agent on the backend.
    pub async fn create_agent(&self, name: &str) -> Result<AgentResponse> {
        debug!(name = %name, "Creating agent");
        let resp = self
            .auth(self.client.post(self.url("/agents")))
            .json(&CreateAgentRequest {
                name: name.to_string(),
            })
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to create agent: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Create agent failed ({}): {}",
                status, body
            )));
        }

        resp.json::<AgentResponse>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// List all agents for the authenticated user.
    pub async fn list_agents(&self) -> Result<Vec<AgentResponse>> {
        let resp = self
            .auth(self.client.get(self.url("/agents")))
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to list agents: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "List agents failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Vec<AgentResponse>>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    // ── File upload ───────────────────────────────────────────────────

    /// Upload agent files via multipart. Each entry is (relative_path, file_bytes).
    pub async fn upload_files(
        &self,
        agent_id: &str,
        files: Vec<(String, Vec<u8>)>,
    ) -> Result<UploadFilesResponse> {
        debug!(agent_id = %agent_id, count = files.len(), "Uploading agent files");

        let mut form = multipart::Form::new();
        for (path, data) in files {
            let part = multipart::Part::bytes(data)
                .file_name(path.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| StarpodError::Channel(format!("Failed to create part: {}", e)))?;
            form = form.part(path, part);
        }

        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/agents/{}/files", agent_id))),
            )
            .multipart(form)
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to upload files: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Upload files failed ({}): {}",
                status, body
            )));
        }

        resp.json::<UploadFilesResponse>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    // ── Secrets ───────────────────────────────────────────────────────

    /// Set an agent-scoped secret.
    pub async fn set_secret(
        &self,
        agent_id: &str,
        key: &str,
        value: &str,
    ) -> Result<SecretResponse> {
        debug!(agent_id = %agent_id, key = %key, "Setting agent secret");
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/agents/{}/secrets", agent_id))),
            )
            .json(&CreateSecretRequest {
                key: key.to_string(),
                value: value.to_string(),
            })
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to set secret: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Set secret failed ({}): {}",
                status, body
            )));
        }

        resp.json::<SecretResponse>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    // ── Instance ──────────────────────────────────────────────────────

    /// Create a new instance for the given agent.
    pub async fn create_instance(
        &self,
        agent_id: &str,
        zone: Option<&str>,
        machine_type: Option<&str>,
    ) -> Result<InstanceResponse> {
        debug!(agent_id = %agent_id, "Creating instance");
        let resp = self
            .auth(self.client.post(self.url("/instances")))
            .json(&CreateInstanceRequest {
                agent_id: agent_id.to_string(),
                zone: zone.map(String::from),
                machine_type: machine_type.map(String::from),
            })
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

        resp.json::<InstanceResponse>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// Get an instance by ID.
    pub async fn get_instance(&self, instance_id: &str) -> Result<InstanceResponse> {
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/instances/{}", instance_id))),
            )
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

        resp.json::<InstanceResponse>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// Poll an instance until it reaches "running" or "error" status.
    /// Returns the final instance state. Calls `on_poll` after each check
    /// so the caller can display progress.
    pub async fn wait_for_instance_ready(
        &self,
        instance_id: &str,
        timeout: Duration,
        mut on_poll: impl FnMut(&InstanceResponse),
    ) -> Result<InstanceResponse> {
        let start = tokio::time::Instant::now();
        let mut delay = Duration::from_secs(3);

        loop {
            let inst = self.get_instance(instance_id).await?;
            on_poll(&inst);

            match inst.status.as_str() {
                "running" => return Ok(inst),
                "error" => {
                    let msg = inst
                        .error_message
                        .as_deref()
                        .unwrap_or("unknown error");
                    return Err(StarpodError::Channel(format!(
                        "Instance failed: {}",
                        msg
                    )));
                }
                "deleted" | "deleting" => {
                    return Err(StarpodError::Channel(
                        "Instance was deleted during provisioning".into(),
                    ));
                }
                _ => {} // pending, provisioning — keep waiting
            }

            if start.elapsed() > timeout {
                return Err(StarpodError::Channel(format!(
                    "Timed out waiting for instance to become ready (last status: {})",
                    inst.status
                )));
            }

            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(15));
        }
    }

    // ── High-level deploy ─────────────────────────────────────────────

    /// Deploy an agent from a local workspace directory.
    ///
    /// 1. Collects agent files from `agent_dir` (agent.toml, SOUL.md, etc.)
    /// 2. Collects shared skills from `skills_dir`
    /// 3. Collects user directories
    /// 4. Creates/reuses the agent on the backend
    /// 5. Uploads all files
    /// 6. Sets secrets from the provided env map
    /// 7. Optionally creates an instance
    pub async fn deploy(&self, opts: DeployOpts<'_>) -> Result<DeploySummary> {
        // Step 1: Create or find agent
        let agent = self.find_or_create_agent(opts.agent_name).await?;
        let agent_id = &agent.id;

        // Step 2: Collect all files to upload
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();

        // Agent blueprint files (agent.toml, SOUL.md, HEARTBEAT.md, BOOT.md, BOOTSTRAP.md, users/, files/)
        collect_files_recursive(opts.agent_dir, "", &mut files)?;

        // Shared skills → uploaded under skills/
        if let Some(sd) = opts.skills_dir {
            if sd.exists() {
                collect_files_recursive(sd, "skills/", &mut files)?;
            }
        }

        // Step 3: Upload files one at a time (the spawner uploads each to GCS sequentially)
        let total_files = files.len();
        for file in &files {
            self.upload_files(agent_id, vec![file.clone()]).await?;
        }

        // Step 4: Set secrets
        let secrets_count = opts.env_vars.len();
        for (key, value) in &opts.env_vars {
            self.set_secret(agent_id, key, value).await?;
        }

        // Step 5: Optionally create instance and wait for it to become ready
        let instance = if opts.create_instance {
            let created = self
                .create_instance(agent_id, opts.zone, opts.machine_type)
                .await?;

            let on_poll = opts.on_instance_poll.unwrap_or_else(|| Box::new(|_| {}));

            // Wait up to 15 minutes for the instance to reach "running"
            let ready = self
                .wait_for_instance_ready(
                    &created.id,
                    Duration::from_secs(900),
                    on_poll,
                )
                .await?;

            Some(ready)
        } else {
            None
        };

        Ok(DeploySummary {
            agent_id: agent_id.clone(),
            agent_name: opts.agent_name.to_string(),
            files_uploaded: total_files,
            secrets_set: secrets_count,
            instance,
        })
    }

    /// Find an existing agent by name or create a new one.
    async fn find_or_create_agent(&self, name: &str) -> Result<AgentResponse> {
        let agents = self.list_agents().await?;
        if let Some(existing) = agents.into_iter().find(|a| a.name == name) {
            debug!(agent_id = %existing.id, "Using existing agent");
            return Ok(existing);
        }
        self.create_agent(name).await
    }
}

/// Recursively collect all files under `dir`, prepending `prefix` to the relative path.
fn collect_files_recursive(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(dir).map_err(|e| {
        StarpodError::Config(format!("Failed to read directory {:?}: {}", dir, e))
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            StarpodError::Config(format!("Failed to read dir entry: {}", e))
        })?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files and known non-deployable items
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }

        let relative = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}{}", prefix, name)
        };

        if path.is_dir() {
            collect_files_recursive(&path, &format!("{}/", relative), out)?;
        } else {
            let data = std::fs::read(&path).map_err(|e| {
                StarpodError::Config(format!("Failed to read file {:?}: {}", path, e))
            })?;
            out.push((relative, data));
        }
    }

    Ok(())
}

/// Parse a .env file into a key-value map. Handles KEY=VALUE lines, ignoring comments and empty lines.
pub fn parse_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        StarpodError::Config(format!("Failed to read {:?}: {}", path, e))
    })?;

    let mut env = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            // Strip surrounding quotes if present
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(&value)
                .to_string();
            if !key.is_empty() {
                env.insert(key, value);
            }
        }
    }

    Ok(env)
}
