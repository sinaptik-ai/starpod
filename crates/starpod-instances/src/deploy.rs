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
    #[serde(default)]
    pub md5_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncManifestRequest {
    pub files: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifestResponse {
    pub to_upload: Vec<String>,
    pub to_download: Vec<UploadedFile>,
    pub to_delete_remote: Vec<String>,
    pub to_delete_local: Vec<String>,
}

/// Summary of a push or pull operation.
#[derive(Debug)]
pub struct SyncSummary {
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted_remote: usize,
    pub deleted_local: usize,
    pub unchanged: usize,
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
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub zone: Option<String>,
    pub machine_type: Option<String>,
    pub ip_address: Option<String>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub starpod_api_key: Option<String>,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub direct_url: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub zone: Option<String>,
    pub machine_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable_overrides: Option<HashMap<String, String>>,
}

// ── Deploy config types ──────────────────────────────────────────────────

/// Secret declaration status from deploy.toml readiness check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretStatusInfo {
    pub key: String,
    pub required: bool,
    #[serde(default)]
    pub description: String,
    pub present: bool,
    pub scope: Option<String>,
    pub hint: Option<String>,
    #[serde(default)]
    pub resolved_from: Option<String>,
}

/// Variable declaration status from deploy.toml readiness check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableStatusInfo {
    pub key: String,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub scope: String,
}

/// Deploy readiness response from `GET /agents/{id}/deploy-config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployReadiness {
    pub version: u32,
    pub variables: Vec<VariableStatusInfo>,
    pub secrets: Vec<SecretStatusInfo>,
    pub ready: bool,
    pub missing_required: Vec<String>,
}

impl DeployReadiness {
    /// Returns keys of optional secrets that are not present remotely but exist in the given local env map.
    pub fn missing_optional_in_env(&self, local_env: &HashMap<String, String>) -> Vec<String> {
        self.secrets
            .iter()
            .filter(|s| !s.present && !s.required && local_env.contains_key(s.key.as_str()))
            .map(|s| s.key.clone())
            .collect()
    }

    /// Returns keys of required secrets that are missing remotely but exist in the given local env map.
    pub fn missing_required_in_env(&self, local_env: &HashMap<String, String>) -> Vec<String> {
        self.missing_required
            .iter()
            .filter(|k| local_env.contains_key(k.as_str()))
            .cloned()
            .collect()
    }
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
    pub instance_name: Option<&'a str>,
    pub instance_description: Option<&'a str>,
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

    // ── Deploy Config ─────────────────────────────────────────────────

    /// Get the deploy config readiness for an agent.
    pub async fn get_deploy_config(&self, agent_id: &str) -> Result<Option<DeployReadiness>> {
        debug!(agent_id = %agent_id, "Fetching deploy config");
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/agents/{}/deploy-config", agent_id))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to get deploy config: {}", e)))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Get deploy config failed ({}): {}",
                status, body
            )));
        }

        let config = resp
            .json::<DeployReadiness>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))?;
        Ok(Some(config))
    }

    // ── Secrets ───────────────────────────────────────────────────────

    /// List secrets for an agent (returns metadata only, never values).
    pub async fn list_agent_secrets(&self, agent_id: &str) -> Result<Vec<SecretResponse>> {
        debug!(agent_id = %agent_id, "Listing agent secrets");
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/agents/{}/secrets", agent_id))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to list secrets: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "List secrets failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Vec<SecretResponse>>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// List user-global secrets.
    pub async fn list_user_secrets(&self) -> Result<Vec<SecretResponse>> {
        debug!("Listing user-global secrets");
        let resp = self
            .auth(self.client.get(self.url("/secrets")))
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to list secrets: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "List secrets failed ({}): {}",
                status, body
            )));
        }

        resp.json::<Vec<SecretResponse>>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// Set a user-global secret.
    pub async fn set_user_secret(&self, key: &str, value: &str) -> Result<SecretResponse> {
        debug!(key = %key, "Setting user-global secret");
        let resp = self
            .auth(self.client.post(self.url("/secrets")))
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

    /// Delete a user-global secret by ID.
    pub async fn delete_user_secret(&self, secret_id: &str) -> Result<()> {
        debug!(secret_id = %secret_id, "Deleting user-global secret");
        let resp = self
            .auth(
                self.client
                    .delete(self.url(&format!("/secrets/{}", secret_id))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to delete secret: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Delete secret failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    /// Delete an agent-scoped secret by ID.
    pub async fn delete_agent_secret(&self, agent_id: &str, secret_id: &str) -> Result<()> {
        debug!(agent_id = %agent_id, secret_id = %secret_id, "Deleting agent secret");
        let resp = self
            .auth(
                self.client.delete(self.url(&format!(
                    "/agents/{}/secrets/{}",
                    agent_id, secret_id
                ))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to delete secret: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Delete secret failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

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
        name: Option<&str>,
        description: Option<&str>,
        zone: Option<&str>,
        machine_type: Option<&str>,
    ) -> Result<InstanceResponse> {
        debug!(agent_id = %agent_id, "Creating instance");
        let resp = self
            .auth(self.client.post(self.url("/instances")))
            .json(&CreateInstanceRequest {
                agent_id: agent_id.to_string(),
                name: name.map(String::from),
                description: description.map(String::from),
                zone: zone.map(String::from),
                machine_type: machine_type.map(String::from),
                variable_overrides: None,
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
                .create_instance(agent_id, opts.instance_name, opts.instance_description, opts.zone, opts.machine_type)
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

    // ── Sync ──────────────────────────────────────────────────────────

    /// Compute the diff between a local manifest and remote state.
    pub async fn sync_manifest(
        &self,
        agent_id: &str,
        manifest: &SyncManifestRequest,
    ) -> Result<SyncManifestResponse> {
        debug!(agent_id = %agent_id, files = manifest.files.len(), "Computing sync manifest");
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/agents/{}/sync", agent_id))),
            )
            .json(manifest)
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to sync: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Sync manifest failed ({}): {}",
                status, body
            )));
        }

        resp.json::<SyncManifestResponse>()
            .await
            .map_err(|e| StarpodError::Channel(format!("Invalid response: {}", e)))
    }

    /// Download a single file from the agent.
    pub async fn download_file(
        &self,
        agent_id: &str,
        file_path: &str,
    ) -> Result<Vec<u8>> {
        debug!(agent_id = %agent_id, path = %file_path, "Downloading file");
        let resp = self
            .auth(
                self.client.get(self.url(&format!(
                    "/agents/{}/files/{}",
                    agent_id,
                    urlencoding::encode(file_path)
                ))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to download file: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Download file failed ({}): {}",
                status, body
            )));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| StarpodError::Channel(format!("Failed to read body: {}", e)))
    }

    /// Delete a single file from the agent on the backend.
    pub async fn delete_file(
        &self,
        agent_id: &str,
        file_path: &str,
    ) -> Result<()> {
        debug!(agent_id = %agent_id, path = %file_path, "Deleting remote file");
        let resp = self
            .auth(
                self.client.delete(self.url(&format!(
                    "/agents/{}/files/{}",
                    agent_id,
                    urlencoding::encode(file_path)
                ))),
            )
            .send()
            .await
            .map_err(|e| StarpodError::Channel(format!("Failed to delete file: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StarpodError::Channel(format!(
                "Delete file failed ({}): {}",
                status, body
            )));
        }

        Ok(())
    }

    // ── High-level push/pull ─────────────────────────────────────────

    /// Compute the diff between local and remote without modifying anything.
    /// Returns the raw SyncManifestResponse for display.
    pub async fn diff_agent(
        &self,
        agent_name: &str,
        agent_dir: &Path,
        skills_dir: Option<&Path>,
    ) -> Result<SyncManifestResponse> {
        let agent = self.find_or_create_agent(agent_name).await?;
        let agent_id = &agent.id;

        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        collect_files_recursive(agent_dir, "", &mut files)?;
        if let Some(sd) = skills_dir {
            if sd.exists() {
                collect_files_recursive(sd, "skills/", &mut files)?;
            }
        }

        let manifest = compute_manifest(&files);
        self.sync_manifest(agent_id, &manifest).await
    }

    /// Push local agent files to the remote, uploading only changed files.
    pub async fn push_agent(
        &self,
        agent_name: &str,
        agent_dir: &Path,
        skills_dir: Option<&Path>,
    ) -> Result<SyncSummary> {
        let agent = self.find_or_create_agent(agent_name).await?;
        let agent_id = &agent.id;

        // Collect local files and compute manifest
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        collect_files_recursive(agent_dir, "", &mut files)?;
        if let Some(sd) = skills_dir {
            if sd.exists() {
                collect_files_recursive(sd, "skills/", &mut files)?;
            }
        }

        let manifest = compute_manifest(&files);
        let total_local = manifest.files.len();

        // Get diff from server
        let diff = self.sync_manifest(agent_id, &manifest).await?;

        // Upload changed/new files
        let files_map: HashMap<&str, &[u8]> = files
            .iter()
            .map(|(p, d)| (p.as_str(), d.as_slice()))
            .collect();

        for path in &diff.to_upload {
            if let Some(data) = files_map.get(path.as_str()) {
                self.upload_files(agent_id, vec![(path.clone(), data.to_vec())])
                    .await?;
            }
        }

        // Delete stale remote files
        for path in &diff.to_delete_remote {
            self.delete_file(agent_id, path).await?;
        }

        let unchanged = total_local - diff.to_upload.len();
        Ok(SyncSummary {
            uploaded: diff.to_upload.len(),
            downloaded: 0,
            deleted_remote: diff.to_delete_remote.len(),
            deleted_local: 0,
            unchanged,
        })
    }

    /// Pull remote agent files to the local workspace, downloading only changed files.
    pub async fn pull_agent(
        &self,
        agent_name: &str,
        agent_dir: &Path,
    ) -> Result<SyncSummary> {
        let agents = self.list_agents().await?;
        let agent = agents
            .into_iter()
            .find(|a| a.name == agent_name)
            .ok_or_else(|| {
                StarpodError::Channel(format!("Agent '{}' not found on remote", agent_name))
            })?;
        let agent_id = &agent.id;

        // Collect local files and compute manifest
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        if agent_dir.exists() {
            collect_files_recursive(agent_dir, "", &mut files)?;
        }

        let manifest = compute_manifest(&files);

        // Get diff from server
        let diff = self.sync_manifest(agent_id, &manifest).await?;

        // Download changed/new files
        for file_info in &diff.to_download {
            let data = self.download_file(agent_id, &file_info.path).await?;
            let dest = agent_dir.join(&file_info.path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    StarpodError::Config(format!("Failed to create dir {:?}: {}", parent, e))
                })?;
            }
            std::fs::write(&dest, data).map_err(|e| {
                StarpodError::Config(format!("Failed to write {:?}: {}", dest, e))
            })?;
        }

        // Delete local files no longer on remote
        for path in &diff.to_delete_local {
            let dest = agent_dir.join(path);
            if dest.exists() {
                std::fs::remove_file(&dest).map_err(|e| {
                    StarpodError::Config(format!("Failed to delete {:?}: {}", dest, e))
                })?;
            }
        }

        let total_remote = diff.to_download.len()
            + (manifest.files.len() - diff.to_delete_local.len());
        let unchanged = total_remote.saturating_sub(diff.to_download.len());
        Ok(SyncSummary {
            uploaded: 0,
            downloaded: diff.to_download.len(),
            deleted_remote: 0,
            deleted_local: diff.to_delete_local.len(),
            unchanged,
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

/// Compute a sync manifest from collected files: path → base64-encoded MD5 hash.
fn compute_manifest(files: &[(String, Vec<u8>)]) -> SyncManifestRequest {
    use base64::Engine;
    let mut map = HashMap::new();
    for (path, data) in files {
        let digest = md5::compute(data);
        let hash = base64::engine::general_purpose::STANDARD.encode(digest.as_ref());
        map.insert(path.clone(), hash);
    }
    SyncManifestRequest { files: map }
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

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- compute_manifest tests ---

    #[test]
    fn manifest_produces_base64_md5() {
        use base64::Engine;
        let files = vec![("test.txt".to_string(), b"hello world".to_vec())];
        let manifest = compute_manifest(&files);

        assert_eq!(manifest.files.len(), 1);
        let hash = manifest.files.get("test.txt").unwrap();

        // Verify it matches: base64(md5("hello world"))
        let expected_digest = md5::compute(b"hello world");
        let expected = base64::engine::general_purpose::STANDARD.encode(expected_digest.as_ref());
        assert_eq!(hash, &expected);
    }

    #[test]
    fn manifest_empty_files() {
        let manifest = compute_manifest(&[]);
        assert!(manifest.files.is_empty());
    }

    #[test]
    fn manifest_different_content_different_hashes() {
        let files = vec![
            ("a.txt".to_string(), b"hello".to_vec()),
            ("b.txt".to_string(), b"world".to_vec()),
        ];
        let manifest = compute_manifest(&files);
        let hash_a = manifest.files.get("a.txt").unwrap();
        let hash_b = manifest.files.get("b.txt").unwrap();
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn manifest_same_content_same_hash() {
        let files = vec![
            ("a.txt".to_string(), b"same".to_vec()),
            ("b.txt".to_string(), b"same".to_vec()),
        ];
        let manifest = compute_manifest(&files);
        let hash_a = manifest.files.get("a.txt").unwrap();
        let hash_b = manifest.files.get("b.txt").unwrap();
        assert_eq!(hash_a, hash_b);
    }

    // --- DeployClient sync integration tests (wiremock) ---

    async fn setup_client() -> (MockServer, DeployClient) {
        let server = MockServer::start().await;
        let client = DeployClient::new(&server.uri(), "test-key").unwrap();
        (server, client)
    }

    #[tokio::test]
    async fn sync_manifest_sends_correct_request() {
        let (server, client) = setup_client().await;

        let response_body = serde_json::json!({
            "to_upload": ["SOUL.md"],
            "to_download": [],
            "to_delete_remote": [],
            "to_delete_local": []
        });

        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-123/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let mut files = HashMap::new();
        files.insert("SOUL.md".to_string(), "somehash".to_string());
        let manifest = SyncManifestRequest { files };

        let result = client.sync_manifest("agent-123", &manifest).await.unwrap();
        assert_eq!(result.to_upload, vec!["SOUL.md"]);
        assert!(result.to_download.is_empty());
    }

    #[tokio::test]
    async fn download_file_returns_content() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/agents/agent-123/files/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"file content here"))
            .expect(1)
            .mount(&server)
            .await;

        let data = client.download_file("agent-123", "SOUL.md").await.unwrap();
        assert_eq!(data, b"file content here");
    }

    #[tokio::test]
    async fn delete_file_succeeds() {
        let (server, client) = setup_client().await;

        Mock::given(method("DELETE"))
            .and(path_regex(r"/api/v1/agents/agent-123/files/.*"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        client.delete_file("agent-123", "old.md").await.unwrap();
    }

    #[tokio::test]
    async fn push_agent_uploads_changed_and_deletes_stale() {
        let (server, client) = setup_client().await;

        // Mock: list agents → return existing agent
        Mock::given(method("GET"))
            .and(path("/api/v1/agents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "agent-id-1", "name": "test-agent", "gcs_path": "agents/1/", "created_at": "2026-01-01T00:00:00Z"}
            ])))
            .mount(&server)
            .await;

        // Mock: sync → one file to upload, one to delete
        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-id-1/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "to_upload": ["SOUL.md"],
                "to_download": [],
                "to_delete_remote": ["old-file.md"],
                "to_delete_local": []
            })))
            .mount(&server)
            .await;

        // Mock: upload file
        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-id-1/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "uploaded": [{"path": "SOUL.md", "size": 11}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        // Mock: delete file
        Mock::given(method("DELETE"))
            .and(path_regex(r"/api/v1/agents/agent-id-1/files/.*"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        // Create temp agent dir with one file
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("test-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("SOUL.md"), "hello world").unwrap();

        let summary = client.push_agent("test-agent", &agent_dir, None).await.unwrap();
        assert_eq!(summary.uploaded, 1);
        assert_eq!(summary.deleted_remote, 1);
        assert_eq!(summary.downloaded, 0);
    }

    #[tokio::test]
    async fn pull_agent_downloads_and_deletes_local() {
        let (server, client) = setup_client().await;

        // Mock: list agents
        Mock::given(method("GET"))
            .and(path("/api/v1/agents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "agent-id-1", "name": "test-agent", "gcs_path": "agents/1/", "created_at": "2026-01-01T00:00:00Z"}
            ])))
            .mount(&server)
            .await;

        // Mock: sync → one file to download, one to delete locally
        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-id-1/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "to_upload": [],
                "to_download": [{"path": "BOOT.md", "size": 100, "md5_hash": "abc123"}],
                "to_delete_remote": [],
                "to_delete_local": ["stale.md"]
            })))
            .mount(&server)
            .await;

        // Mock: download BOOT.md
        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/agents/agent-id-1/files/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"boot content"))
            .expect(1)
            .mount(&server)
            .await;

        // Create temp agent dir with stale file
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("test-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("stale.md"), "old content").unwrap();

        let summary = client.pull_agent("test-agent", &agent_dir).await.unwrap();
        assert_eq!(summary.downloaded, 1);
        assert_eq!(summary.deleted_local, 1);
        assert_eq!(summary.uploaded, 0);

        // Verify: BOOT.md was written
        let boot_content = std::fs::read_to_string(agent_dir.join("BOOT.md")).unwrap();
        assert_eq!(boot_content, "boot content");

        // Verify: stale.md was deleted
        assert!(!agent_dir.join("stale.md").exists());
    }

    // --- parse_env_file tests ---

    #[test]
    fn parse_env_basic_key_value() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "KEY1=value1\nKEY2=value2\n").unwrap();

        let env = parse_env_file(&env_path).unwrap();
        assert_eq!(env.get("KEY1").unwrap(), "value1");
        assert_eq!(env.get("KEY2").unwrap(), "value2");
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn parse_env_strips_quotes() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "KEY=\"quoted value\"\n").unwrap();

        let env = parse_env_file(&env_path).unwrap();
        assert_eq!(env.get("KEY").unwrap(), "quoted value");
    }

    #[test]
    fn parse_env_skips_comments_and_empty_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "# comment\n\nKEY=val\n  \n# another\n").unwrap();

        let env = parse_env_file(&env_path).unwrap();
        assert_eq!(env.len(), 1);
        assert_eq!(env.get("KEY").unwrap(), "val");
    }

    #[test]
    fn parse_env_handles_whitespace() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "  KEY = value  \n").unwrap();

        let env = parse_env_file(&env_path).unwrap();
        assert_eq!(env.get("KEY").unwrap(), "value");
    }

    #[test]
    fn parse_env_missing_file_errors() {
        let result = parse_env_file(Path::new("/nonexistent/.env"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_env_value_with_equals_sign() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "DATABASE_URL=postgres://user:pass@host/db?opt=1\n").unwrap();

        let env = parse_env_file(&env_path).unwrap();
        assert_eq!(
            env.get("DATABASE_URL").unwrap(),
            "postgres://user:pass@host/db?opt=1"
        );
    }

    // --- collect_files_recursive tests ---

    #[test]
    fn collect_files_skips_hidden_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        std::fs::write(dir.join("visible.txt"), "ok").unwrap();
        std::fs::create_dir(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join(".hidden").join("secret.txt"), "nope").unwrap();
        std::fs::write(dir.join(".gitignore"), "nope").unwrap();

        let mut files = Vec::new();
        collect_files_recursive(dir, "", &mut files).unwrap();

        let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(names.contains(&"visible.txt"));
        assert!(!names.iter().any(|n| n.contains("hidden") || n.contains(".git")));
    }

    #[test]
    fn collect_files_with_nested_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        std::fs::write(dir.join("root.md"), "root").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("nested.md"), "nested").unwrap();

        let mut files = Vec::new();
        collect_files_recursive(dir, "", &mut files).unwrap();

        let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(names.contains(&"root.md"));
        assert!(names.contains(&"sub/nested.md"));
    }

    #[test]
    fn collect_files_with_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("skill.md"), "data").unwrap();

        let mut files = Vec::new();
        collect_files_recursive(dir, "skills/", &mut files).unwrap();

        assert_eq!(files[0].0, "skills/skill.md");
    }

    #[test]
    fn collect_files_nonexistent_dir_is_ok() {
        let mut files = Vec::new();
        let result = collect_files_recursive(Path::new("/nonexistent"), "", &mut files);
        assert!(result.is_ok());
        assert!(files.is_empty());
    }

    #[test]
    fn collect_files_skips_node_modules_and_target() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        std::fs::write(dir.join("keep.md"), "ok").unwrap();
        std::fs::create_dir(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("node_modules").join("pkg.js"), "skip").unwrap();
        std::fs::create_dir(dir.join("target")).unwrap();
        std::fs::write(dir.join("target").join("bin"), "skip").unwrap();

        let mut files = Vec::new();
        collect_files_recursive(dir, "", &mut files).unwrap();

        let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec!["keep.md"]);
    }

    // --- get_deploy_config tests ---

    #[tokio::test]
    async fn get_deploy_config_returns_readiness() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/agents/agent-123/deploy-config"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "version": 1,
                "variables": [
                    {"key": "MODEL", "default": "claude-sonnet", "description": "Model to use", "scope": "agent"}
                ],
                "secrets": [
                    {"key": "ANTHROPIC_API_KEY", "required": true, "description": "API key", "present": true, "scope": "agent", "hint": "sk-a"}
                ],
                "ready": true,
                "missing_required": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = client.get_deploy_config("agent-123").await.unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert!(config.ready);
        assert_eq!(config.version, 1);
        assert_eq!(config.variables.len(), 1);
        assert_eq!(config.variables[0].key, "MODEL");
        assert_eq!(config.variables[0].default.as_deref(), Some("claude-sonnet"));
        assert_eq!(config.secrets.len(), 1);
        assert!(config.secrets[0].present);
        assert!(config.missing_required.is_empty());
    }

    #[tokio::test]
    async fn get_deploy_config_returns_none_on_404() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/agents/agent-123/deploy-config"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let config = client.get_deploy_config("agent-123").await.unwrap();
        assert!(config.is_none());
    }

    #[tokio::test]
    async fn get_deploy_config_errors_on_500() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/agents/agent-123/deploy-config"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .expect(1)
            .mount(&server)
            .await;

        let result = client.get_deploy_config("agent-123").await;
        assert!(result.is_err());
    }

    // --- Secrets CRUD tests ---

    #[tokio::test]
    async fn list_agent_secrets_returns_list() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/agents/agent-123/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "s1", "key": "API_KEY", "hint": "sk-a", "agent_id": "agent-123", "created_at": "2026-01-01T00:00:00Z"},
                {"id": "s2", "key": "DB_URL", "hint": "post", "agent_id": "agent-123", "created_at": "2026-01-01T00:00:00Z"}
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let secrets = client.list_agent_secrets("agent-123").await.unwrap();
        assert_eq!(secrets.len(), 2);
        assert_eq!(secrets[0].key, "API_KEY");
        assert_eq!(secrets[1].key, "DB_URL");
    }

    #[tokio::test]
    async fn list_user_secrets_returns_list() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "s1", "key": "GLOBAL_KEY", "hint": "glo", "agent_id": null, "created_at": "2026-01-01T00:00:00Z"}
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let secrets = client.list_user_secrets().await.unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].key, "GLOBAL_KEY");
        assert!(secrets[0].agent_id.is_none());
    }

    #[tokio::test]
    async fn set_user_secret_succeeds() {
        let (server, client) = setup_client().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "s-new", "key": "MY_SECRET", "hint": "val", "agent_id": null, "created_at": "2026-01-01T00:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let secret = client.set_user_secret("MY_SECRET", "my-value").await.unwrap();
        assert_eq!(secret.key, "MY_SECRET");
        assert_eq!(secret.id, "s-new");
    }

    #[tokio::test]
    async fn delete_user_secret_succeeds() {
        let (server, client) = setup_client().await;

        Mock::given(method("DELETE"))
            .and(path("/api/v1/secrets/s-123"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        client.delete_user_secret("s-123").await.unwrap();
    }

    #[tokio::test]
    async fn delete_agent_secret_succeeds() {
        let (server, client) = setup_client().await;

        Mock::given(method("DELETE"))
            .and(path("/api/v1/agents/agent-123/secrets/s-456"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        client.delete_agent_secret("agent-123", "s-456").await.unwrap();
    }

    #[tokio::test]
    async fn list_secrets_error_propagates() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/secrets"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .expect(1)
            .mount(&server)
            .await;

        let result = client.list_user_secrets().await;
        assert!(result.is_err());
    }

    // --- Error case tests for sync operations ---

    #[tokio::test]
    async fn sync_manifest_error_on_server_failure() {
        let (server, client) = setup_client().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-123/sync"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
            .expect(1)
            .mount(&server)
            .await;

        let manifest = SyncManifestRequest { files: HashMap::new() };
        let result = client.sync_manifest("agent-123", &manifest).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn download_file_error_on_404() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/agents/agent-123/files/.*"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .expect(1)
            .mount(&server)
            .await;

        let result = client.download_file("agent-123", "missing.md").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_file_error_on_500() {
        let (server, client) = setup_client().await;

        Mock::given(method("DELETE"))
            .and(path_regex(r"/api/v1/agents/agent-123/files/.*"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal"))
            .expect(1)
            .mount(&server)
            .await;

        let result = client.delete_file("agent-123", "bad.md").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn pull_agent_errors_when_agent_not_found() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/agents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let result = client.pull_agent("nonexistent", tmp.path()).await;
        assert!(result.is_err());
    }

    // --- push_agent with skills dir ---

    #[tokio::test]
    async fn push_agent_includes_skills_dir() {
        let (server, client) = setup_client().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/agents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "agent-id-1", "name": "test-agent", "gcs_path": "agents/1/", "created_at": "2026-01-01T00:00:00Z"}
            ])))
            .mount(&server)
            .await;

        // Sync returns both agent file and skill file need upload
        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-id-1/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "to_upload": ["SOUL.md", "skills/greet.md"],
                "to_download": [],
                "to_delete_remote": [],
                "to_delete_local": []
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-id-1/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "uploaded": [{"path": "SOUL.md", "size": 5}, {"path": "skills/greet.md", "size": 6}]
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("SOUL.md"), "soul").unwrap();

        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("greet.md"), "greet!").unwrap();

        let summary = client
            .push_agent("test-agent", &agent_dir, Some(&skills_dir))
            .await
            .unwrap();
        assert_eq!(summary.uploaded, 2);
        assert_eq!(summary.unchanged, 0);
    }

    #[tokio::test]
    async fn diff_agent_returns_manifest_without_mutations() {
        let (server, client) = setup_client().await;
        let tmp = tempfile::tempdir().unwrap();

        // Create a local agent file
        std::fs::write(tmp.path().join("agent.toml"), b"[agent]").unwrap();
        std::fs::write(tmp.path().join("SOUL.md"), b"# Soul").unwrap();

        // Mock: agent lookup
        let agent = AgentResponse {
            id: "agent-123".to_string(),
            name: "test-agent".to_string(),
            gcs_path: "agents/test-agent/".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        Mock::given(method("GET"))
            .and(path("/api/v1/agents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![&agent]))
            .mount(&server)
            .await;

        // Mock: sync manifest returns diff
        let sync_resp = SyncManifestResponse {
            to_upload: vec!["SOUL.md".to_string()],
            to_download: vec![UploadedFile {
                path: "BOOT.md".to_string(),
                size: 100,
                md5_hash: "abc123".to_string(),
            }],
            to_delete_remote: vec!["old-file.md".to_string()],
            to_delete_local: vec![],
        };
        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-123/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sync_resp))
            .mount(&server)
            .await;

        let diff = client.diff_agent("test-agent", tmp.path(), None).await.unwrap();

        // Verify we got the diff back correctly
        assert_eq!(diff.to_upload, vec!["SOUL.md"]);
        assert_eq!(diff.to_download.len(), 1);
        assert_eq!(diff.to_download[0].path, "BOOT.md");
        assert_eq!(diff.to_delete_remote, vec!["old-file.md"]);
        assert!(diff.to_delete_local.is_empty());

        // Verify NO upload or delete requests were made
        // (only GET /agents and POST /agents/agent-123/sync should have been called)
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 2, "diff_agent should only make 2 requests (list agents + sync)");
    }

    #[tokio::test]
    async fn diff_agent_with_skills_dir() {
        let (server, client) = setup_client().await;
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");

        // Create local files
        std::fs::write(tmp.path().join("agent.toml"), b"[agent]").unwrap();
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("my-skill.md"), b"# Skill").unwrap();

        let agent = AgentResponse {
            id: "agent-456".to_string(),
            name: "skill-agent".to_string(),
            gcs_path: "agents/skill-agent/".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        Mock::given(method("GET"))
            .and(path("/api/v1/agents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![&agent]))
            .mount(&server)
            .await;

        let sync_resp = SyncManifestResponse {
            to_upload: vec!["skills/my-skill.md".to_string()],
            to_download: vec![],
            to_delete_remote: vec![],
            to_delete_local: vec![],
        };
        Mock::given(method("POST"))
            .and(path("/api/v1/agents/agent-456/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sync_resp))
            .mount(&server)
            .await;

        let diff = client.diff_agent("skill-agent", tmp.path(), Some(&skills_dir)).await.unwrap();
        assert_eq!(diff.to_upload, vec!["skills/my-skill.md"]);
    }

    // --- DeployReadiness helper tests ---

    fn make_secret(key: &str, required: bool, present: bool) -> SecretStatusInfo {
        SecretStatusInfo {
            key: key.to_string(),
            required,
            description: String::new(),
            present,
            scope: None,
            hint: None,
            resolved_from: None,
        }
    }

    fn make_readiness(secrets: Vec<SecretStatusInfo>, missing_required: Vec<&str>) -> DeployReadiness {
        DeployReadiness {
            version: 1,
            variables: vec![],
            secrets,
            ready: missing_required.is_empty(),
            missing_required: missing_required.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn missing_optional_in_env_finds_matching_keys() {
        let readiness = make_readiness(
            vec![
                make_secret("API_KEY", true, true),
                make_secret("OPTIONAL_A", false, false),
                make_secret("OPTIONAL_B", false, false),
                make_secret("OPTIONAL_C", false, true), // already present
            ],
            vec![],
        );
        let local_env: HashMap<String, String> = [
            ("OPTIONAL_A".to_string(), "val_a".to_string()),
            ("OPTIONAL_C".to_string(), "val_c".to_string()),
            ("UNRELATED".to_string(), "val".to_string()),
        ]
        .into_iter()
        .collect();

        let result = readiness.missing_optional_in_env(&local_env);
        assert_eq!(result, vec!["OPTIONAL_A"]);
    }

    #[test]
    fn missing_optional_in_env_empty_when_no_env() {
        let readiness = make_readiness(
            vec![make_secret("OPT", false, false)],
            vec![],
        );
        let local_env = HashMap::new();
        assert!(readiness.missing_optional_in_env(&local_env).is_empty());
    }

    #[test]
    fn missing_optional_in_env_ignores_required() {
        let readiness = make_readiness(
            vec![make_secret("REQ", true, false)],
            vec!["REQ"],
        );
        let local_env: HashMap<String, String> =
            [("REQ".to_string(), "val".to_string())].into_iter().collect();
        assert!(readiness.missing_optional_in_env(&local_env).is_empty());
    }

    #[test]
    fn missing_required_in_env_finds_matching_keys() {
        let readiness = make_readiness(
            vec![
                make_secret("REQ_A", true, false),
                make_secret("REQ_B", true, false),
            ],
            vec!["REQ_A", "REQ_B"],
        );
        let local_env: HashMap<String, String> =
            [("REQ_A".to_string(), "val".to_string())].into_iter().collect();

        let result = readiness.missing_required_in_env(&local_env);
        assert_eq!(result, vec!["REQ_A"]);
    }

    #[test]
    fn missing_required_in_env_empty_when_no_match() {
        let readiness = make_readiness(
            vec![make_secret("REQ", true, false)],
            vec!["REQ"],
        );
        let local_env = HashMap::new();
        assert!(readiness.missing_required_in_env(&local_env).is_empty());
    }
}
