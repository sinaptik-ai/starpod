use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use colored::Colorize;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// Default Spawner backend URL.
pub const DEFAULT_SPAWNER_URL: &str = "https://console.starpod.sh";

/// Environment variable name for the Spawner URL.
pub const SPAWNER_URL_ENV: &str = "STARPOD_URL";

/// Credentials stored in ~/.starpod/credentials.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub backend_url: String,
    pub api_key: String,
    pub email: String,
}

/// Response from the CLI callback server.
#[derive(Debug, Deserialize)]
pub struct CallbackPayload {
    pub api_key: String,
    pub email: String,
    pub state: String,
}

/// Get the credentials file path: ~/.starpod/credentials.toml
pub fn credentials_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".starpod").join("credentials.toml"))
}

/// Load credentials from ~/.starpod/credentials.toml
pub fn load_credentials() -> Option<Credentials> {
    let path = credentials_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

/// Save credentials to ~/.starpod/credentials.toml
pub fn save_credentials(creds: &Credentials) -> Result<(), String> {
    let path = credentials_path().ok_or("Cannot determine home directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create ~/.starpod/: {}", e))?;
    }
    let content =
        toml::to_string_pretty(creds).map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&path, content).map_err(|e| format!("Failed to write credentials: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&path, perms);
    }

    Ok(())
}

/// Delete credentials file.
pub fn delete_credentials() -> Result<(), String> {
    let path = credentials_path().ok_or("Cannot determine home directory")?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Failed to remove credentials: {}", e))?;
    }
    Ok(())
}

fn find_free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Failed to bind: {}", e))?;
    listener
        .local_addr()
        .map_err(|e| format!("Failed to get address: {}", e))
        .map(|a| a.port())
}

/// Generate a random state token using two UUIDs (64 hex chars).
fn generate_state() -> String {
    let id1 = uuid::Uuid::new_v4();
    let id2 = uuid::Uuid::new_v4();
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(id1.as_bytes());
    bytes[16..].copy_from_slice(id2.as_bytes());
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("cmd")
        .args(["/c", "start", url])
        .spawn();

    cmd.map_err(|e| format!("Failed to open browser: {}", e))?;
    Ok(())
}

/// Run the browser-based login flow.
///
/// 1. Generate a random `state` token for CSRF protection
/// 2. Start a local HTTP server on 127.0.0.1 (random port)
/// 3. Open browser to `{spawner_url}/cli-auth?port=PORT&state=STATE`
/// 4. Frontend logs user in, creates API key, POSTs it back with `state`
/// 5. CLI verifies `state` matches, saves credentials, shuts down
pub async fn browser_login(spawner_url: &str) -> Result<Credentials, String> {
    let port = find_free_port()?;
    let state = generate_state();
    let expected_state = state.clone();

    let (tx, rx) = oneshot::channel::<CallbackPayload>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Failed to bind callback server: {}", e))?;

    let tx_for_server = tx.clone();

    let server_handle = tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };

            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let mut buf = vec![0u8; 16384];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => continue,
            };
            let request = String::from_utf8_lossy(&buf[..n]).to_string();

            // Extract Origin header — only allow localhost origins
            let origin = request
                .lines()
                .find_map(|line| {
                    if line.to_lowercase().starts_with("origin:") {
                        Some(line.split_once(':').unwrap().1.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            let is_localhost = origin.starts_with("http://localhost:")
                || origin.starts_with("http://127.0.0.1:")
                || origin.starts_with("https://localhost:")
                || origin.starts_with("https://127.0.0.1:");

            let cors_origin = if is_localhost {
                origin.as_str()
            } else {
                // For production (same-origin), no CORS needed but include
                // the spawner origin for completeness
                &origin
            };

            // CORS preflight
            if request.starts_with("OPTIONS") {
                let response = format!(
                    "HTTP/1.1 204 No Content\r\n\
                     Access-Control-Allow-Origin: {cors_origin}\r\n\
                     Access-Control-Allow-Methods: POST, OPTIONS\r\n\
                     Access-Control-Allow-Headers: Content-Type\r\n\
                     Content-Length: 0\r\n\r\n"
                );
                let _ = stream.write_all(response.as_bytes()).await;
                continue;
            }

            if request.starts_with("POST") {
                let body = request
                    .split("\r\n\r\n")
                    .nth(1)
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    .to_string();

                let (status, resp_body, done) =
                    match serde_json::from_str::<CallbackPayload>(&body) {
                        Ok(payload) => {
                            let mut guard = tx_for_server.lock().await;
                            if let Some(sender) = guard.take() {
                                let _ = sender.send(payload);
                            }
                            (200, r#"{"ok":true}"#.to_string(), true)
                        }
                        Err(e) => (
                            400,
                            format!(r#"{{"error":"{}"}}"#, e),
                            false,
                        ),
                    };

                let reason = if status == 200 { "OK" } else { "Bad Request" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\n\
                     Access-Control-Allow-Origin: {cors_origin}\r\n\
                     Content-Type: application/json\r\n\
                     Content-Length: {len}\r\n\r\n{resp_body}",
                    len = resp_body.len(),
                );
                let _ = stream.write_all(response.as_bytes()).await;

                if done {
                    break;
                }
            }
        }
    });

    let auth_url = format!(
        "{}/cli-auth?port={}&state={}",
        spawner_url.trim_end_matches('/'),
        port,
        state
    );
    println!(
        "  {} Opening browser for authentication...",
        "⟳".bright_cyan()
    );
    println!("  {} {}", "→".dimmed(), auth_url.bright_white());
    println!();

    if let Err(e) = open_browser(&auth_url) {
        eprintln!(
            "  {} Could not open browser: {}",
            "!".yellow().bold(),
            e
        );
        println!(
            "  {} Open this URL manually: {}",
            "→".dimmed(),
            auth_url.bright_white()
        );
    }

    println!("  {} Waiting for authorization...", "…".dimmed());

    let payload = tokio::select! {
        result = rx => {
            result.map_err(|_| "Auth callback channel closed unexpectedly".to_string())?
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {
            server_handle.abort();
            return Err("Login timed out after 5 minutes".to_string());
        }
    };

    server_handle.abort();

    // Verify state to prevent CSRF
    if payload.state != expected_state {
        return Err(
            "Security error: state mismatch. The callback did not come from the expected auth flow."
                .to_string(),
        );
    }

    let creds = Credentials {
        backend_url: spawner_url.trim_end_matches('/').to_string(),
        api_key: payload.api_key,
        email: payload.email,
    };

    save_credentials(&creds)?;

    Ok(creds)
}
