//! Filesystem browser API endpoints.
//!
//! Provides read/write access to the agent's home directory sandbox (`home/`),
//! gated by the per-user `filesystem_enabled` flag in the auth database.
//!
//! ## Security model
//!
//! - **Authentication**: all endpoints call [`authenticate_request`] first.
//! - **Feature gate**: the authenticated user must have `filesystem_enabled = true`.
//!   When auth is disabled (no users in the database), access is allowed by default.
//! - **Sandbox**: paths are validated by [`validate_sandbox_path`] which rejects
//!   absolute paths, `..` traversal, and any access to `.starpod/`.
//!   Existing paths are canonicalized to prevent symlink escapes.
//!
//! ## Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/api/files?path=.` | List directory contents |
//! | `GET` | `/api/files/read?path=file.txt` | Read file content |
//! | `PUT` | `/api/files/write` | Write file content |
//! | `DELETE` | `/api/files?path=file.txt` | Delete file or directory |
//! | `POST` | `/api/files/mkdir` | Create directory (recursive) |

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::routes::{authenticate_request, ErrorResponse};
use crate::AppState;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (status, Json(ErrorResponse { error: msg.into() }))
}

fn internal(msg: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    err(StatusCode::INTERNAL_SERVER_ERROR, msg.to_string())
}

/// Build the router for all `/api/files*` endpoints.
pub fn files_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/files", get(list_handler).delete(delete_handler))
        .route("/api/files/read", get(read_handler))
        .route("/api/files/write", axum::routing::put(write_handler))
        .route("/api/files/mkdir", post(mkdir_handler))
}

// ── Auth helper ──────────────────────────────────────────────────────────

/// Authenticate the request and verify the user has filesystem access.
///
/// Returns `Ok(())` if the user is authorized:
/// - The user exists and has `filesystem_enabled == true`, or
/// - Auth is disabled (no users in the database — fresh install).
///
/// Returns `403 Forbidden` if the user exists but `filesystem_enabled` is false.
/// Propagates `401 Unauthorized` from [`authenticate_request`] for missing/invalid keys.
async fn require_filesystem_access(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let user = authenticate_request(state, headers).await?;
    match user {
        Some(u) if u.filesystem_enabled => Ok(()),
        Some(_) => Err(err(StatusCode::FORBIDDEN, "Filesystem access is not enabled for this user")),
        None => Ok(()), // Auth disabled — allow access
    }
}

// ── Sandbox validation ───────────────────────────────────────────────────

/// Validate and resolve a relative path within the home directory sandbox.
///
/// Rejects paths that:
/// - Start with `/` or `\` (absolute paths)
/// - Contain `..` components (directory traversal)
/// - Start with `.starpod` (internal config directory)
///
/// For existing paths, the resolved path is canonicalized and checked to
/// ensure it remains within `home_dir` (prevents symlink escapes).
fn validate_sandbox_path(relative: &str, home_dir: &Path) -> Result<PathBuf, String> {
    if relative.starts_with('/') || relative.starts_with('\\') {
        return Err("Absolute paths are not allowed".into());
    }

    for component in Path::new(relative).components() {
        if matches!(component, Component::ParentDir) {
            return Err("Path traversal (..) is not allowed".into());
        }
    }

    let normalized = relative.replace('\\', "/");
    if normalized == ".starpod" || normalized.starts_with(".starpod/") {
        return Err("Cannot access .starpod/ directory".into());
    }

    let resolved = home_dir.join(relative);

    if resolved.exists() {
        let canonical = resolved.canonicalize().map_err(|e| format!("Failed to resolve path: {}", e))?;
        let root_canonical = home_dir.canonicalize().map_err(|e| format!("Failed to resolve root: {}", e))?;
        if !canonical.starts_with(&root_canonical) {
            return Err("Path resolves outside the sandbox".into());
        }
    }

    Ok(resolved)
}

// ── Types ────────────────────────────────────────────────────────────────

/// Query parameter for endpoints that accept a `path` argument.
#[derive(Debug, Deserialize)]
struct PathQuery {
    /// Relative path within the sandbox. Defaults to `"."` (root).
    #[serde(default = "default_path")]
    path: String,
}

fn default_path() -> String {
    ".".into()
}

/// A single file or directory entry returned by the list endpoint.
#[derive(Debug, Serialize)]
struct FileEntry {
    /// Entry name. Directories have a trailing `/`.
    name: String,
    /// `"file"` or `"directory"`.
    #[serde(rename = "type")]
    entry_type: String,
    /// File size in bytes (0 for directories).
    size: u64,
}

/// Request body for `PUT /api/files/write`.
#[derive(Debug, Deserialize)]
struct WriteRequest {
    /// Relative path to write to.
    path: String,
    /// File content (UTF-8 text).
    content: String,
}

/// Request body for `POST /api/files/mkdir`.
#[derive(Debug, Deserialize)]
struct MkdirRequest {
    /// Relative path of the directory to create.
    path: String,
}

/// Response body for `GET /api/files/read`.
#[derive(Debug, Serialize)]
struct FileContent {
    /// The requested path (echoed back).
    path: String,
    /// File content as UTF-8 text.
    content: String,
    /// File size in bytes.
    size: u64,
}

// ── Handlers ─────────────────────────────────────────────────────────────

/// List directory contents.
///
/// `GET /api/files?path=.`
///
/// Returns a sorted array of [`FileEntry`] objects. Directories are listed
/// with a trailing `/` in their name. The `.starpod/` directory is always
/// hidden from listings.
async fn list_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<PathQuery>,
) -> ApiResult<Vec<FileEntry>> {
    require_filesystem_access(&state, &headers).await?;

    let home = &state.paths.home_dir;
    let resolved = if params.path == "." {
        home.clone()
    } else {
        validate_sandbox_path(&params.path, home)
            .map_err(|e| err(StatusCode::BAD_REQUEST, e))?
    };

    if !resolved.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, format!("Not a directory: {}", params.path)));
    }

    let entries = std::fs::read_dir(&resolved).map_err(|e| internal(e))?;
    let mut items: Vec<FileEntry> = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".starpod" {
            continue;
        }
        let meta = entry.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        items.push(FileEntry {
            name: if is_dir { format!("{}/", name) } else { name },
            entry_type: if is_dir { "directory".into() } else { "file".into() },
            size,
        });
    }

    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(items))
}

/// Read file content as UTF-8 text.
///
/// `GET /api/files/read?path=file.txt`
///
/// Returns 404 if the file doesn't exist, 400 if the path points to a
/// directory, or 422 if the file is binary (not valid UTF-8).
async fn read_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<PathQuery>,
) -> ApiResult<FileContent> {
    require_filesystem_access(&state, &headers).await?;

    let home = &state.paths.home_dir;
    let resolved = validate_sandbox_path(&params.path, home)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

    if !resolved.exists() {
        return Err(err(StatusCode::NOT_FOUND, format!("File not found: {}", params.path)));
    }
    if resolved.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "Cannot read a directory — use list instead"));
    }

    let content = std::fs::read_to_string(&resolved)
        .map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, format!("Cannot read file (binary?): {}", e)))?;
    let size = resolved.metadata().map(|m| m.len()).unwrap_or(0);

    Ok(Json(FileContent {
        path: params.path,
        content,
        size,
    }))
}

/// Write file content. Creates parent directories as needed.
///
/// `PUT /api/files/write`
///
/// Request body: `{ "path": "notes.txt", "content": "hello" }`
async fn write_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<WriteRequest>,
) -> ApiResult<serde_json::Value> {
    require_filesystem_access(&state, &headers).await?;

    let home = &state.paths.home_dir;
    let resolved = validate_sandbox_path(&req.path, home)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(|e| internal(e))?;
    }

    std::fs::write(&resolved, &req.content).map_err(|e| internal(e))?;

    Ok(Json(serde_json::json!({"status": "ok"})))
}

/// Delete a file or directory (recursive for directories).
///
/// `DELETE /api/files?path=file.txt`
///
/// Returns 404 if the path doesn't exist.
async fn delete_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<PathQuery>,
) -> ApiResult<serde_json::Value> {
    require_filesystem_access(&state, &headers).await?;

    let home = &state.paths.home_dir;
    let resolved = validate_sandbox_path(&params.path, home)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

    if !resolved.exists() {
        return Err(err(StatusCode::NOT_FOUND, format!("File not found: {}", params.path)));
    }

    if resolved.is_dir() {
        std::fs::remove_dir_all(&resolved).map_err(|e| internal(e))?;
    } else {
        std::fs::remove_file(&resolved).map_err(|e| internal(e))?;
    }

    Ok(Json(serde_json::json!({"status": "ok"})))
}

/// Create a directory (and all parent directories).
///
/// `POST /api/files/mkdir`
///
/// Request body: `{ "path": "reports/2026" }`
async fn mkdir_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MkdirRequest>,
) -> ApiResult<serde_json::Value> {
    require_filesystem_access(&state, &headers).await?;

    let home = &state.paths.home_dir;
    let resolved = validate_sandbox_path(&req.path, home)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

    std::fs::create_dir_all(&resolved).map_err(|e| internal(e))?;

    Ok(Json(serde_json::json!({"status": "ok"})))
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::RwLock;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use starpod_agent::StarpodAgent;
    use starpod_auth::{AuthStore, RateLimiter};
    use starpod_core::{ResolvedPaths, StarpodConfig, Mode};
    use crate::{build_router, GatewayEvent};

    /// Build a test AppState with a real auth store and temp home directory.
    async fn test_app_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        let config_dir = starpod_dir.join("config");
        let db_dir = starpod_dir.join("db");
        let users_dir = starpod_dir.join("users");
        let skills_dir = starpod_dir.join("skills");
        let agent_toml = config_dir.join("agent.toml");
        let home_dir = tmp.path().join("home");

        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::create_dir_all(&users_dir).unwrap();
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&home_dir).unwrap();
        std::fs::write(&agent_toml, "models = [\"anthropic/test\"]\nagent_name = \"Test\"\n").unwrap();

        let config = StarpodConfig {
            db_dir: db_dir.clone(),
            db_path: Some(db_dir.join("memory.db")),
            project_root: tmp.path().to_path_buf(),
            models: vec!["anthropic/test".into()],
            agent_name: "Test".into(),
            ..StarpodConfig::default()
        };

        let agent = StarpodAgent::new(config.clone()).await.unwrap();
        let (events_tx, _) = tokio::sync::broadcast::channel::<GatewayEvent>(16);

        let paths = ResolvedPaths {
            mode: Mode::SingleAgent { starpod_dir: starpod_dir.clone() },
            agent_toml,
            agent_home: starpod_dir,
            config_dir,
            db_dir: db_dir.clone(),
            skills_dir,
            project_root: tmp.path().to_path_buf(),
            instance_root: tmp.path().to_path_buf(),
            home_dir,
            users_dir,
            env_file: None,
        };

        let auth = Arc::new(AuthStore::new(&db_dir.join("users.db")).await.unwrap());
        let rate_limiter = Arc::new(RateLimiter::new(0, Duration::from_secs(60)));

        let state = Arc::new(AppState {
            agent: Arc::new(agent),
            auth,
            rate_limiter,
            config: RwLock::new(config),
            paths,
            model_registry: Arc::new(agent_sdk::models::ModelRegistry::with_defaults()),
            events_tx,
        });

        (tmp, state)
    }

    /// Create a user with filesystem access and return their API key.
    async fn create_fs_user(state: &AppState) -> String {
        let user = state.auth.create_user(None, Some("TestUser"), starpod_auth::Role::User).await.unwrap();
        state.auth.update_user(&user.id, None, None, None, Some(true)).await.unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();
        key.key
    }

    /// Create a user without filesystem access and return their API key.
    async fn create_no_fs_user(state: &AppState) -> String {
        let user = state.auth.create_user(None, Some("NoFsUser"), starpod_auth::Role::User).await.unwrap();
        let key = state.auth.create_api_key(&user.id, None).await.unwrap();
        key.key
    }

    /// Make a GET request with an API key.
    async fn get_with_key(state: Arc<AppState>, path: &str, key: &str) -> (StatusCode, serde_json::Value) {
        let app = build_router(state);
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("x-api-key", key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, json)
    }

    /// Make a PUT request with JSON body and API key.
    async fn put_with_key(state: Arc<AppState>, path: &str, key: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let app = build_router(state);
        let req = Request::builder()
            .method("PUT")
            .uri(path)
            .header("content-type", "application/json")
            .header("x-api-key", key)
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, json)
    }

    /// Make a POST request with JSON body and API key.
    async fn post_with_key(state: Arc<AppState>, path: &str, key: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let app = build_router(state);
        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("x-api-key", key)
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, json)
    }

    /// Make a DELETE request with API key.
    async fn delete_with_key(state: Arc<AppState>, path: &str, key: &str) -> (StatusCode, serde_json::Value) {
        let app = build_router(state);
        let req = Request::builder()
            .method("DELETE")
            .uri(path)
            .header("x-api-key", key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, json)
    }

    // ── Sandbox validation unit tests ────────────────────────────────────

    #[test]
    fn sandbox_allows_simple_relative_paths() {
        let home = PathBuf::from("/tmp/test_home");
        assert!(validate_sandbox_path("notes.txt", &home).is_ok());
        assert!(validate_sandbox_path("docs/readme.md", &home).is_ok());
        assert!(validate_sandbox_path("a/b/c/d.txt", &home).is_ok());
    }

    #[test]
    fn sandbox_rejects_absolute_paths() {
        let home = PathBuf::from("/tmp/test_home");
        assert!(validate_sandbox_path("/etc/passwd", &home).is_err());
        assert!(validate_sandbox_path("\\Windows\\System32", &home).is_err());
    }

    #[test]
    fn sandbox_rejects_traversal() {
        let home = PathBuf::from("/tmp/test_home");
        assert!(validate_sandbox_path("../etc/passwd", &home).is_err());
        assert!(validate_sandbox_path("foo/../../bar", &home).is_err());
        assert!(validate_sandbox_path("..", &home).is_err());
    }

    #[test]
    fn sandbox_rejects_starpod_dir() {
        let home = PathBuf::from("/tmp/test_home");
        assert!(validate_sandbox_path(".starpod", &home).is_err());
        assert!(validate_sandbox_path(".starpod/config/agent.toml", &home).is_err());
        assert!(validate_sandbox_path(".starpod/db/memory.db", &home).is_err());
    }

    #[test]
    fn sandbox_allows_dotfiles_other_than_starpod() {
        let home = PathBuf::from("/tmp/test_home");
        assert!(validate_sandbox_path(".gitignore", &home).is_ok());
        assert!(validate_sandbox_path(".env.example", &home).is_ok());
    }

    // ── Auth & permission gating tests ───────────────────────────────────

    #[tokio::test]
    async fn pre_bootstrap_allows_file_access() {
        let (_tmp, state) = test_app_state().await;
        // No users — auth disabled, should allow access
        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/files")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn user_without_filesystem_gets_403() {
        let (_tmp, state) = test_app_state().await;
        let key = create_no_fs_user(&state).await;

        let (status, json) = get_with_key(Arc::clone(&state), "/api/files", &key).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(json["error"].as_str().unwrap().contains("not enabled"));
    }

    #[tokio::test]
    async fn user_with_filesystem_gets_200() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, _json) = get_with_key(Arc::clone(&state), "/api/files", &key).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_api_key_gets_401() {
        let (_tmp, state) = test_app_state().await;
        // Create a user so auth is enforced
        state.auth.create_user(None, None, starpod_auth::Role::Admin).await.unwrap();

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/files")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── List endpoint tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn list_empty_home_directory() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, json) = get_with_key(Arc::clone(&state), "/api/files", &key).await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_shows_files_and_directories() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        // Create some files and directories
        let home = &state.paths.home_dir;
        std::fs::write(home.join("hello.txt"), "world").unwrap();
        std::fs::create_dir_all(home.join("docs")).unwrap();

        let (status, json) = get_with_key(Arc::clone(&state), "/api/files", &key).await;
        assert_eq!(status, StatusCode::OK);

        let entries = json.as_array().unwrap();
        assert_eq!(entries.len(), 2);

        // Should be sorted: docs/ before hello.txt
        assert_eq!(entries[0]["name"], "docs/");
        assert_eq!(entries[0]["type"], "directory");
        assert_eq!(entries[1]["name"], "hello.txt");
        assert_eq!(entries[1]["type"], "file");
        assert_eq!(entries[1]["size"], 5); // "world" = 5 bytes
    }

    #[tokio::test]
    async fn list_hides_starpod_directory() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        // Create .starpod inside home
        let home = &state.paths.home_dir;
        std::fs::create_dir_all(home.join(".starpod")).unwrap();
        std::fs::write(home.join("visible.txt"), "yes").unwrap();

        let (status, json) = get_with_key(Arc::clone(&state), "/api/files", &key).await;
        assert_eq!(status, StatusCode::OK);

        let entries = json.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "visible.txt");
    }

    #[tokio::test]
    async fn list_subdirectory() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let home = &state.paths.home_dir;
        std::fs::create_dir_all(home.join("docs")).unwrap();
        std::fs::write(home.join("docs/readme.md"), "# Hello").unwrap();

        let (status, json) = get_with_key(
            Arc::clone(&state),
            "/api/files?path=docs",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);

        let entries = json.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "readme.md");
    }

    #[tokio::test]
    async fn list_rejects_traversal() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, json) = get_with_key(
            Arc::clone(&state),
            "/api/files?path=../etc",
            &key,
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("traversal"));
    }

    // ── Read endpoint tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn read_existing_file() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        std::fs::write(state.paths.home_dir.join("test.txt"), "file content here").unwrap();

        let (status, json) = get_with_key(
            Arc::clone(&state),
            "/api/files/read?path=test.txt",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["path"], "test.txt");
        assert_eq!(json["content"], "file content here");
        assert_eq!(json["size"], 17);
    }

    #[tokio::test]
    async fn read_nonexistent_file_returns_404() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, _) = get_with_key(
            Arc::clone(&state),
            "/api/files/read?path=nope.txt",
            &key,
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn read_directory_returns_400() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        std::fs::create_dir_all(state.paths.home_dir.join("mydir")).unwrap();

        let (status, json) = get_with_key(
            Arc::clone(&state),
            "/api/files/read?path=mydir",
            &key,
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("directory"));
    }

    #[tokio::test]
    async fn read_nested_file() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let nested = state.paths.home_dir.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.md"), "# Deep").unwrap();

        let (status, json) = get_with_key(
            Arc::clone(&state),
            "/api/files/read?path=a/b/deep.md",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["content"], "# Deep");
    }

    // ── Write endpoint tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn write_creates_file() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, _) = put_with_key(
            Arc::clone(&state),
            "/api/files/write",
            &key,
            serde_json::json!({"path": "new.txt", "content": "hello world"}),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // Verify file was written
        let content = std::fs::read_to_string(state.paths.home_dir.join("new.txt")).unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn write_creates_parent_directories() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, _) = put_with_key(
            Arc::clone(&state),
            "/api/files/write",
            &key,
            serde_json::json!({"path": "deep/nested/file.txt", "content": "nested!"}),
        ).await;
        assert_eq!(status, StatusCode::OK);

        let content = std::fs::read_to_string(state.paths.home_dir.join("deep/nested/file.txt")).unwrap();
        assert_eq!(content, "nested!");
    }

    #[tokio::test]
    async fn write_overwrites_existing_file() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        std::fs::write(state.paths.home_dir.join("exist.txt"), "old").unwrap();

        let (status, _) = put_with_key(
            Arc::clone(&state),
            "/api/files/write",
            &key,
            serde_json::json!({"path": "exist.txt", "content": "new"}),
        ).await;
        assert_eq!(status, StatusCode::OK);

        let content = std::fs::read_to_string(state.paths.home_dir.join("exist.txt")).unwrap();
        assert_eq!(content, "new");
    }

    #[tokio::test]
    async fn write_rejects_starpod_path() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, json) = put_with_key(
            Arc::clone(&state),
            "/api/files/write",
            &key,
            serde_json::json!({"path": ".starpod/config/agent.toml", "content": "evil"}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains(".starpod"));
    }

    // ── Delete endpoint tests ────────────────────────────────────────────

    #[tokio::test]
    async fn delete_file() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let path = state.paths.home_dir.join("delete_me.txt");
        std::fs::write(&path, "bye").unwrap();
        assert!(path.exists());

        let (status, _) = delete_with_key(
            Arc::clone(&state),
            "/api/files?path=delete_me.txt",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn delete_directory_recursively() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let dir = state.paths.home_dir.join("rm_dir");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/file.txt"), "nested").unwrap();

        let (status, _) = delete_with_key(
            Arc::clone(&state),
            "/api/files?path=rm_dir",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_404() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, _) = delete_with_key(
            Arc::clone(&state),
            "/api/files?path=ghost.txt",
            &key,
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // ── Mkdir endpoint tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn mkdir_creates_directory() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, _) = post_with_key(
            Arc::clone(&state),
            "/api/files/mkdir",
            &key,
            serde_json::json!({"path": "new_dir"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert!(state.paths.home_dir.join("new_dir").is_dir());
    }

    #[tokio::test]
    async fn mkdir_creates_nested_directories() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, _) = post_with_key(
            Arc::clone(&state),
            "/api/files/mkdir",
            &key,
            serde_json::json!({"path": "a/b/c/d"}),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert!(state.paths.home_dir.join("a/b/c/d").is_dir());
    }

    #[tokio::test]
    async fn mkdir_rejects_traversal() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        let (status, _) = post_with_key(
            Arc::clone(&state),
            "/api/files/mkdir",
            &key,
            serde_json::json!({"path": "../../escape"}),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ── End-to-end workflow test ─────────────────────────────────────────

    #[tokio::test]
    async fn full_file_lifecycle() {
        let (_tmp, state) = test_app_state().await;
        let key = create_fs_user(&state).await;

        // 1. Create a directory
        let (status, _) = post_with_key(
            Arc::clone(&state),
            "/api/files/mkdir",
            &key,
            serde_json::json!({"path": "project"}),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // 2. Write a file into it
        let (status, _) = put_with_key(
            Arc::clone(&state),
            "/api/files/write",
            &key,
            serde_json::json!({"path": "project/notes.md", "content": "# Notes\n\nFirst entry."}),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // 3. List the directory
        let (status, json) = get_with_key(
            Arc::clone(&state),
            "/api/files?path=project",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);
        let entries = json.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "notes.md");

        // 4. Read the file back
        let (status, json) = get_with_key(
            Arc::clone(&state),
            "/api/files/read?path=project/notes.md",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["content"], "# Notes\n\nFirst entry.");

        // 5. Update the file
        let (status, _) = put_with_key(
            Arc::clone(&state),
            "/api/files/write",
            &key,
            serde_json::json!({"path": "project/notes.md", "content": "# Notes\n\nUpdated."}),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // 6. Verify update
        let (status, json) = get_with_key(
            Arc::clone(&state),
            "/api/files/read?path=project/notes.md",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["content"], "# Notes\n\nUpdated.");

        // 7. Delete the file
        let (status, _) = delete_with_key(
            Arc::clone(&state),
            "/api/files?path=project/notes.md",
            &key,
        ).await;
        assert_eq!(status, StatusCode::OK);

        // 8. Verify deletion
        let (status, _) = get_with_key(
            Arc::clone(&state),
            "/api/files/read?path=project/notes.md",
            &key,
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
