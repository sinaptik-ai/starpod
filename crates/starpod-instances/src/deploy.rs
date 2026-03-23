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

#[derive(Debug, Clone, Deserialize)]
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
}

/// Deploy readiness response from `GET /agents/{id}/deploy-config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployReadiness {
    pub version: u32,
    pub variables: HashMap<String, String>,
    pub secrets: Vec<SecretStatusInfo>,
    pub ready: bool,
    pub missing_required: Vec<String>,
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
}
