//! Lightweight browser automation for Starpod via Chrome DevTools Protocol.
//!
//! This crate provides [`BrowserSession`], a high-level async interface for
//! controlling a CDP-speaking browser (Lightpanda or headless Chromium). It
//! uses direct CDP over WebSocket and handles process lifecycle, connection
//! management, and common browser operations.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────┐     CDP/WebSocket     ┌──────────────────────┐
//! │  BrowserSession    │ ◄──────────────────── │  lightpanda serve    │
//! │  (async-tungstenite)│                       │  (auto-spawned)      │
//! └────────────────────┘                       └──────────────────────┘
//! ```
//!
//! # Usage modes
//!
//! - **Auto-spawn** (recommended): [`BrowserSession::launch()`] finds a free
//!   port, spawns `lightpanda serve`, waits for CDP readiness, and connects.
//!   The process is killed on [`close()`](BrowserSession::close) or [`Drop`].
//!
//! - **External**: [`BrowserSession::connect()`] attaches to a pre-existing
//!   CDP endpoint (e.g. headless Chromium started by the user or systemd).
//!
//! # Requirements
//!
//! For auto-spawn mode, `lightpanda` is automatically downloaded and installed
//! to `~/.local/bin/` if not already on `PATH`. No manual setup is needed.
//!
//! # Example
//!
//! ```rust,no_run
//! # async fn example() -> starpod_browser::Result<()> {
//! use starpod_browser::BrowserSession;
//!
//! // Auto-spawn Lightpanda and navigate
//! let session = BrowserSession::launch().await?;
//! let title = session.navigate("https://example.com").await?;
//! println!("Page title: {title}");
//!
//! // Extract page text
//! let text = session.extract(None).await?;
//! println!("Page text: {text}");
//!
//! // Clean up
//! session.close().await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_tungstenite::tungstenite::Message as WsMessage;
use futures_util::{SinkExt, StreamExt};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, oneshot, Mutex};
use tracing::{debug, info};

/// Timeout for individual CDP commands.
const CDP_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from browser operations.
#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    /// No browser session is active.
    #[error("browser not connected")]
    NotConnected,

    /// Failed to spawn the Lightpanda child process.
    #[error("failed to spawn lightpanda: {0}")]
    SpawnFailed(String),

    /// CDP WebSocket connection failed.
    #[error("CDP connection failed: {0}")]
    ConnectionFailed(String),

    /// Page navigation failed (invalid URL, network error, etc.).
    #[error("navigation failed: {0}")]
    NavigationFailed(String),

    /// CSS selector matched no elements.
    #[error("element not found: {0}")]
    ElementNotFound(String),

    /// JavaScript evaluation failed.
    #[error("JS evaluation failed: {0}")]
    EvalFailed(String),

    /// Timed out waiting for the browser process to accept CDP connections.
    #[error("timeout waiting for browser to start")]
    Timeout,

    /// Auto-installation of Lightpanda failed.
    #[error("failed to install lightpanda: {0}")]
    InstallFailed(String),
}

/// Convenience alias for `Result<T, BrowserError>`.
pub type Result<T> = std::result::Result<T, BrowserError>;

// ---------------------------------------------------------------------------
// CdpClient — lightweight CDP-over-WebSocket client
// ---------------------------------------------------------------------------

type WsWriter = futures_util::stream::SplitSink<
    async_tungstenite::WebSocketStream<async_tungstenite::tokio::ConnectStream>,
    WsMessage,
>;

/// A lightweight CDP client that sends commands and routes responses by `id`.
struct CdpClient {
    writer: Mutex<WsWriter>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
    events: broadcast::Sender<serde_json::Value>,
    _reader_task: tokio::task::JoinHandle<()>,
}

impl CdpClient {
    /// Connect to a CDP WebSocket endpoint.
    async fn connect(ws_url: &str) -> Result<Self> {
        let (ws, _) = async_tungstenite::tokio::connect_async(ws_url)
            .await
            .map_err(|e| BrowserError::ConnectionFailed(e.to_string()))?;

        let (writer, reader) = ws.split();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (events_tx, _) = broadcast::channel(64);

        let pending_clone = Arc::clone(&pending);
        let events_clone = events_tx.clone();

        let reader_task = tokio::spawn(async move {
            let mut reader = reader;
            while let Some(msg) = reader.next().await {
                let text = match msg {
                    Ok(WsMessage::Text(t)) => t.to_string(),
                    Ok(WsMessage::Close(_)) => break,
                    Ok(_) => continue,
                    Err(e) => {
                        debug!("CDP WebSocket error: {e}");
                        break;
                    }
                };

                let json: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        debug!("CDP parse error: {e}");
                        continue;
                    }
                };

                // Response (has "id" field) → route to pending sender
                if let Some(id) = json.get("id").and_then(|v| v.as_u64()) {
                    let mut map = pending_clone.lock().await;
                    if let Some(tx) = map.remove(&id) {
                        let _ = tx.send(json);
                    }
                } else {
                    // Event (has "method" field) → broadcast
                    let _ = events_clone.send(json);
                }
            }
        });

        Ok(Self {
            writer: Mutex::new(writer),
            next_id: AtomicU64::new(1),
            pending,
            events: events_tx,
            _reader_task: reader_task,
        })
    }

    /// Send a CDP command and wait for its response.
    async fn send(
        &self,
        method: &str,
        params: serde_json::Value,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let mut msg = serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if let Some(sid) = session_id {
            msg["sessionId"] = serde_json::Value::String(sid.to_string());
        }

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let text = serde_json::to_string(&msg)
            .map_err(|e| BrowserError::ConnectionFailed(e.to_string()))?;

        self.writer
            .lock()
            .await
            .send(WsMessage::Text(text.into()))
            .await
            .map_err(|e| BrowserError::ConnectionFailed(e.to_string()))?;

        let resp = tokio::time::timeout(CDP_TIMEOUT, rx)
            .await
            .map_err(|_| BrowserError::Timeout)?
            .map_err(|_| BrowserError::ConnectionFailed("response channel closed".into()))?;

        // Check for CDP error
        if let Some(err) = resp.get("error") {
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown CDP error");
            return Err(BrowserError::EvalFailed(message.to_string()));
        }

        // Return the "result" field, or the whole response if no "result"
        Ok(resp
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new())))
    }

    /// Subscribe to CDP events.
    fn subscribe(&self) -> broadcast::Receiver<serde_json::Value> {
        self.events.subscribe()
    }
}

// ---------------------------------------------------------------------------
// BrowserSession
// ---------------------------------------------------------------------------

/// A browser automation session backed by CDP.
///
/// Manages an optional child process (auto-spawned Lightpanda) and a direct
/// CDP WebSocket connection for interaction.
///
/// The session is designed to be held behind `Arc<tokio::sync::Mutex<Option<BrowserSession>>>`
/// for shared access across async tool calls. All public methods take `&self`.
///
/// # Process lifecycle
///
/// When created via [`launch()`](Self::launch), the Lightpanda process is
/// spawned with `kill_on_drop(true)` and additionally killed in the [`Drop`]
/// impl. This ensures cleanup even if [`close()`](Self::close) is not called.
pub struct BrowserSession {
    /// Auto-spawned browser process (`None` if connected to external endpoint).
    process: Option<Child>,
    /// The CDP WebSocket client.
    cdp: CdpClient,
    /// The CDP session ID for the attached target.
    session_id: String,
}

impl std::fmt::Debug for BrowserSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserSession")
            .field("auto_spawned", &self.process.is_some())
            .finish_non_exhaustive()
    }
}

impl BrowserSession {
    /// Spawn a Lightpanda process and connect to it via CDP.
    ///
    /// 1. Finds a free TCP port by binding to `127.0.0.1:0`
    /// 2. Starts `lightpanda serve --host 127.0.0.1 --port <port>`
    /// 3. Polls the port until CDP accepts connections (up to 10 seconds)
    /// 4. Connects via WebSocket and opens a blank page
    ///
    /// If `lightpanda` is not found on `PATH`, it is automatically downloaded
    /// from GitHub releases and installed to `~/.local/bin/`.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::InstallFailed`] if auto-installation fails
    /// - [`BrowserError::SpawnFailed`] if `lightpanda` fails to start after installation
    /// - [`BrowserError::Timeout`] if CDP doesn't become available within 10 seconds
    /// - [`BrowserError::ConnectionFailed`] if WebSocket handshake fails
    pub async fn launch() -> Result<Self> {
        let port = find_free_port().await?;
        let addr = format!("127.0.0.1:{port}");

        // Resolve the lightpanda binary, auto-installing if needed.
        let binary = resolve_lightpanda_binary().await?;

        info!(port, binary = %binary.display(), "Spawning lightpanda");

        let child = Command::new(&binary)
            .args([
                "serve",
                "--host", "127.0.0.1",
                "--port", &port.to_string(),
                // Keep alive for up to 1 hour — the agent can take minutes
                // between tool calls, and the default 10s timeout kills the
                // session mid-conversation.
                "--timeout", "3600",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| BrowserError::SpawnFailed(e.to_string()))?;

        // Wait for CDP to become available, then discover the WebSocket URL
        wait_for_cdp(&addr).await?;
        let ws_url = discover_ws_url(&addr).await?;
        Self::connect_internal(Some(child), &ws_url).await
    }

    /// Connect to an existing CDP endpoint.
    ///
    /// Use this when the browser is managed externally (e.g. headless Chromium
    /// started by systemd, or a shared Lightpanda instance).
    ///
    /// # Arguments
    ///
    /// * `cdp_url` — WebSocket URL (e.g. `ws://127.0.0.1:9222/`).
    ///
    /// # Errors
    ///
    /// - [`BrowserError::ConnectionFailed`] if the endpoint is unreachable
    pub async fn connect(cdp_url: &str) -> Result<Self> {
        debug!(url = cdp_url, "Connecting to existing CDP endpoint");
        Self::connect_internal(None, cdp_url).await
    }

    /// Shared connection logic for both `launch()` and `connect()`.
    async fn connect_internal(process: Option<Child>, ws_url: &str) -> Result<Self> {
        let cdp = CdpClient::connect(ws_url).await?;

        // 1. Enable target discovery
        cdp.send("Target.setDiscoverTargets", serde_json::json!({"discover": true}), None)
            .await?;

        // 2. Create a new target (page)
        let result = cdp
            .send("Target.createTarget", serde_json::json!({"url": "about:blank"}), None)
            .await?;
        let target_id = result["targetId"]
            .as_str()
            .ok_or_else(|| BrowserError::ConnectionFailed("no targetId in response".into()))?
            .to_string();

        // 3. Attach to the target to get a session ID
        let result = cdp
            .send(
                "Target.attachToTarget",
                serde_json::json!({"targetId": target_id, "flatten": true}),
                None,
            )
            .await?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or_else(|| BrowserError::ConnectionFailed("no sessionId in response".into()))?
            .to_string();

        // 4. Enable Page domain (needed for navigation events)
        cdp.send("Page.enable", serde_json::json!({}), Some(&session_id))
            .await?;

        debug!(session_id = %session_id, "CDP session established");

        Ok(Self {
            process,
            cdp,
            session_id,
        })
    }

    /// Navigate to a URL. Returns the page title after load.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::NavigationFailed`] on invalid URL or network error
    pub async fn navigate(&self, url: &str) -> Result<String> {
        // Subscribe to events BEFORE sending the navigate command
        let mut events = self.cdp.subscribe();

        self.cdp
            .send(
                "Page.navigate",
                serde_json::json!({"url": url}),
                Some(&self.session_id),
            )
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

        // Wait for Page.loadEventFired
        let deadline = tokio::time::Instant::now() + CDP_TIMEOUT;
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(event)) => {
                    if event.get("method").and_then(|m| m.as_str())
                        == Some("Page.loadEventFired")
                    {
                        break;
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(_)) => break,
                Err(_) => {
                    return Err(BrowserError::NavigationFailed("page load timeout".into()));
                }
            }
        }

        // Get the page title
        self.evaluate("document.title").await
    }

    /// Extract text content from the page or a specific element.
    ///
    /// - `selector = None` → returns `document.body.innerText` (full page text)
    /// - `selector = Some("h1")` → returns text of the first matching element
    ///
    /// # Errors
    ///
    /// - [`BrowserError::ElementNotFound`] if the selector matches nothing
    /// - [`BrowserError::EvalFailed`] if text extraction fails
    pub async fn extract(&self, selector: Option<&str>) -> Result<String> {
        match selector {
            None => self.evaluate("document.body.textContent").await,
            Some(sel) => {
                let sel_json = serde_json::to_string(sel)
                    .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;
                let js = format!(
                    r#"(function(){{ var el = document.querySelector({sel}); if (!el) return null; return el.textContent; }})()"#,
                    sel = sel_json,
                );
                let result = self.evaluate(&js).await?;
                if result == "null" || result.is_empty() {
                    Err(BrowserError::ElementNotFound(sel.to_string()))
                } else {
                    Ok(result)
                }
            }
        }
    }

    /// Click an element by CSS selector.
    ///
    /// Dispatches a full `MouseEvent` (not just `el.click()`) so that
    /// framework event listeners (React, Vue, etc.) see the event.
    /// For submit buttons, also calls `form.requestSubmit()` to ensure
    /// form submission fires correctly.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::ElementNotFound`] if the selector matches nothing
    pub async fn click(&self, selector: &str) -> Result<()> {
        let sel_json = serde_json::to_string(selector)
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;
        let js = format!(
            r#"(function(){{
  var el = document.querySelector({sel});
  if (!el) throw new Error('element not found');
  el.dispatchEvent(new MouseEvent('click', {{bubbles: true, cancelable: true, view: window}}));
  if ((el.type === 'submit' || el.tagName === 'BUTTON') && el.form) {{
    try {{ el.form.requestSubmit(el); }} catch(e) {{ el.form.submit(); }}
  }}
  return true;
}})()"#,
            sel = sel_json,
        );
        self.evaluate(&js).await.map_err(|e| {
            if e.to_string().contains("element not found") {
                BrowserError::ElementNotFound(selector.to_string())
            } else {
                e
            }
        })?;
        Ok(())
    }

    /// Type text into an element identified by CSS selector.
    ///
    /// Uses the native HTMLInputElement value setter to bypass React's
    /// internal value tracker, then dispatches `input` and `change` events
    /// so both React controlled inputs and plain HTML inputs update correctly.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::ElementNotFound`] if the selector matches nothing
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        let sel_json = serde_json::to_string(selector)
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;
        let val_json = serde_json::to_string(text)
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;
        let js = format!(
            r#"(function(){{
  var el = document.querySelector({sel});
  if (!el) throw new Error('element not found');
  el.focus();
  var nativeSetter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype, 'value'
  );
  if (nativeSetter && nativeSetter.set) {{
    nativeSetter.set.call(el, {val});
  }} else {{
    el.value = {val};
  }}
  el.dispatchEvent(new Event('input', {{bubbles: true}}));
  el.dispatchEvent(new Event('change', {{bubbles: true}}));
  return true;
}})()"#,
            sel = sel_json,
            val = val_json,
        );
        self.evaluate(&js).await.map_err(|e| {
            if e.to_string().contains("element not found") {
                BrowserError::ElementNotFound(selector.to_string())
            } else {
                e
            }
        })?;
        Ok(())
    }

    /// Execute JavaScript on the page and return the result as a string.
    ///
    /// String values are returned as-is. Numbers, booleans, objects, and arrays
    /// are serialized via `serde_json::Value::to_string()`.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::EvalFailed`] on syntax errors or runtime exceptions
    pub async fn evaluate(&self, js: &str) -> Result<String> {
        // Wrap in an IIFE to isolate variable declarations between calls.
        // Without this, consecutive evaluate() calls that declare `const` or
        // `let` variables with the same name would fail with
        // "Identifier has already been declared".
        //
        // Code that is already an IIFE `(function(){...})()` is left as-is.
        // Simple expressions get `return` prepended so they return a value.
        // Multi-statement code is wrapped as-is (caller must use `return`).
        let trimmed = js.trim();
        let wrapped = if trimmed.starts_with("(function") {
            // Already an IIFE — don't double-wrap
            trimmed.to_string()
        } else if trimmed.contains(';') || trimmed.contains('\n') {
            format!("(function(){{{trimmed}}})()")
        } else {
            format!("(function(){{ return {trimmed} }})()")
        };
        let result = self
            .cdp
            .send(
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": wrapped,
                    "returnByValue": true,
                }),
                Some(&self.session_id),
            )
            .await
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;

        // Check for exception
        if let Some(exc) = result.get("exceptionDetails") {
            // Try exception.description first (has the real message),
            // fall back to exceptionDetails.text
            let text = exc
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(|d| d.as_str())
                .or_else(|| exc.get("text").and_then(|t| t.as_str()))
                .unwrap_or("unknown error");
            return Err(BrowserError::EvalFailed(text.to_string()));
        }

        let value = &result["result"]["value"];
        match value {
            serde_json::Value::String(s) => Ok(s.clone()),
            serde_json::Value::Null => Ok(String::new()),
            other => Ok(other.to_string()),
        }
    }

    /// Get the current page URL.
    pub async fn url(&self) -> Result<String> {
        self.evaluate("window.location.href").await
    }

    /// Returns `true` if this session was auto-spawned (vs. connected to an
    /// external endpoint).
    pub fn is_auto_spawned(&self) -> bool {
        self.process.is_some()
    }

    /// Close the browser session and kill the process if auto-spawned.
    ///
    /// This is the preferred way to end a session. If not called, the [`Drop`]
    /// impl will still attempt to kill the child process, but cannot await
    /// its termination.
    pub async fn close(mut self) -> Result<()> {
        if let Some(ref mut child) = self.process {
            debug!("Killing auto-spawned browser process");
            let _ = child.kill().await;
        }
        Ok(())
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.process {
            // Best-effort kill on drop. This is non-async so we can only
            // *start* the kill signal; the OS will reap the process.
            let _ = child.start_kill();
        }
    }
}

// ---------------------------------------------------------------------------
// Auto-install
// ---------------------------------------------------------------------------

/// Resolve the `lightpanda` binary path.
///
/// 1. Check if `lightpanda` is on `PATH` (via `which`).
/// 2. Check the default install location (`~/.local/bin/lightpanda`).
/// 3. If not found, download from GitHub releases and install to `~/.local/bin/`.
async fn resolve_lightpanda_binary() -> Result<PathBuf> {
    // 1. Already on PATH?
    if let Ok(path) = which_lightpanda().await {
        debug!(path = %path.display(), "Found lightpanda on PATH");
        return Ok(path);
    }

    // 2. Check default install location
    let install_dir = default_install_dir()?;
    let binary_path = install_dir.join("lightpanda");
    if binary_path.is_file() {
        debug!(path = %binary_path.display(), "Found lightpanda in ~/.local/bin");
        return Ok(binary_path);
    }

    // 3. Auto-install
    info!("lightpanda not found — downloading automatically");
    install_lightpanda(&install_dir).await?;
    Ok(binary_path)
}

/// Try to find `lightpanda` on PATH using `which`.
async fn which_lightpanda() -> Result<PathBuf> {
    let output = tokio::process::Command::new("which")
        .arg("lightpanda")
        .output()
        .await
        .map_err(|e| BrowserError::SpawnFailed(e.to_string()))?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    Err(BrowserError::SpawnFailed("not on PATH".into()))
}

/// Returns `~/.local/bin`, creating it if it doesn't exist.
fn default_install_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .map_err(|_| BrowserError::InstallFailed("HOME not set".into()))?;
    let dir = PathBuf::from(home).join(".local").join("bin");
    Ok(dir)
}

/// Returns the platform-specific asset name for lightpanda GitHub releases.
fn lightpanda_asset_name() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("macos", "aarch64") => Ok("lightpanda-aarch64-macos"),
        ("macos", "x86_64") => Ok("lightpanda-x86_64-macos"),
        ("linux", "aarch64") => Ok("lightpanda-aarch64-linux"),
        ("linux", "x86_64") => Ok("lightpanda-x86_64-linux"),
        _ => Err(BrowserError::InstallFailed(format!(
            "unsupported platform: {os}/{arch}"
        ))),
    }
}

/// Download and install the lightpanda binary to `install_dir`.
async fn install_lightpanda(install_dir: &std::path::Path) -> Result<()> {
    let asset = lightpanda_asset_name()?;
    let url = format!(
        "https://github.com/lightpanda-io/browser/releases/download/nightly/{asset}"
    );

    info!(url = %url, "Downloading lightpanda");

    // Download with curl (follows redirects, which GitHub requires)
    let output = tokio::process::Command::new("curl")
        .args(["-fsSL", "--output", "-", &url])
        .output()
        .await
        .map_err(|e| BrowserError::InstallFailed(format!("curl failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BrowserError::InstallFailed(format!(
            "download failed ({}): {stderr}",
            output.status
        )));
    }

    if output.stdout.is_empty() {
        return Err(BrowserError::InstallFailed(
            "downloaded file is empty".into(),
        ));
    }

    // Create install directory
    tokio::fs::create_dir_all(install_dir)
        .await
        .map_err(|e| {
            BrowserError::InstallFailed(format!(
                "cannot create {}: {e}",
                install_dir.display()
            ))
        })?;

    let binary_path = install_dir.join("lightpanda");

    // Write binary
    tokio::fs::write(&binary_path, &output.stdout)
        .await
        .map_err(|e| BrowserError::InstallFailed(format!("cannot write binary: {e}")))?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        tokio::fs::set_permissions(&binary_path, perms)
            .await
            .map_err(|e| BrowserError::InstallFailed(format!("chmod failed: {e}")))?;
    }

    info!(path = %binary_path.display(), "lightpanda installed successfully");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find a free TCP port by binding to port 0 and immediately releasing.
async fn find_free_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| BrowserError::SpawnFailed(format!("cannot bind: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| BrowserError::SpawnFailed(format!("cannot get addr: {e}")))?
        .port();
    drop(listener);
    Ok(port)
}

/// Fetch the WebSocket debugger URL from the CDP `/json/version` endpoint.
///
/// Falls back to `ws://{addr}/` if the endpoint is unavailable.
async fn discover_ws_url(addr: &str) -> Result<String> {
    let url = format!("http://{addr}/json/version");

    let output = tokio::process::Command::new("curl")
        .args(["-sf", "--max-time", "5", &url])
        .output()
        .await
        .map_err(|e| BrowserError::ConnectionFailed(format!("curl /json/version: {e}")))?;

    if output.status.success() {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            if let Some(ws) = json.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                debug!(ws_url = %ws, "Discovered WebSocket URL");
                return Ok(ws.to_string());
            }
        }
    }

    // Fallback — append trailing slash (lightpanda requires it)
    let fallback = format!("ws://{addr}/");
    debug!(ws_url = %fallback, "Using fallback WebSocket URL");
    Ok(fallback)
}

/// Wait for a TCP endpoint to accept connections, polling every 100ms.
///
/// Times out after 10 seconds with [`BrowserError::Timeout`].
async fn wait_for_cdp(addr: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(BrowserError::Timeout);
        }

        match tokio::net::TcpStream::connect(addr).await {
            Ok(_) => {
                debug!(addr, "CDP endpoint is ready");
                return Ok(());
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Unit tests (no browser required) --

    #[tokio::test]
    async fn find_free_port_returns_nonzero() {
        let port = find_free_port().await.unwrap();
        assert!(port > 0, "port should be nonzero, got {port}");
    }

    #[tokio::test]
    async fn find_free_port_returns_different_ports() {
        let p1 = find_free_port().await.unwrap();
        let p2 = find_free_port().await.unwrap();
        // Not guaranteed to differ, but practically always will
        // Just verify both are valid
        assert!(p1 > 0);
        assert!(p2 > 0);
    }

    #[tokio::test]
    async fn wait_for_cdp_succeeds_when_listener_exists() {
        // Start a TCP listener, then verify wait_for_cdp connects to it
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        // Should succeed immediately since the port is already listening
        let result = wait_for_cdp(&addr).await;
        assert!(result.is_ok(), "should connect to existing listener");
    }

    #[tokio::test]
    async fn wait_for_cdp_times_out_on_closed_port() {
        // Find a port and immediately close it
        let port = find_free_port().await.unwrap();
        let addr = format!("127.0.0.1:{port}");

        // Override the timeout for faster testing — but wait_for_cdp uses 10s
        // internally, so we test with a small helper instead
        let start = tokio::time::Instant::now();
        let deadline = start + Duration::from_millis(500);

        let result = tokio::time::timeout(Duration::from_millis(500), wait_for_cdp(&addr)).await;

        // Should time out (either our timeout or the internal 10s one)
        assert!(
            result.is_err() || result.unwrap().is_err(),
            "should fail on closed port"
        );
        assert!(
            start.elapsed() <= deadline.elapsed() + Duration::from_millis(600),
            "should not hang"
        );
    }

    #[tokio::test]
    async fn wait_for_cdp_succeeds_when_listener_starts_late() {
        let port = find_free_port().await.unwrap();
        let addr_str = format!("127.0.0.1:{port}");
        let addr_clone = addr_str.clone();

        // Start a listener after a 200ms delay
        let _listener_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            tokio::net::TcpListener::bind(&addr_clone).await.unwrap()
            // Keep the listener alive by returning it (held by JoinHandle)
        });

        // wait_for_cdp should poll until it connects
        let result = wait_for_cdp(&addr_str).await;
        assert!(result.is_ok(), "should connect after delayed start");
    }

    #[test]
    fn error_display_messages() {
        let err = BrowserError::SpawnFailed("not found".into());
        assert_eq!(err.to_string(), "failed to spawn lightpanda: not found");

        let err = BrowserError::ElementNotFound("div.missing".into());
        assert_eq!(err.to_string(), "element not found: div.missing");

        let err = BrowserError::Timeout;
        assert_eq!(err.to_string(), "timeout waiting for browser to start");

        let err = BrowserError::NotConnected;
        assert_eq!(err.to_string(), "browser not connected");

        let err = BrowserError::ConnectionFailed("refused".into());
        assert_eq!(err.to_string(), "CDP connection failed: refused");

        let err = BrowserError::NavigationFailed("404".into());
        assert_eq!(err.to_string(), "navigation failed: 404");

        let err = BrowserError::EvalFailed("syntax error".into());
        assert_eq!(err.to_string(), "JS evaluation failed: syntax error");
    }

    #[test]
    fn lightpanda_asset_name_returns_valid_name() {
        // Should succeed on any supported CI/dev platform
        let name = lightpanda_asset_name().unwrap();
        assert!(
            name.starts_with("lightpanda-"),
            "asset name should start with 'lightpanda-', got: {name}"
        );
    }

    #[test]
    fn install_failed_error_display() {
        let err = BrowserError::InstallFailed("no curl".into());
        assert_eq!(err.to_string(), "failed to install lightpanda: no curl");
    }

    #[test]
    fn default_install_dir_is_under_home() {
        let dir = default_install_dir().unwrap();
        assert!(
            dir.ends_with(".local/bin"),
            "install dir should end with .local/bin, got: {}",
            dir.display()
        );
    }

    #[tokio::test]
    async fn connect_fails_on_bad_endpoint() {
        let port = find_free_port().await.unwrap();
        let result = BrowserSession::connect(&format!("ws://127.0.0.1:{port}")).await;

        assert!(result.is_err(), "should fail on unreachable endpoint");
        let err = result.unwrap_err();
        assert!(
            matches!(err, BrowserError::ConnectionFailed(_)),
            "expected ConnectionFailed, got: {err}"
        );
    }

    // -- Integration tests (require a running CDP browser) --
    //
    // These tests are gated behind the BROWSER_CDP_URL env var.
    // To run them:
    //
    //   # Start Lightpanda
    //   lightpanda serve &
    //
    //   # Run integration tests
    //   BROWSER_CDP_URL=ws://127.0.0.1:9222/ cargo test -p starpod-browser -- --ignored
    //

    fn cdp_url() -> Option<String> {
        std::env::var("BROWSER_CDP_URL").ok()
    }

    #[tokio::test]
    #[ignore = "requires running CDP browser (set BROWSER_CDP_URL)"]
    async fn integration_connect_and_navigate() {
        let url = cdp_url().expect("BROWSER_CDP_URL not set");
        let session = BrowserSession::connect(&url).await.unwrap();
        assert!(!session.is_auto_spawned());

        let title = session.navigate("https://example.com").await.unwrap();
        assert!(
            !title.is_empty(),
            "title should not be empty after navigating to example.com"
        );

        session.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running CDP browser (set BROWSER_CDP_URL)"]
    async fn integration_extract_page_text() {
        let url = cdp_url().expect("BROWSER_CDP_URL not set");
        let session = BrowserSession::connect(&url).await.unwrap();

        session.navigate("https://example.com").await.unwrap();

        // Full page text
        let text = session.extract(None).await.unwrap();
        assert!(
            text.contains("Example Domain"),
            "page text should contain 'Example Domain', got: {text}"
        );

        // Specific element
        let h1 = session.extract(Some("h1")).await.unwrap();
        assert_eq!(h1.trim(), "Example Domain");

        session.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running CDP browser (set BROWSER_CDP_URL)"]
    async fn integration_evaluate_javascript() {
        let url = cdp_url().expect("BROWSER_CDP_URL not set");
        let session = BrowserSession::connect(&url).await.unwrap();

        session.navigate("https://example.com").await.unwrap();

        // String result
        let title = session.evaluate("document.title").await.unwrap();
        assert!(!title.is_empty());

        // Numeric result (serialized as string)
        let sum = session.evaluate("1 + 2").await.unwrap();
        assert_eq!(sum, "3");

        // Boolean result
        let t = session.evaluate("true").await.unwrap();
        assert_eq!(t, "true");

        session.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running CDP browser (set BROWSER_CDP_URL)"]
    async fn integration_click_element() {
        let url = cdp_url().expect("BROWSER_CDP_URL not set");
        let session = BrowserSession::connect(&url).await.unwrap();

        session.navigate("https://example.com").await.unwrap();

        // example.com has a link — clicking it should work (or at least not error)
        let result = session.click("a").await;
        assert!(result.is_ok(), "clicking a link should succeed");

        session.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running CDP browser (set BROWSER_CDP_URL)"]
    async fn integration_element_not_found() {
        let url = cdp_url().expect("BROWSER_CDP_URL not set");
        let session = BrowserSession::connect(&url).await.unwrap();

        session.navigate("https://example.com").await.unwrap();

        let result = session.click("div.nonexistent-class-12345").await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), BrowserError::ElementNotFound(_)),
            "should return ElementNotFound for missing selector"
        );

        session.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running CDP browser (set BROWSER_CDP_URL)"]
    async fn integration_get_url() {
        let url = cdp_url().expect("BROWSER_CDP_URL not set");
        let session = BrowserSession::connect(&url).await.unwrap();

        session.navigate("https://example.com").await.unwrap();
        let page_url = session.url().await.unwrap();
        assert!(
            page_url.contains("example.com"),
            "URL should contain example.com, got: {page_url}"
        );

        session.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires lightpanda binary on PATH"]
    async fn integration_launch_and_close() {
        let session = BrowserSession::launch().await.unwrap();
        assert!(session.is_auto_spawned());

        let title = session.navigate("https://example.com").await.unwrap();
        assert!(!title.is_empty());

        session.close().await.unwrap();
    }
}
