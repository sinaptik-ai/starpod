//! System API — version checking, self-update, and restart.
//!
//! Endpoints live under `/api/system/*` and are protected by admin middleware.
//!
//! # Version check — `GET /api/system/version`
//!
//! Queries the GitHub Releases API for `sinaptik-ai/starpod` and compares
//! the latest tag against the compile-time version (`CARGO_PKG_VERSION`).
//! The result is cached in-memory for 1 hour to avoid rate-limiting.
//! If the fetch fails, stale cache is returned when available.
//!
//! The response includes the current platform target triple (detected at
//! compile time) so the frontend knows which binary will be downloaded.
//!
//! # Self-update — `POST /api/system/update`
//!
//! Triggers a background update pipeline:
//!
//! 1. **Download** — Fetches the platform-appropriate `.tar.gz` from GitHub Releases.
//! 2. **Verify** — Checks SHA-256 against the checksums manifest (if available).
//! 3. **Backup** — Copies the current binary, all `.db` files, and `agent.toml`
//!    into `.starpod/backups/` tagged with the current version.
//! 4. **Replace** — Renames the old binary to `.bak`, writes the new one, `chmod +x`.
//! 5. **Restart** — Spawns the new binary with the same CLI args as a detached child.
//! 6. **Monitor** — Watches the child for 30 seconds. If it exits with an error,
//!    the old binary is restored from `.bak`. If it stays alive, the old process
//!    performs a graceful shutdown via [`tokio::sync::watch`].
//!
//! # Rollback safety
//!
//! - Binary: `.bak` rename is atomic on the same filesystem; restored on failure.
//! - Databases: Copied to `backups/db-{version}/` before the update. SQLx migrations
//!   run transactionally on the new binary's startup.
//! - Config: Copied to `backups/agent-{version}.toml`. The config migration system
//!   ([`starpod_core::config_migrate`]) handles schema changes at startup.
//! - If the new binary crashes within 30 seconds, the old binary detects this via
//!   `try_wait()` and restores itself automatically.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::routes::ErrorResponse;
use crate::AppState;

// ── Constants ──────────────────────────────────────────────────────────

/// GitHub API endpoint for the latest release of sinaptik-ai/starpod.
const GITHUB_RELEASES_URL: &str =
    "https://api.github.com/repos/sinaptik-ai/starpod/releases/latest";

/// How long to cache release info before re-fetching from GitHub (1 hour).
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// The version baked into this binary at compile time from `Cargo.toml`.
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long to wait for the new binary to stay alive before assuming it's healthy.
/// If the child process exits with a non-zero code within this window, the old
/// binary restores itself from the `.bak` backup.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);

/// Returns the platform target triple for the current binary.
///
/// Resolved at compile time via `cfg!()`. Maps to the asset filenames
/// in GitHub Releases (e.g., `starpod-aarch64-apple-darwin.tar.gz`).
/// Returns `"unsupported"` on platforms we don't ship binaries for.
fn platform_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        "unsupported"
    }
}

// ── Types ──────────────────────────────────────────────────────────────

/// Cached release information fetched from the GitHub Releases API.
///
/// Stored in [`AppState::update_cache`] behind an `Arc<RwLock<...>>`.
/// The cache is populated on the first `GET /api/system/version` request
/// and refreshed when `fetched_at` exceeds [`CACHE_TTL`] (1 hour).
#[derive(Debug, Clone)]
pub struct CachedRelease {
    /// Semver version string (e.g., `"0.3.0"`), with any `v` prefix stripped.
    pub version: String,
    /// ISO 8601 timestamp from the GitHub release.
    pub published_at: String,
    /// URL to the release page on GitHub (for "What's new" links).
    pub release_notes_url: String,
    /// Platform target triple → download info. Only populated for the 4
    /// supported targets (linux x86_64/aarch64, macOS x86_64/aarch64).
    pub assets: std::collections::HashMap<String, AssetInfo>,
    /// When this entry was fetched, for TTL expiration.
    pub fetched_at: tokio::time::Instant,
}

/// Download URL and optional checksum for a single platform asset.
///
/// The `sha256` field is populated from a checksums manifest file
/// (e.g., `SHA256SUMS`) attached to the same GitHub release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    /// Direct download URL for the `.tar.gz` archive.
    pub url: String,
    /// Hex-encoded SHA-256 hash, if available from the release checksums.
    pub sha256: Option<String>,
}

/// Response body for `GET /api/system/version`.
///
/// Always includes the `current` version and `platform`. If the GitHub API
/// is unreachable, `latest` and related fields will be `null`.
#[derive(Debug, Serialize)]
struct VersionResponse {
    /// Compile-time version of the running binary (e.g., `"0.2.1"`).
    current: String,
    /// Latest available version from GitHub, or `null` if the check failed.
    latest: Option<String>,
    /// `true` when `latest` is strictly newer than `current` per semver.
    update_available: bool,
    /// URL to the GitHub release page (for "What's new" link in the UI).
    release_notes_url: Option<String>,
    /// ISO 8601 publication timestamp of the latest release.
    published_at: Option<String>,
    /// Target triple of this binary (e.g., `"aarch64-apple-darwin"`).
    platform: String,
}

/// Response body for `POST /api/system/update`.
///
/// Returned immediately after the background update task is spawned.
/// The actual update continues asynchronously — the frontend should
/// poll `GET /api/health` for the new version.
#[derive(Debug, Serialize)]
struct UpdateResponse {
    /// Always `"updating"` on success.
    status: String,
    /// The version being updated to.
    version: String,
    /// Human-readable status message.
    message: String,
}

/// Subset of the GitHub release API response that we deserialize.
///
/// See <https://docs.github.com/en/rest/releases/releases#get-the-latest-release>.
/// Extra fields in the response are silently ignored by serde.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    /// Git tag name, typically `"v0.3.0"`.
    tag_name: String,
    /// ISO 8601 timestamp; `None` for draft releases.
    published_at: Option<String>,
    /// URL to the release page on GitHub.
    html_url: String,
    /// Attached binary assets (tarballs, checksums, etc.).
    assets: Vec<GitHubAsset>,
    /// Markdown body of the release notes.
    #[allow(dead_code)]
    body: Option<String>,
}

/// A single asset attached to a GitHub release.
#[derive(Debug, Deserialize)]
struct GitHubAsset {
    /// Filename as uploaded (e.g., `"starpod-aarch64-apple-darwin.tar.gz"`).
    name: String,
    /// Direct download URL (redirects to the CDN).
    browser_download_url: String,
}

/// Thread-safe update cache stored in [`AppState`].
///
/// `None` means no release info has been fetched yet. `Some(release)` holds
/// the last-fetched release, which may be stale (check `fetched_at`).
pub type UpdateCache = Arc<RwLock<Option<CachedRelease>>>;

/// Create an empty update cache. Called once during [`AppState`] construction.
pub fn new_update_cache() -> UpdateCache {
    Arc::new(RwLock::new(None))
}

// ── Routes ─────────────────────────────────────────────────────────────

/// Build the system sub-router with `/api/system/*` routes.
///
/// Protected by admin middleware (same as settings routes).
pub fn system_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/system/version", get(get_version))
        .route("/api/system/update", post(trigger_update))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::settings::require_admin_middleware,
        ))
}

// ── Version check ──────────────────────────────────────────────────────

/// `GET /api/system/version` — Return current and latest version info.
///
/// Always succeeds (200). If the GitHub check fails, `latest` will be `null`
/// and `update_available` will be `false`.
async fn get_version(
    State(state): State<Arc<AppState>>,
) -> Result<Json<VersionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let release = fetch_or_cached_release(&state).await;

    match release {
        Some(rel) => {
            let update_available = is_newer(&rel.version, CURRENT_VERSION);
            Ok(Json(VersionResponse {
                current: CURRENT_VERSION.to_string(),
                latest: Some(rel.version),
                update_available,
                release_notes_url: Some(rel.release_notes_url),
                published_at: Some(rel.published_at),
                platform: platform_target().to_string(),
            }))
        }
        None => Ok(Json(VersionResponse {
            current: CURRENT_VERSION.to_string(),
            latest: None,
            update_available: false,
            release_notes_url: None,
            published_at: None,
            platform: platform_target().to_string(),
        })),
    }
}

/// Fetch release info from cache or GitHub API.
async fn fetch_or_cached_release(state: &AppState) -> Option<CachedRelease> {
    // Check cache first
    {
        let cache = state.update_cache.read().await;
        if let Some(ref cached) = *cache {
            if cached.fetched_at.elapsed() < CACHE_TTL {
                return Some(cached.clone());
            }
        }
    }

    // Fetch from GitHub
    match fetch_latest_release().await {
        Ok(release) => {
            let mut cache = state.update_cache.write().await;
            *cache = Some(release.clone());
            Some(release)
        }
        Err(e) => {
            warn!(error = %e, "failed to fetch latest release from GitHub");
            // Return stale cache if available
            let cache = state.update_cache.read().await;
            cache.clone()
        }
    }
}

/// Fetch the latest release from the GitHub API.
async fn fetch_latest_release() -> Result<CachedRelease, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(GITHUB_RELEASES_URL)
        .header("User-Agent", format!("starpod/{}", CURRENT_VERSION))
        .header("Accept", "application/vnd.github.v3+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }

    let gh_release: GitHubRelease = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    // Parse version from tag (strip leading 'v' if present)
    let version = gh_release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&gh_release.tag_name)
        .to_string();

    // Build asset map from GitHub release assets
    let mut assets = std::collections::HashMap::new();
    for asset in &gh_release.assets {
        // Match asset names like "starpod-x86_64-unknown-linux-gnu.tar.gz"
        let name = &asset.name;
        for target in &[
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ] {
            if name.contains(target) && name.ends_with(".tar.gz") {
                assets.insert(
                    target.to_string(),
                    AssetInfo {
                        url: asset.browser_download_url.clone(),
                        sha256: None, // Will try to fetch from checksums file
                    },
                );
            }
        }
    }

    // Try to find a checksums file in assets (e.g., "checksums.txt" or "SHA256SUMS")
    for asset in &gh_release.assets {
        let lower = asset.name.to_lowercase();
        if lower.contains("checksum") || lower.contains("sha256") {
            if let Ok(checksums) = fetch_checksums(&asset.browser_download_url).await {
                for (filename, hash) in checksums {
                    for (target, info) in assets.iter_mut() {
                        if filename.contains(target) {
                            info.sha256 = Some(hash.clone());
                        }
                    }
                }
            }
            break;
        }
    }

    Ok(CachedRelease {
        version,
        published_at: gh_release.published_at.unwrap_or_default(),
        release_notes_url: gh_release.html_url,
        assets,
        fetched_at: tokio::time::Instant::now(),
    })
}

/// Fetch and parse a checksums file (format: "hash  filename" per line).
async fn fetch_checksums(url: &str) -> Result<Vec<(String, String)>, String> {
    let client = reqwest::Client::new();
    let text = client
        .get(url)
        .header("User-Agent", format!("starpod/{}", CURRENT_VERSION))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("{}", e))?
        .text()
        .await
        .map_err(|e| format!("{}", e))?;

    Ok(text
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                Some((parts[1].to_string(), parts[0].to_string()))
            } else {
                None
            }
        })
        .collect())
}

/// Compare two semver strings; returns true if `latest` is newer than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

// ── Self-update ────────────────────────────────────────────────────────

/// `POST /api/system/update` — Start the self-update pipeline.
///
/// Returns immediately with `{ status: "updating" }`. The actual download,
/// verification, backup, and restart happen in a background tokio task.
///
/// # Errors
///
/// - `502 Bad Gateway` — Cannot reach GitHub to fetch release info.
/// - `409 Conflict` — Already running the latest version.
/// - `404 Not Found` — No release asset for this platform.
async fn trigger_update(
    State(state): State<Arc<AppState>>,
) -> Result<Json<UpdateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 1. Get latest release info
    let release = fetch_or_cached_release(&state)
        .await
        .ok_or_else(|| sys_err(StatusCode::BAD_GATEWAY, "Could not fetch release info"))?;

    if !is_newer(&release.version, CURRENT_VERSION) {
        return Err(sys_err(
            StatusCode::CONFLICT,
            "Already on the latest version",
        ));
    }

    // 2. Find asset for current platform
    let target = platform_target();
    let asset = release.assets.get(target).ok_or_else(|| {
        sys_err(
            StatusCode::NOT_FOUND,
            format!("No release asset found for platform: {}", target),
        )
    })?;

    let asset_url = asset.url.clone();
    let expected_sha = asset.sha256.clone();
    let new_version = release.version.clone();
    let agent_home = state.paths.agent_home.clone();
    let shutdown_tx = state.shutdown_tx.clone();

    let version_for_response = new_version.clone();

    // 3. Spawn background update task
    tokio::spawn(async move {
        if let Err(e) = run_update(
            agent_home,
            &asset_url,
            expected_sha.as_deref(),
            &new_version,
            shutdown_tx,
        )
        .await
        {
            error!(error = %e, "self-update failed");
        }
    });

    Ok(Json(UpdateResponse {
        status: "updating".to_string(),
        version: version_for_response,
        message: "Update started. Starpod will restart automatically.".to_string(),
    }))
}

/// Execute the full update pipeline: download → verify → backup → replace → restart.
async fn run_update(
    agent_home: PathBuf,
    asset_url: &str,
    expected_sha: Option<&str>,
    new_version: &str,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
) -> Result<(), String> {
    let tmp_dir = agent_home.join("tmp");
    let backup_dir = agent_home.join("backups");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| format!("Failed to create tmp dir: {}", e))?;
    tokio::fs::create_dir_all(&backup_dir)
        .await
        .map_err(|e| format!("Failed to create backup dir: {}", e))?;

    // ── Download ───────────────────────────────────────────────────────
    info!(url = %asset_url, "downloading update");
    let tarball_path = tmp_dir.join("starpod-update.tar.gz");
    let client = reqwest::Client::new();
    let resp = client
        .get(asset_url)
        .header("User-Agent", format!("starpod/{}", CURRENT_VERSION))
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Download returned HTTP {}", resp.status()));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read download: {}", e))?;

    tokio::fs::write(&tarball_path, &bytes)
        .await
        .map_err(|e| format!("Failed to write tarball: {}", e))?;

    // ── Verify SHA-256 ─────────────────────────────────────────────────
    if let Some(expected) = expected_sha {
        info!("verifying SHA-256 checksum");
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            // Clean up
            let _ = tokio::fs::remove_file(&tarball_path).await;
            return Err(format!(
                "Checksum mismatch: expected {}, got {}",
                expected, actual
            ));
        }
        info!("checksum verified");
    } else {
        warn!("no checksum available, skipping verification");
    }

    // ── Extract tarball ────────────────────────────────────────────────
    info!("extracting update");
    let extract_dir = tmp_dir.join("extracted");
    let _ = tokio::fs::remove_dir_all(&extract_dir).await;
    tokio::fs::create_dir_all(&extract_dir)
        .await
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    let tarball_data = bytes.to_vec();
    let extract_dir_clone = extract_dir.clone();
    let new_binary_path = tokio::task::spawn_blocking(move || -> Result<PathBuf, String> {
        let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(tarball_data));
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(&extract_dir_clone)
            .map_err(|e| format!("Failed to extract tarball: {}", e))?;

        // Find the starpod binary in the extracted files
        find_binary_in_dir(&extract_dir_clone)
    })
    .await
    .map_err(|e| format!("Extract task panicked: {}", e))?
    .map_err(|e| format!("Extract failed: {}", e))?;

    // ── Backup ─────────────────────────────────────────────────────────
    info!("creating backup");
    let current_binary =
        std::env::current_exe().map_err(|e| format!("Cannot determine current binary: {}", e))?;

    // Backup binary
    let binary_backup = backup_dir.join(format!("starpod-{}", CURRENT_VERSION));
    tokio::fs::copy(&current_binary, &binary_backup)
        .await
        .map_err(|e| format!("Failed to backup binary: {}", e))?;

    // Backup databases
    let db_dir = agent_home.join("db");
    if db_dir.exists() {
        let db_backup = backup_dir.join(format!("db-{}", CURRENT_VERSION));
        tokio::fs::create_dir_all(&db_backup)
            .await
            .map_err(|e| format!("Failed to create db backup dir: {}", e))?;
        let mut entries = tokio::fs::read_dir(&db_dir)
            .await
            .map_err(|e| format!("Failed to read db dir: {}", e))?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("db") {
                if let Some(name) = path.file_name() {
                    tokio::fs::copy(&path, db_backup.join(name))
                        .await
                        .map_err(|e| format!("Failed to backup {}: {}", path.display(), e))?;
                }
            }
        }
    }

    // Backup config
    let config_dir = agent_home.join("config");
    let agent_toml = config_dir.join("agent.toml");
    if agent_toml.exists() {
        let config_backup = backup_dir.join(format!("agent-{}.toml", CURRENT_VERSION));
        tokio::fs::copy(&agent_toml, &config_backup)
            .await
            .map_err(|e| format!("Failed to backup agent.toml: {}", e))?;
    }

    info!(backup_dir = %backup_dir.display(), "backup complete");

    // ── Replace binary ─────────────────────────────────────────────────
    info!("replacing binary");

    // Rename current binary to .bak
    let bak_path = current_binary.with_extension("bak");
    tokio::fs::rename(&current_binary, &bak_path)
        .await
        .map_err(|e| format!("Failed to rename current binary: {}", e))?;

    // Copy new binary into place
    if let Err(e) = tokio::fs::copy(&new_binary_path, &current_binary).await {
        // Restore from .bak on failure
        let _ = tokio::fs::rename(&bak_path, &current_binary).await;
        return Err(format!("Failed to install new binary: {}", e));
    }

    // chmod +x
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&current_binary, perms)
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    info!(version = %new_version, "binary replaced");

    // ── Restart ────────────────────────────────────────────────────────
    info!("spawning new process");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let child = std::process::Command::new(&current_binary)
        .args(&args)
        .spawn()
        .map_err(|e| format!("Failed to spawn new process: {}", e))?;

    // Monitor child for 30s — if it exits early, roll back
    let child_id = child.id();
    info!(
        pid = child_id,
        "new process spawned, monitoring for {}s",
        HEALTH_TIMEOUT.as_secs()
    );

    tokio::spawn(async move {
        monitor_and_shutdown(child, &bak_path, &current_binary, &agent_home, shutdown_tx).await;
    });

    // Clean up temp files
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    Ok(())
}

/// Monitor the newly spawned child process. If it exits within the timeout,
/// roll back. Otherwise, signal the old process to shut down.
async fn monitor_and_shutdown(
    mut child: std::process::Child,
    bak_path: &Path,
    binary_path: &Path,
    agent_home: &Path,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
) {
    let bak = bak_path.to_path_buf();
    let bin = binary_path.to_path_buf();
    let home = agent_home.to_path_buf();

    // Poll child status for up to HEALTH_TIMEOUT
    let deadline = tokio::time::Instant::now() + HEALTH_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                error!(
                    code = ?status.code(),
                    "new process exited with error, rolling back"
                );
                rollback(&bak, &bin, &home).await;
                return;
            }
            Ok(Some(_)) => {
                // Exited successfully (unusual for a server) — don't roll back
                // but do shut down the old process
                break;
            }
            Ok(None) => {
                // Still running
                if tokio::time::Instant::now() >= deadline {
                    info!(
                        "new process healthy after {}s, shutting down old process",
                        HEALTH_TIMEOUT.as_secs()
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                error!(error = %e, "failed to check child status, shutting down anyway");
                break;
            }
        }
    }

    // Clean up .bak file
    let _ = tokio::fs::remove_file(&bak).await;

    // Signal graceful shutdown
    let _ = shutdown_tx.send(true);
}

/// Roll back: restore the old binary and log guidance for DB/config restoration.
async fn rollback(bak_path: &Path, binary_path: &Path, agent_home: &Path) {
    warn!("rolling back to previous binary");
    if let Err(e) = tokio::fs::rename(bak_path, binary_path).await {
        error!(error = %e, "CRITICAL: failed to restore binary from backup");
        error!(
            "Manual recovery: copy from {}/backups/ to {}",
            agent_home.display(),
            binary_path.display()
        );
    } else {
        info!("binary restored from .bak");
    }
}

/// Find the `starpod` binary inside an extracted tarball directory.
fn find_binary_in_dir(dir: &Path) -> Result<PathBuf, String> {
    for entry in walkdir(dir)? {
        let name = entry
            .file_name()
            .ok_or("no filename")?
            .to_str()
            .ok_or("invalid filename")?;
        if name == "starpod" {
            return Ok(entry);
        }
    }
    Err("starpod binary not found in tarball".to_string())
}

/// Simple recursive directory walk.
fn walkdir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut results = Vec::new();
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}", e))?;
        let path = entry.path();
        if path.is_dir() {
            results.extend(walkdir(&path)?);
        } else {
            results.push(path);
        }
    }
    Ok(results)
}

// ── Helpers ────────────────────────────────────────────────────────────

fn sys_err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (status, Json(ErrorResponse { error: msg.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_newer ───────────────────────────────────────────────────────

    #[test]
    fn newer_patch_version() {
        assert!(is_newer("0.2.2", "0.2.1"));
    }

    #[test]
    fn newer_minor_version() {
        assert!(is_newer("0.3.0", "0.2.1"));
    }

    #[test]
    fn newer_major_version() {
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn same_version_not_newer() {
        assert!(!is_newer("0.2.1", "0.2.1"));
    }

    #[test]
    fn older_patch_not_newer() {
        assert!(!is_newer("0.2.0", "0.2.1"));
    }

    #[test]
    fn older_minor_not_newer() {
        assert!(!is_newer("0.1.0", "0.2.1"));
    }

    #[test]
    fn older_major_not_newer() {
        assert!(!is_newer("0.2.1", "1.0.0"));
    }

    #[test]
    fn prerelease_not_newer_than_release() {
        // 1.0.0-alpha < 1.0.0 per semver spec
        assert!(!is_newer("1.0.0-alpha", "1.0.0"));
    }

    #[test]
    fn release_newer_than_prerelease() {
        assert!(is_newer("1.0.0", "1.0.0-alpha"));
    }

    #[test]
    fn invalid_semver_returns_false() {
        assert!(!is_newer("not-a-version", "0.2.1"));
        assert!(!is_newer("0.2.1", "garbage"));
        assert!(!is_newer("", ""));
    }

    #[test]
    fn large_version_numbers() {
        assert!(is_newer("100.200.300", "100.200.299"));
        assert!(!is_newer("100.200.300", "100.200.300"));
    }

    // ── platform_target ────────────────────────────────────────────────

    #[test]
    fn platform_target_returns_known_triple() {
        let target = platform_target();
        assert!(!target.is_empty());

        let known = [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "unsupported",
        ];
        assert!(known.contains(&target), "unexpected target: {}", target);
    }

    #[cfg(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    ))]
    #[test]
    fn platform_target_not_unsupported_on_known_platforms() {
        assert_ne!(platform_target(), "unsupported");
    }

    // ── find_binary_in_dir / walkdir ───────────────────────────────────

    #[test]
    fn find_binary_flat_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("starpod"), b"fake-binary").unwrap();
        std::fs::write(dir.path().join("README.md"), b"readme").unwrap();

        let result = find_binary_in_dir(dir.path()).unwrap();
        assert_eq!(result.file_name().unwrap(), "starpod");
    }

    #[test]
    fn find_binary_nested_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("starpod-0.3.0").join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("starpod"), b"fake-binary").unwrap();

        let result = find_binary_in_dir(dir.path()).unwrap();
        assert_eq!(result.file_name().unwrap(), "starpod");
        assert!(result.starts_with(dir.path()));
    }

    #[test]
    fn find_binary_missing_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("not-starpod"), b"wrong").unwrap();

        let result = find_binary_in_dir(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn find_binary_empty_dir_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = find_binary_in_dir(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn walkdir_collects_all_files_recursively() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("b.txt"), b"").unwrap();
        let deep = sub.join("deep");
        std::fs::create_dir(&deep).unwrap();
        std::fs::write(deep.join("c.txt"), b"").unwrap();

        let files = walkdir(dir.path()).unwrap();
        assert_eq!(files.len(), 3);
        let names: Vec<&str> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
        assert!(names.contains(&"c.txt"));
    }

    #[test]
    fn walkdir_nonexistent_dir_returns_error() {
        let result = walkdir(Path::new("/nonexistent/path/12345"));
        assert!(result.is_err());
    }

    // ── SHA-256 verification ───────────────────────────────────────────

    #[test]
    fn sha256_computation_matches_known_value() {
        // SHA-256 of "hello world\n" (echo "hello world" | sha256sum)
        let data = b"hello world\n";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hex = format!("{:x}", hasher.finalize());
        assert_eq!(
            hex,
            "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447"
        );
    }

    // ── Checksums file parsing ─────────────────────────────────────────

    #[test]
    fn parse_checksums_standard_format() {
        // Mimics typical SHA256SUMS format: "hash  filename"
        let lines = "abc123  starpod-x86_64-unknown-linux-gnu.tar.gz\ndef456  starpod-aarch64-apple-darwin.tar.gz\n";
        let parsed: Vec<(String, String)> = lines
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some((parts[1].to_string(), parts[0].to_string()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "starpod-x86_64-unknown-linux-gnu.tar.gz");
        assert_eq!(parsed[0].1, "abc123");
        assert_eq!(parsed[1].0, "starpod-aarch64-apple-darwin.tar.gz");
        assert_eq!(parsed[1].1, "def456");
    }

    #[test]
    fn parse_checksums_ignores_blank_lines() {
        let lines = "\n\nabc123  file.tar.gz\n\n";
        let parsed: Vec<(String, String)> = lines
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some((parts[1].to_string(), parts[0].to_string()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(parsed.len(), 1);
    }

    // ── GitHub response parsing ────────────────────────────────────────

    #[test]
    fn github_release_deserializes_minimal() {
        let json = serde_json::json!({
            "tag_name": "v0.3.0",
            "html_url": "https://github.com/sinaptik-ai/starpod/releases/tag/v0.3.0",
            "assets": [],
        });
        let release: GitHubRelease = serde_json::from_value(json).unwrap();
        assert_eq!(release.tag_name, "v0.3.0");
        assert!(release.published_at.is_none());
        assert!(release.body.is_none());
        assert!(release.assets.is_empty());
    }

    #[test]
    fn github_release_deserializes_full() {
        let json = serde_json::json!({
            "tag_name": "v0.3.0",
            "published_at": "2026-03-30T10:00:00Z",
            "html_url": "https://github.com/sinaptik-ai/starpod/releases/tag/v0.3.0",
            "body": "## What's new\n- Feature A\n- Fix B",
            "assets": [
                {
                    "name": "starpod-aarch64-apple-darwin.tar.gz",
                    "browser_download_url": "https://github.com/download/starpod-aarch64-apple-darwin.tar.gz"
                },
                {
                    "name": "SHA256SUMS",
                    "browser_download_url": "https://github.com/download/SHA256SUMS"
                }
            ]
        });
        let release: GitHubRelease = serde_json::from_value(json).unwrap();
        assert_eq!(release.tag_name, "v0.3.0");
        assert_eq!(
            release.published_at.as_deref(),
            Some("2026-03-30T10:00:00Z")
        );
        assert_eq!(release.assets.len(), 2);
        assert_eq!(
            release.assets[0].name,
            "starpod-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn tag_name_v_prefix_stripped() {
        let tag = "v0.3.0";
        let version = tag.strip_prefix('v').unwrap_or(tag);
        assert_eq!(version, "0.3.0");
    }

    #[test]
    fn tag_name_without_v_preserved() {
        let tag = "0.3.0";
        let version = tag.strip_prefix('v').unwrap_or(tag);
        assert_eq!(version, "0.3.0");
    }

    // ── Asset matching ─────────────────────────────────────────────────

    #[test]
    fn asset_name_matches_target_triples() {
        let targets = [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ];

        for target in &targets {
            let name = format!("starpod-{}.tar.gz", target);
            assert!(
                name.contains(target) && name.ends_with(".tar.gz"),
                "asset {} should match target {}",
                name,
                target
            );
        }
    }

    #[test]
    fn asset_name_non_tarball_ignored() {
        let name = "starpod-x86_64-unknown-linux-gnu.zip";
        assert!(!name.ends_with(".tar.gz"));
    }

    #[test]
    fn checksums_file_detected_by_name() {
        for name in &["SHA256SUMS", "checksums.txt", "CHECKSUMS", "sha256sums.txt"] {
            let lower = name.to_lowercase();
            assert!(
                lower.contains("checksum") || lower.contains("sha256"),
                "{} should be detected as checksums file",
                name
            );
        }
    }

    // ── CachedRelease ──────────────────────────────────────────────────

    #[test]
    fn cached_release_clone_preserves_fields() {
        let mut assets = std::collections::HashMap::new();
        assets.insert(
            "aarch64-apple-darwin".to_string(),
            AssetInfo {
                url: "https://example.com/binary.tar.gz".to_string(),
                sha256: Some("abc123".to_string()),
            },
        );

        let release = CachedRelease {
            version: "0.3.0".to_string(),
            published_at: "2026-03-30T10:00:00Z".to_string(),
            release_notes_url: "https://github.com/releases/v0.3.0".to_string(),
            assets,
            fetched_at: tokio::time::Instant::now(),
        };

        let cloned = release.clone();
        assert_eq!(cloned.version, "0.3.0");
        assert_eq!(cloned.assets.len(), 1);
        assert_eq!(
            cloned.assets["aarch64-apple-darwin"].sha256.as_deref(),
            Some("abc123")
        );
    }

    // ── UpdateCache ────────────────────────────────────────────────────

    #[tokio::test]
    async fn new_update_cache_starts_empty() {
        let cache = new_update_cache();
        assert!(cache.read().await.is_none());
    }

    #[tokio::test]
    async fn update_cache_stores_and_retrieves() {
        let cache = new_update_cache();
        let release = CachedRelease {
            version: "1.0.0".to_string(),
            published_at: String::new(),
            release_notes_url: String::new(),
            assets: std::collections::HashMap::new(),
            fetched_at: tokio::time::Instant::now(),
        };
        *cache.write().await = Some(release);
        let read = cache.read().await;
        assert_eq!(read.as_ref().unwrap().version, "1.0.0");
    }

    // ── Tarball extraction (end-to-end with real tar.gz) ───────────────

    #[test]
    fn extract_and_find_binary_from_tarball() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();

        // Create a tar.gz containing "starpod" binary
        let tarball_path = dir.path().join("test.tar.gz");
        {
            let file = std::fs::File::create(&tarball_path).unwrap();
            let enc = GzEncoder::new(file, Compression::fast());
            let mut builder = tar::Builder::new(enc);

            // Add a "starpod" file
            let data = b"#!/bin/sh\necho hello";
            let mut header = tar::Header::new_gnu();
            header.set_path("starpod-0.3.0/starpod").unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, &data[..]).unwrap();

            // Add a README
            let readme = b"Release notes";
            let mut rh = tar::Header::new_gnu();
            rh.set_path("starpod-0.3.0/README.md").unwrap();
            rh.set_size(readme.len() as u64);
            rh.set_mode(0o644);
            rh.set_cksum();
            builder.append(&rh, &readme[..]).unwrap();

            builder.into_inner().unwrap().finish().unwrap();
        }

        // Extract
        let extract_dir = dir.path().join("extracted");
        std::fs::create_dir_all(&extract_dir).unwrap();
        let tarball_data = std::fs::read(&tarball_path).unwrap();
        let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(tarball_data));
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(&extract_dir).unwrap();

        // Find binary
        let binary = find_binary_in_dir(&extract_dir).unwrap();
        assert_eq!(binary.file_name().unwrap(), "starpod");
        assert!(binary.exists());

        // Verify content
        let content = std::fs::read(&binary).unwrap();
        assert_eq!(content, b"#!/bin/sh\necho hello");
    }

    // ── VersionResponse serialization ──────────────────────────────────

    #[test]
    fn version_response_serializes_all_fields() {
        let resp = VersionResponse {
            current: "0.2.1".to_string(),
            latest: Some("0.3.0".to_string()),
            update_available: true,
            release_notes_url: Some("https://example.com".to_string()),
            published_at: Some("2026-03-30T10:00:00Z".to_string()),
            platform: "aarch64-apple-darwin".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["current"], "0.2.1");
        assert_eq!(json["latest"], "0.3.0");
        assert_eq!(json["update_available"], true);
        assert_eq!(json["platform"], "aarch64-apple-darwin");
    }

    #[test]
    fn version_response_with_no_latest() {
        let resp = VersionResponse {
            current: "0.2.1".to_string(),
            latest: None,
            update_available: false,
            release_notes_url: None,
            published_at: None,
            platform: "aarch64-apple-darwin".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["current"], "0.2.1");
        assert!(json["latest"].is_null());
        assert_eq!(json["update_available"], false);
    }

    // ── UpdateResponse serialization ───────────────────────────────────

    #[test]
    fn update_response_serializes() {
        let resp = UpdateResponse {
            status: "updating".to_string(),
            version: "0.3.0".to_string(),
            message: "Update started.".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "updating");
        assert_eq!(json["version"], "0.3.0");
        assert_eq!(json["message"], "Update started.");
    }

    // ── AssetInfo serialization round-trip ──────────────────────────────

    #[test]
    fn asset_info_round_trips_through_json() {
        let info = AssetInfo {
            url: "https://example.com/binary.tar.gz".to_string(),
            sha256: Some("deadbeef".to_string()),
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: AssetInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.url, info.url);
        assert_eq!(back.sha256, info.sha256);
    }

    #[test]
    fn asset_info_with_null_sha256() {
        let info = AssetInfo {
            url: "https://example.com".to_string(),
            sha256: None,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert!(json["sha256"].is_null());
    }

    // ── sys_err helper ─────────────────────────────────────────────────

    #[test]
    fn sys_err_returns_correct_status_and_message() {
        let (status, Json(body)) = sys_err(StatusCode::CONFLICT, "test message");
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "test message");
    }

    #[test]
    fn sys_err_accepts_string_type() {
        let msg = format!("error: {}", 42);
        let (status, Json(body)) = sys_err(StatusCode::BAD_GATEWAY, msg);
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body.error, "error: 42");
    }

    // ── CURRENT_VERSION ────────────────────────────────────────────────

    #[test]
    fn current_version_is_valid_semver() {
        semver::Version::parse(CURRENT_VERSION).expect("CARGO_PKG_VERSION should be valid semver");
    }

    // ── CACHE_TTL / HEALTH_TIMEOUT constants ───────────────────────────

    #[test]
    fn cache_ttl_is_one_hour() {
        assert_eq!(CACHE_TTL, Duration::from_secs(3600));
    }

    #[test]
    fn health_timeout_is_30_seconds() {
        assert_eq!(HEALTH_TIMEOUT, Duration::from_secs(30));
    }
}
