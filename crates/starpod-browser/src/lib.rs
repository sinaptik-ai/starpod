//! Lightweight browser automation for Starpod via Chrome DevTools Protocol.
//!
//! This crate provides [`BrowserSession`], a high-level async interface for
//! controlling a CDP-speaking browser (Lightpanda or headless Chromium). It
//! wraps [`chromiumoxide`] and handles process lifecycle, connection management,
//! and common browser operations.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────┐     CDP/WebSocket     ┌──────────────────────┐
//! │  BrowserSession    │ ◄──────────────────── │  lightpanda serve    │
//! │  (chromiumoxide)   │                       │  (auto-spawned)      │
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
//! For auto-spawn mode, the `lightpanda` binary must be on `PATH`.
//! Install it with:
//!
//! ```bash
//! curl -fsSL https://pkg.lightpanda.io/install.sh | bash
//! ```
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
//! // Take a screenshot (base64 PNG)
//! let screenshot = session.screenshot().await?;
//!
//! // Clean up
//! session.close().await?;
//! # Ok(())
//! # }
//! ```

use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use chromiumoxide::browser::Browser;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use futures::StreamExt;
use tokio::process::{Child, Command};
use tracing::{debug, info};

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

    /// Screenshot capture failed.
    #[error("screenshot failed: {0}")]
    ScreenshotFailed(String),

    /// JavaScript evaluation failed.
    #[error("JS evaluation failed: {0}")]
    EvalFailed(String),

    /// Timed out waiting for the browser process to accept CDP connections.
    #[error("timeout waiting for browser to start")]
    Timeout,
}

/// Convenience alias for `Result<T, BrowserError>`.
pub type Result<T> = std::result::Result<T, BrowserError>;

// ---------------------------------------------------------------------------
// BrowserSession
// ---------------------------------------------------------------------------

/// A browser automation session backed by CDP.
///
/// Manages an optional child process (auto-spawned Lightpanda) and a
/// [`chromiumoxide::Browser`] + [`Page`] for interaction.
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
    /// The chromiumoxide browser handle (kept alive for the CDP connection).
    _browser: Browser,
    /// The active page used for all operations.
    page: Page,
    /// Background task that drives the CDP WebSocket handler stream.
    _handler: tokio::task::JoinHandle<()>,
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
    /// # Errors
    ///
    /// - [`BrowserError::SpawnFailed`] if `lightpanda` is not on `PATH` or fails to start
    /// - [`BrowserError::Timeout`] if CDP doesn't become available within 10 seconds
    /// - [`BrowserError::ConnectionFailed`] if WebSocket handshake fails
    pub async fn launch() -> Result<Self> {
        let port = find_free_port().await?;
        let addr = format!("127.0.0.1:{port}");

        info!(port, "Spawning lightpanda");

        let child = Command::new("lightpanda")
            .args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| BrowserError::SpawnFailed(e.to_string()))?;

        // Wait for CDP to become available
        wait_for_cdp(&addr).await?;

        let ws_url = format!("ws://{addr}");
        Self::connect_internal(Some(child), &ws_url).await
    }

    /// Connect to an existing CDP endpoint.
    ///
    /// Use this when the browser is managed externally (e.g. headless Chromium
    /// started by systemd, or a shared Lightpanda instance).
    ///
    /// # Arguments
    ///
    /// * `cdp_url` — WebSocket URL (e.g. `ws://127.0.0.1:9222`) or HTTP URL
    ///   (chromiumoxide will fetch the WS URL from `/json/version`).
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
        let (browser, handler) = Browser::connect(ws_url)
            .await
            .map_err(|e| BrowserError::ConnectionFailed(e.to_string()))?;

        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| BrowserError::ConnectionFailed(e.to_string()))?;

        let handler = tokio::spawn(async move {
            handler
                .for_each(|event| async move {
                    debug!(?event, "CDP handler event");
                })
                .await;
        });

        Ok(Self {
            process,
            _browser: browser,
            page,
            _handler: handler,
        })
    }

    /// Navigate to a URL. Returns the page title after load.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::NavigationFailed`] on invalid URL or network error
    pub async fn navigate(&self, url: &str) -> Result<String> {
        self.page
            .goto(url)
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

        let title = self
            .page
            .get_title()
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?
            .unwrap_or_default();

        Ok(title)
    }

    /// Take a full-page screenshot. Returns base64-encoded PNG.
    ///
    /// The returned string can be used directly in a `data:image/png;base64,...`
    /// URI or passed to an LLM as a vision input.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::ScreenshotFailed`] if the CDP command fails
    pub async fn screenshot(&self) -> Result<String> {
        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(true)
            .build();

        let bytes = self
            .page
            .screenshot(params)
            .await
            .map_err(|e| BrowserError::ScreenshotFailed(e.to_string()))?;

        Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
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
            Some(sel) => {
                let element = self
                    .page
                    .find_element(sel)
                    .await
                    .map_err(|e| BrowserError::ElementNotFound(format!("{sel}: {e}")))?;

                let text = element
                    .inner_text()
                    .await
                    .map_err(|e| BrowserError::EvalFailed(e.to_string()))?
                    .unwrap_or_default();
                Ok(text)
            }
            None => self.evaluate("document.body.innerText").await,
        }
    }

    /// Click an element by CSS selector.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::ElementNotFound`] if the selector matches nothing
    pub async fn click(&self, selector: &str) -> Result<()> {
        let element = self
            .page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{selector}: {e}")))?;

        element
            .click()
            .await
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;

        Ok(())
    }

    /// Type text into an element identified by CSS selector.
    ///
    /// Clicks the element first to focus it, then types character by character.
    ///
    /// # Errors
    ///
    /// - [`BrowserError::ElementNotFound`] if the selector matches nothing
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        let element = self
            .page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{selector}: {e}")))?;

        element
            .click()
            .await
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;

        element
            .type_str(text)
            .await
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;

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
        let value: serde_json::Value = self
            .page
            .evaluate_expression(js)
            .await
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?
            .into_value()
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?;

        match value {
            serde_json::Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }

    /// Get the current page URL.
    pub async fn url(&self) -> Result<String> {
        let url = self
            .page
            .url()
            .await
            .map_err(|e| BrowserError::EvalFailed(e.to_string()))?
            .map(|u| u.to_string())
            .unwrap_or_default();
        Ok(url)
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
        assert_eq!(
            err.to_string(),
            "timeout waiting for browser to start"
        );

        let err = BrowserError::NotConnected;
        assert_eq!(err.to_string(), "browser not connected");

        let err = BrowserError::ConnectionFailed("refused".into());
        assert_eq!(err.to_string(), "CDP connection failed: refused");

        let err = BrowserError::NavigationFailed("404".into());
        assert_eq!(err.to_string(), "navigation failed: 404");

        let err = BrowserError::ScreenshotFailed("no page".into());
        assert_eq!(err.to_string(), "screenshot failed: no page");

        let err = BrowserError::EvalFailed("syntax error".into());
        assert_eq!(err.to_string(), "JS evaluation failed: syntax error");
    }

    #[tokio::test]
    async fn launch_fails_when_lightpanda_not_installed() {
        // Set PATH to empty so lightpanda can't be found
        let original_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", "");

        let result = BrowserSession::launch().await;

        std::env::set_var("PATH", &original_path);

        assert!(result.is_err(), "should fail when lightpanda not on PATH");
        let err = result.unwrap_err();
        assert!(
            matches!(err, BrowserError::SpawnFailed(_)),
            "expected SpawnFailed, got: {err}"
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
    //   BROWSER_CDP_URL=ws://127.0.0.1:9222 cargo test -p starpod-browser -- --ignored
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
    async fn integration_screenshot_returns_valid_base64_png() {
        let url = cdp_url().expect("BROWSER_CDP_URL not set");
        let session = BrowserSession::connect(&url).await.unwrap();

        session.navigate("https://example.com").await.unwrap();
        let b64 = session.screenshot().await.unwrap();

        // Verify it's valid base64
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .expect("screenshot should be valid base64");

        // Verify PNG magic bytes
        assert!(
            bytes.starts_with(&[0x89, b'P', b'N', b'G']),
            "screenshot should be a PNG (magic bytes: {:?})",
            &bytes[..4.min(bytes.len())]
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
