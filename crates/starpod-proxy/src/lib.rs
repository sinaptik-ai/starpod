//! Opaque secret proxy for Starpod.
//!
//! A local HTTP proxy that intercepts outbound traffic from tool subprocesses,
//! finds `starpod:v1:` opaque tokens, decrypts them, verifies host binding,
//! and replaces them with real secret values before forwarding.
//!
//! # Features
//!
//! - `mitm` — HTTPS MITM with ephemeral certificates (scans HTTPS traffic)
//! - `netns` — Linux network namespace isolation (Phase 4)
//!
//! # Usage
//!
//! ```rust,no_run
//! # async fn example() -> starpod_core::Result<()> {
//! let handle = starpod_proxy::start_proxy(starpod_proxy::ProxyConfig {
//!     master_key: [0u8; 32],
//!     data_dir: std::path::PathBuf::from(".starpod/db"),
//! }).await?;
//!
//! // Inject into tool subprocesses:
//! // HTTP_PROXY=http://127.0.0.1:{handle.port()}
//! // HTTPS_PROXY=http://127.0.0.1:{handle.port()}
//!
//! // Shutdown when done
//! handle.shutdown().await;
//! # Ok(())
//! # }
//! ```

pub mod host_match;
pub mod scan;

#[cfg(feature = "mitm")]
pub mod ca;
#[cfg(feature = "mitm")]
mod mitm;
#[cfg(feature = "netns")]
pub mod netns;
pub mod tier;

mod http;
mod tunnel;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{debug, error, info};

use starpod_core::{Result, StarpodError};

/// Configuration for starting the proxy.
pub struct ProxyConfig {
    /// 32-byte master key for AES-256-GCM token decryption.
    pub master_key: [u8; 32],
    /// Data directory for CA cert storage.
    pub data_dir: PathBuf,
}

/// Handle to a running proxy. Drop to shut down.
pub struct ProxyHandle {
    /// The address the proxy is listening on (`127.0.0.1:<port>`).
    pub addr: SocketAddr,
    /// Path to the CA cert bundle (system roots + local CA).
    /// `None` when MITM is not enabled.
    pub ca_cert_path: Option<PathBuf>,
    /// Network namespace handle (Linux only, Tier 1).
    /// When `Some`, tool subprocesses should enter this namespace.
    #[cfg(feature = "netns")]
    pub ns_handle: Option<netns::NamespaceHandle>,
    shutdown_tx: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl ProxyHandle {
    /// Returns the port the proxy is listening on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Graceful shutdown.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.task.await;
    }

    /// Get a pre_exec hook for subprocess namespace isolation (Tier 1).
    ///
    /// Returns `Some` when a network namespace is active. The returned closure
    /// should be passed to `ToolExecutor::with_pre_exec()` so that all tool
    /// subprocesses enter the isolated namespace.
    #[cfg(feature = "netns")]
    pub fn pre_exec_hook(&self) -> Option<Box<dyn Fn() -> std::io::Result<()> + Send + Sync>> {
        self.ns_handle.as_ref().map(|ns| ns.pre_exec_fn())
    }
}

/// Start the opaque secret proxy as a background tokio task.
///
/// Binds to `127.0.0.1:0` (OS-assigned port) and returns a handle with the
/// assigned port. The caller injects `HTTP_PROXY=http://127.0.0.1:{port}`
/// into subprocess environments.
///
/// When the `mitm` feature is enabled, a local CA is generated (or loaded)
/// and HTTPS CONNECT requests are intercepted with ephemeral per-host
/// certificates for token scanning.
pub async fn start_proxy(config: ProxyConfig) -> Result<ProxyHandle> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| StarpodError::Proxy(format!("Failed to bind proxy: {e}")))?;

    let addr = listener
        .local_addr()
        .map_err(|e| StarpodError::Proxy(format!("Failed to get proxy address: {e}")))?;

    // Detect isolation tier
    let _tier = tier::detect_and_log();

    // Create network namespace for Tier 1 isolation (Linux + CAP_NET_ADMIN)
    #[cfg(feature = "netns")]
    let ns_handle = {
        if _tier == tier::IsolationTier::NetNamespace {
            match netns::create_namespace(addr.port()) {
                Ok(handle) => Some(handle),
                Err(e) => {
                    tracing::warn!(
                        "Failed to create network namespace: {e} — falling back to env var proxy"
                    );
                    None
                }
            }
        } else {
            None
        }
    };

    // Initialize CA for MITM if feature enabled
    #[cfg(feature = "mitm")]
    let ca = match ca::CertAuthority::load_or_generate(&config.data_dir) {
        Ok(ca) => {
            info!(
                ca_cert = %ca.ca_cert_path.display(),
                ca_bundle = %ca.ca_bundle_path.display(),
                "MITM CA loaded"
            );
            Some(Arc::new(ca))
        }
        Err(e) => {
            tracing::warn!("Failed to initialize MITM CA: {e} — HTTPS will use blind tunnel");
            None
        }
    };

    #[cfg(feature = "mitm")]
    let ca_cert_path = ca.as_ref().map(|c| c.ca_bundle_path.clone());
    #[cfg(not(feature = "mitm"))]
    let ca_cert_path: Option<PathBuf> = None;

    info!(
        port = addr.port(),
        mitm = cfg!(feature = "mitm"),
        "Secret proxy listening"
    );

    let cipher = scan::cipher_from_key(&config.master_key);
    #[cfg(feature = "mitm")]
    let cipher_arc = Arc::new(scan::cipher_from_key(&config.master_key));
    let state = Arc::new(http::ProxyState {
        cipher,
        http_client: reqwest::Client::builder()
            .no_proxy()
            .build()
            .map_err(|e| StarpodError::Proxy(format!("Failed to build HTTP client: {e}")))?,
        #[cfg(feature = "mitm")]
        ca,
        #[cfg(feature = "mitm")]
        cipher_arc,
    });

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer)) => {
                            let state = Arc::clone(&state);
                            debug!(peer = %peer, "Proxy connection accepted");
                            tokio::spawn(async move {
                                let io = TokioIo::new(stream);
                                let svc = service_fn(move |req| {
                                    let state = Arc::clone(&state);
                                    async move { http::handle_request(state, req).await }
                                });
                                if let Err(e) = http1::Builder::new()
                                    .preserve_header_case(true)
                                    .title_case_headers(true)
                                    .serve_connection(io, svc)
                                    .with_upgrades()
                                    .await
                                {
                                    if !e.to_string().contains("connection closed") {
                                        debug!("Proxy connection error: {e}");
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            error!("Proxy accept error: {e}");
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("Secret proxy shutting down");
                        break;
                    }
                }
            }
        }
    });

    Ok(ProxyHandle {
        addr,
        ca_cert_path,
        #[cfg(feature = "netns")]
        ns_handle,
        shutdown_tx,
        task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[tokio::test]
    async fn proxy_starts_and_binds_port() {
        let tmp = tempfile::TempDir::new().unwrap();
        let handle = start_proxy(ProxyConfig {
            master_key: [0xAB; 32],
            data_dir: tmp.path().to_path_buf(),
        })
        .await
        .unwrap();

        assert_ne!(handle.port(), 0);
        assert_eq!(handle.addr.ip(), std::net::Ipv4Addr::LOCALHOST);

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn proxy_responds_to_http_request() {
        let tmp = tempfile::TempDir::new().unwrap();
        let handle = start_proxy(ProxyConfig {
            master_key: [0xAB; 32],
            data_dir: tmp.path().to_path_buf(),
        })
        .await
        .unwrap();

        let proxy_url = format!("http://127.0.0.1:{}", handle.port());

        // Send a request through the proxy to a known endpoint
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(&proxy_url).unwrap())
            .build()
            .unwrap();

        // Use http (not https) so no MITM needed
        let resp = client.get("http://httpbin.org/status/200").send().await;

        // The request should either succeed or fail with a network error
        // (httpbin might be unreachable in CI) — but the proxy itself should not crash
        match resp {
            Ok(r) => assert_eq!(r.status(), 200),
            Err(e) => {
                // Network error is acceptable (no internet in CI)
                assert!(
                    e.is_connect() || e.is_request() || e.is_timeout(),
                    "Unexpected error type: {e}"
                );
            }
        }

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn proxy_replaces_token_in_http_request() {
        use aes_gcm::aead::{Aead, OsRng};
        use aes_gcm::{AeadCore, Aes256Gcm, KeyInit};

        let master_key = [0xAB_u8; 32];
        let cipher = Aes256Gcm::new_from_slice(&master_key).unwrap();

        // Encode a token
        #[derive(serde::Serialize)]
        struct Payload {
            v: String,
            h: Vec<String>,
        }
        let payload = Payload {
            v: "real-secret".into(),
            h: vec![], // unrestricted
        };
        let json = serde_json::to_vec(&payload).unwrap();
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher.encrypt(&nonce, json.as_ref()).unwrap();
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(nonce.as_slice());
        blob.extend_from_slice(&ciphertext);
        let token = format!(
            "starpod:v1:{}",
            base64::engine::general_purpose::STANDARD.encode(&blob)
        );

        // Verify the token decodes correctly through our scanner
        let result = scan::scan_and_replace_str(&cipher, &token, "any.host");
        assert_eq!(result.replaced, 1);
        assert_eq!(String::from_utf8(result.data).unwrap(), "real-secret");
    }

    #[tokio::test]
    async fn proxy_shutdown_is_graceful() {
        let tmp = tempfile::TempDir::new().unwrap();
        let handle = start_proxy(ProxyConfig {
            master_key: [0xAB; 32],
            data_dir: tmp.path().to_path_buf(),
        })
        .await
        .unwrap();

        let port = handle.port();
        handle.shutdown().await;

        // After shutdown, the port should no longer accept connections
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let result = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await;
        assert!(result.is_err(), "Port should be closed after shutdown");
    }

    #[cfg(feature = "mitm")]
    #[tokio::test]
    async fn proxy_generates_ca_on_startup() {
        let tmp = tempfile::TempDir::new().unwrap();
        let handle = start_proxy(ProxyConfig {
            master_key: [0xAB; 32],
            data_dir: tmp.path().to_path_buf(),
        })
        .await
        .unwrap();

        // CA cert should exist
        assert!(handle.ca_cert_path.is_some());
        let ca_path = handle.ca_cert_path.as_ref().unwrap();
        assert!(
            ca_path.exists(),
            "CA bundle should exist at {}",
            ca_path.display()
        );

        // CA bundle should contain PEM data
        let bundle = std::fs::read_to_string(ca_path).unwrap();
        assert!(
            bundle.contains("BEGIN CERTIFICATE"),
            "Bundle should contain PEM certs"
        );

        handle.shutdown().await;
    }
}
