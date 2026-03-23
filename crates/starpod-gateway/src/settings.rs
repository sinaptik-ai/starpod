//! Settings API — read and write agent configuration, markdown files, user data, and skills.
//!
//! All endpoints live under `/api/settings/*` and are protected by the same
//! `X-API-Key` header check used by the rest of the gateway.
//!
//! ## Config write flow
//!
//! 1. Handler reads `agent.toml` from disk as `toml::Value` (preserving structure).
//! 2. Patches the changed fields.
//! 3. Writes back via `toml::to_string_pretty`.
//! 4. The existing file watcher in [`crate::start_config_watcher`] detects the change
//!    (2-second debounce) and calls `reload_agent_config` → `StarpodAgent::reload_config`.
//!
//! ## User CRUD
//!
//! Users are stored as directories in `.starpod/users/<id>/`. Each directory
//! contains `USER.md`, `MEMORY.md`, and a `memory/` subdirectory for daily logs.
//! User IDs are validated to be alphanumeric with hyphens/underscores, max 32
//! characters, with no path traversal.
//!
//! ## Skill CRUD
//!
//! Skills are managed via [`starpod_skills::SkillStore`], which reads/writes
//! `SKILL.md` files in `.starpod/skills/<name>/`. The store is instantiated
//! per-request (cheap — just a `create_dir_all` + struct init).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use starpod_auth::Role;
use starpod_core::{FrontendConfig, FollowupMode, ReasoningEffort, reload_agent_config};

use starpod_core::ResolvedPaths;

use crate::routes::{authenticate_request, ErrorResponse};
use crate::AppState;

// ── Response helpers ────────────────────────────────────────────────────

fn ok_json() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (status, Json(ErrorResponse { error: msg.into() }))
}

fn internal(msg: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    err(StatusCode::INTERNAL_SERVER_ERROR, msg.to_string())
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    err(StatusCode::BAD_REQUEST, msg)
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

// ── Request/Response types ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct GeneralSettings {
    provider: String,
    model: String,
    max_turns: u32,
    max_tokens: u32,
    agent_name: String,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    compaction_model: Option<String>,
    #[serde(default)]
    compaction_provider: Option<String>,
    #[serde(default)]
    followup_mode: FollowupMode,
    server_addr: String,
    #[serde(default)]
    self_improve: bool,
}

#[derive(Debug, Serialize)]
struct ModelsResponse {
    models: std::collections::HashMap<String, Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MemorySettings {
    half_life_days: f64,
    mmr_lambda: f64,
    vector_search: bool,
    chunk_size: usize,
    chunk_overlap: usize,
    export_sessions: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CronSettings {
    default_max_retries: u32,
    default_timeout_secs: u64,
    max_concurrent_runs: usize,
    heartbeat_interval_minutes: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct HeartbeatSettings {
    enabled: bool,
    interval_minutes: u32,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FrontendSettings {
    greeting: Option<String>,
    prompts: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BrowserSettings {
    enabled: bool,
    cdp_url: Option<String>,
    startup_timeout_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CompactionSettings {
    context_budget: u64,
    summary_max_tokens: u32,
    min_keep_messages: usize,
    max_tool_result_bytes: usize,
    prune_threshold_pct: u8,
    prune_tool_result_max_chars: usize,
    memory_flush: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct InternetSettings {
    enabled: bool,
    timeout_secs: u64,
    max_fetch_bytes: usize,
    max_text_chars: usize,
    #[serde(default)]
    brave_api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FileContent {
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UserInfo {
    id: String,
    has_user_md: bool,
    has_memory_md: bool,
    daily_log_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct UserDetail {
    id: String,
    user_md: String,
    memory_md: String,
    daily_log_count: usize,
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    id: String,
}

#[derive(Debug, Serialize)]
struct SkillInfo {
    name: String,
    description: String,
    version: Option<String>,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct SkillDetail {
    name: String,
    description: String,
    version: Option<String>,
    body: String,
    raw_content: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct CreateSkillRequest {
    name: String,
    description: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct UpdateSkillRequest {
    description: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct GenerateSkillRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(Debug, Serialize)]
struct GenerateSkillResponse {
    description: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct CreateAuthUserRequest {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default = "default_role")]
    role: Role,
}

fn default_role() -> Role {
    Role::User
}

#[derive(Debug, Deserialize)]
struct UpdateAuthUserRequest {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    role: Option<Role>,
}

#[derive(Debug, Deserialize)]
struct CreateApiKeyRequest {
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChannelsSettings {
    telegram: TelegramChannelSettings,
}

#[derive(Debug, Serialize, Deserialize)]
struct TelegramChannelSettings {
    enabled: bool,
    #[serde(default)]
    gap_minutes: Option<i64>,
    #[serde(default)]
    stream_mode: Option<String>,
    /// Bot token — read from / written to `.env` as `TELEGRAM_BOT_TOKEN`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bot_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LinkTelegramRequest {
    telegram_id: i64,
    #[serde(default)]
    username: Option<String>,
}

// ── Routes ──────────────────────────────────────────────────────────────

/// Build the settings sub-router with all `/api/settings/*` routes.
pub fn settings_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/settings/general", get(get_general).put(put_general))
        .route("/api/settings/models", get(get_models))
        .route("/api/settings/memory", get(get_memory).put(put_memory))
        .route("/api/settings/cron", get(get_cron).put(put_cron))
        .route("/api/settings/frontend", get(get_frontend).put(put_frontend))
        .route("/api/settings/browser", get(get_browser).put(put_browser))
        .route("/api/settings/heartbeat", get(get_heartbeat).put(put_heartbeat))
        .route("/api/settings/files/{name}", get(get_file).put(put_file))
        .route("/api/settings/users", get(list_users).post(create_user))
        .route(
            "/api/settings/users/{id}",
            get(get_user).put(update_user).delete(delete_user),
        )
        .route("/api/settings/skills", get(list_skills).post(create_skill))
        .route("/api/settings/skills/generate", axum::routing::post(generate_skill))
        .route(
            "/api/settings/skills/{name}",
            get(get_skill).put(update_skill).delete(delete_skill),
        )
        // Auth user management
        .route("/api/settings/auth/users", get(list_auth_users).post(create_auth_user))
        .route(
            "/api/settings/auth/users/{id}",
            get(get_auth_user).put(update_auth_user),
        )
        .route("/api/settings/auth/users/{id}/deactivate", axum::routing::post(deactivate_auth_user))
        .route("/api/settings/auth/users/{id}/activate", axum::routing::post(activate_auth_user))
        .route(
            "/api/settings/auth/users/{id}/api-keys",
            get(list_auth_api_keys).post(create_auth_api_key),
        )
        .route("/api/settings/auth/api-keys/{id}/revoke", axum::routing::post(revoke_auth_api_key))
        // Compaction
        .route("/api/settings/compaction", get(get_compaction).put(put_compaction))
        // Internet
        .route("/api/settings/internet", get(get_internet).put(put_internet))
        // Channels
        .route("/api/settings/channels", get(get_channels).put(put_channels))
        // Costs
        .route("/api/settings/costs", get(get_costs))
        // Telegram linking per user
        .route(
            "/api/settings/auth/users/{id}/telegram",
            get(get_user_telegram).put(put_user_telegram).delete(delete_user_telegram),
        )
}

// ── General ─────────────────────────────────────────────────────────────

async fn get_general(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<GeneralSettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    let cfg = state.config.read().unwrap();
    Ok(Json(GeneralSettings {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
        max_turns: cfg.max_turns,
        max_tokens: cfg.max_tokens,
        agent_name: cfg.agent_name.clone(),
        timezone: cfg.timezone.clone(),
        reasoning_effort: cfg.reasoning_effort,
        compaction_model: cfg.compaction_model.clone(),
        compaction_provider: cfg.compaction_provider.clone(),
        followup_mode: cfg.followup_mode,
        server_addr: cfg.server_addr.clone(),
        self_improve: cfg.self_improve,
    }))
}

async fn put_general(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<GeneralSettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    if settings.model.is_empty() {
        return Err(bad_request("model cannot be empty"));
    }
    if settings.provider.is_empty() {
        return Err(bad_request("provider cannot be empty"));
    }
    if settings.max_turns == 0 {
        return Err(bad_request("max_turns must be > 0"));
    }

    let mut doc = read_agent_toml(&state)?;
    let table = doc.as_table_mut().ok_or_else(|| internal("agent.toml is not a table"))?;

    table.insert("provider".into(), toml::Value::String(settings.provider));
    table.insert("model".into(), toml::Value::String(settings.model));
    table.insert("max_turns".into(), toml::Value::Integer(settings.max_turns as i64));
    table.insert("max_tokens".into(), toml::Value::Integer(settings.max_tokens as i64));
    table.insert("agent_name".into(), toml::Value::String(settings.agent_name));
    table.insert("server_addr".into(), toml::Value::String(settings.server_addr));

    set_or_remove_string(table, "timezone", settings.timezone);
    set_or_remove_string(table, "compaction_model", settings.compaction_model);
    set_or_remove_string(table, "compaction_provider", settings.compaction_provider);

    match settings.reasoning_effort {
        Some(re) => {
            let val = match re {
                ReasoningEffort::Low => "low",
                ReasoningEffort::Medium => "medium",
                ReasoningEffort::High => "high",
            };
            table.insert("reasoning_effort".into(), toml::Value::String(val.into()));
        }
        None => { table.remove("reasoning_effort"); }
    }

    let fm = match settings.followup_mode {
        FollowupMode::Inject => "inject",
        FollowupMode::Queue => "queue",
    };
    table.insert("followup_mode".into(), toml::Value::String(fm.into()));

    table.insert("self_improve".into(), toml::Value::Boolean(settings.self_improve));

    write_agent_toml(&state, &doc)?;
    Ok(ok_json())
}

// ── Models ──────────────────────────────────────────────────────────────

async fn get_models(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<ModelsResponse> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    Ok(Json(ModelsResponse { models: state.model_registry.models_by_provider() }))
}

// ── Memory ──────────────────────────────────────────────────────────────

async fn get_memory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<MemorySettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    let cfg = state.config.read().unwrap();
    Ok(Json(MemorySettings {
        half_life_days: cfg.memory.half_life_days,
        mmr_lambda: cfg.memory.mmr_lambda,
        vector_search: cfg.memory.vector_search,
        chunk_size: cfg.memory.chunk_size,
        chunk_overlap: cfg.memory.chunk_overlap,
        export_sessions: cfg.memory.export_sessions,
    }))
}

async fn put_memory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<MemorySettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    let mut doc = read_agent_toml(&state)?;
    let table = doc.as_table_mut().ok_or_else(|| internal("agent.toml is not a table"))?;

    let mem = table
        .entry("memory")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| internal("[memory] is not a table"))?;

    mem.insert("half_life_days".into(), toml::Value::Float(settings.half_life_days));
    mem.insert("mmr_lambda".into(), toml::Value::Float(settings.mmr_lambda));
    mem.insert("vector_search".into(), toml::Value::Boolean(settings.vector_search));
    mem.insert("chunk_size".into(), toml::Value::Integer(settings.chunk_size as i64));
    mem.insert("chunk_overlap".into(), toml::Value::Integer(settings.chunk_overlap as i64));
    mem.insert("export_sessions".into(), toml::Value::Boolean(settings.export_sessions));

    write_agent_toml(&state, &doc)?;
    Ok(ok_json())
}

// ── Cron ────────────────────────────────────────────────────────────────

async fn get_cron(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<CronSettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    let cfg = state.config.read().unwrap();
    Ok(Json(CronSettings {
        default_max_retries: cfg.cron.default_max_retries,
        default_timeout_secs: cfg.cron.default_timeout_secs,
        max_concurrent_runs: cfg.cron.max_concurrent_runs,
        heartbeat_interval_minutes: cfg.cron.heartbeat_interval_minutes,
    }))
}

async fn put_cron(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<CronSettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    let mut doc = read_agent_toml(&state)?;
    let table = doc.as_table_mut().ok_or_else(|| internal("agent.toml is not a table"))?;

    let cron = table
        .entry("cron")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| internal("[cron] is not a table"))?;

    cron.insert("default_max_retries".into(), toml::Value::Integer(settings.default_max_retries as i64));
    cron.insert("default_timeout_secs".into(), toml::Value::Integer(settings.default_timeout_secs as i64));
    cron.insert("max_concurrent_runs".into(), toml::Value::Integer(settings.max_concurrent_runs as i64));
    cron.insert("heartbeat_interval_minutes".into(), toml::Value::Integer(settings.heartbeat_interval_minutes.max(1) as i64));

    write_agent_toml(&state, &doc)?;
    Ok(ok_json())
}

// ── Browser config ──────────────────────────────────────────────────────

async fn get_browser(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<BrowserSettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    let cfg = state.config.read().unwrap();
    Ok(Json(BrowserSettings {
        enabled: cfg.browser.enabled,
        cdp_url: cfg.browser.cdp_url.clone(),
        startup_timeout_secs: cfg.browser.startup_timeout_secs,
    }))
}

async fn put_browser(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<BrowserSettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    let mut doc = read_agent_toml(&state)?;
    let table = doc.as_table_mut().ok_or_else(|| internal("agent.toml is not a table"))?;

    let browser = table
        .entry("browser")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| internal("[browser] is not a table"))?;

    browser.insert("enabled".into(), toml::Value::Boolean(settings.enabled));
    browser.insert("startup_timeout_secs".into(), toml::Value::Integer(settings.startup_timeout_secs.max(1) as i64));

    // Handle optional cdp_url: remove the key if None, set if Some
    match settings.cdp_url {
        Some(url) if !url.trim().is_empty() => {
            browser.insert("cdp_url".into(), toml::Value::String(url));
        }
        _ => {
            browser.remove("cdp_url");
        }
    }

    write_agent_toml(&state, &doc)?;
    Ok(ok_json())
}

// ── Frontend config ─────────────────────────────────────────────────────

async fn get_frontend(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<FrontendSettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    let cfg = FrontendConfig::load(&state.paths.config_dir);
    Ok(Json(FrontendSettings {
        greeting: cfg.greeting,
        prompts: cfg.prompts,
    }))
}

async fn put_frontend(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<FrontendSettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    let cfg = FrontendConfig {
        greeting: settings.greeting,
        prompts: settings.prompts,
    };
    let toml_str = toml::to_string_pretty(&cfg).map_err(|e| internal(e))?;
    let path = state.paths.config_dir.join("frontend.toml");
    std::fs::write(&path, toml_str).map_err(|e| internal(e))?;

    Ok(ok_json())
}

// ── Heartbeat ────────────────────────────────────────────────────────────

async fn get_heartbeat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<HeartbeatSettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    let content = state.agent.memory().read_file("HEARTBEAT.md").unwrap_or_default();
    let enabled = !content.trim().is_empty();
    let cfg = state.config.read().unwrap();
    let interval_minutes = cfg.cron.heartbeat_interval_minutes;

    Ok(Json(HeartbeatSettings { enabled, interval_minutes, content }))
}

async fn put_heartbeat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<HeartbeatSettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    // Save interval to config
    let interval = settings.interval_minutes.max(1);
    {
        let mut doc = read_agent_toml(&state)?;
        let table = doc.as_table_mut().ok_or_else(|| internal("agent.toml is not a table"))?;
        let cron = table
            .entry("cron")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| internal("[cron] is not a table"))?;
        cron.insert("heartbeat_interval_minutes".into(), toml::Value::Integer(interval as i64));
        write_agent_toml(&state, &doc)?;
    }

    if settings.enabled {
        // Save content and ensure cron job exists
        let content = if settings.content.trim().is_empty() {
            // Enabled but no content — keep a placeholder so the job stays alive
            return Err(bad_request("Heartbeat content cannot be empty when enabled"));
        } else {
            settings.content
        };

        state.agent.memory().write_file("HEARTBEAT.md", &content).await.map_err(|e| internal(e))?;

        // Create the cron job if it doesn't exist
        let cron_store = state.agent.cron();
        if cron_store.get_job_by_name("__heartbeat__").await.map_err(|e| internal(e))?.is_none() {
            let resolved_tz = state.config.read().unwrap().resolved_timezone();
            let schedule = starpod_cron::Schedule::Cron {
                expr: format!("0 */{interval} * * * *"),
            };
            cron_store.add_job_full(
                "__heartbeat__", &content, &schedule,
                false, resolved_tz.as_deref(), 3, 7200,
                starpod_cron::SessionMode::Main, None,
            ).await.map_err(|e| internal(e))?;
        } else {
            // Update the schedule if the interval changed
            let job = cron_store.get_job_by_name("__heartbeat__").await.map_err(|e| internal(e))?.unwrap();
            let new_schedule = starpod_cron::Schedule::Cron {
                expr: format!("0 */{interval} * * * *"),
            };
            let update = starpod_cron::JobUpdate {
                schedule: Some(new_schedule),
                ..Default::default()
            };
            cron_store.update_job(&job.id, &update).await.map_err(|e| internal(e))?;
        }
    } else {
        // Disabled: clear the file and remove the cron job
        state.agent.memory().write_file("HEARTBEAT.md", "").await.map_err(|e| internal(e))?;

        let cron_store = state.agent.cron();
        // Remove the job if it exists (ignore errors — may not exist)
        let _ = cron_store.remove_job_by_name("__heartbeat__").await;
    }

    Ok(ok_json())
}

// ── Config files (SOUL.md, HEARTBEAT.md, etc.) ──────────────────────────

/// Config markdown files that can be read/written via the settings API.
/// Other files are rejected with 400 Bad Request.
const ALLOWED_FILES: &[&str] = &["SOUL.md", "HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md"];

async fn get_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult<FileContent> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    if !ALLOWED_FILES.contains(&name.as_str()) {
        return Err(bad_request(format!("File '{}' is not editable", name)));
    }

    let content = state.agent.memory().read_file(&name).unwrap_or_default();
    Ok(Json(FileContent { content }))
}

async fn put_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<FileContent>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    if !ALLOWED_FILES.contains(&name.as_str()) {
        return Err(bad_request(format!("File '{}' is not editable", name)));
    }

    state
        .agent
        .memory()
        .write_file(&name, &body.content)
        .await
        .map_err(|e| internal(e))?;

    Ok(ok_json())
}

// ── Users ───────────────────────────────────────────────────────────────

/// Validate a user ID: 1-32 chars, alphanumeric/hyphens/underscores, no traversal.
fn validate_user_id(id: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if id.is_empty() || id.len() > 32 {
        return Err(bad_request("User ID must be 1-32 characters"));
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(bad_request("User ID must be alphanumeric, hyphens, or underscores"));
    }
    if id.contains("..") || id.contains('/') || id.contains('\\') {
        return Err(bad_request("Invalid user ID"));
    }
    Ok(())
}

/// Count `.md` files in the user's `memory/` directory.
fn count_daily_logs(user_dir: &std::path::Path) -> usize {
    let memory_dir = user_dir.join("memory");
    if !memory_dir.is_dir() {
        return 0;
    }
    std::fs::read_dir(&memory_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "md")
                })
                .count()
        })
        .unwrap_or(0)
}

async fn list_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Vec<UserInfo>> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }

    let users_dir = &state.paths.users_dir;
    let mut users = Vec::new();

    if users_dir.is_dir() {
        let entries = std::fs::read_dir(users_dir).map_err(|e| internal(e))?;
        for entry in entries.flatten() {
            let ft = entry.file_type().map_err(|e| internal(e))?;
            if !ft.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            let user_dir = entry.path();
            users.push(UserInfo {
                id,
                has_user_md: user_dir.join("USER.md").is_file(),
                has_memory_md: user_dir.join("MEMORY.md").is_file(),
                daily_log_count: count_daily_logs(&user_dir),
            });
        }
    }

    users.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(users))
}

async fn create_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserInfo>), (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    validate_user_id(&req.id)?;

    let user_dir = state.paths.users_dir.join(&req.id);
    if user_dir.exists() {
        return Err(bad_request(format!("User '{}' already exists", req.id)));
    }

    // Create user directory + seed defaults
    std::fs::create_dir_all(user_dir.join("memory")).map_err(|e| internal(e))?;

    let default_user_md = "# User Profile\n\n## Name\n\n## Role\n\n## Expertise\n\n## Preferences\n\n## Context\n";
    std::fs::write(user_dir.join("USER.md"), default_user_md).map_err(|e| internal(e))?;
    std::fs::write(
        user_dir.join("MEMORY.md"),
        "# Memory Index\n\nImportant facts and links to memory files.\n",
    )
    .map_err(|e| internal(e))?;

    Ok((
        StatusCode::CREATED,
        Json(UserInfo {
            id: req.id,
            has_user_md: true,
            has_memory_md: true,
            daily_log_count: 0,
        }),
    ))
}

async fn get_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<UserDetail> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    validate_user_id(&id)?;

    let user_dir = state.paths.users_dir.join(&id);
    if !user_dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("User '{}' not found", id)));
    }

    let user_md = std::fs::read_to_string(user_dir.join("USER.md")).unwrap_or_default();
    let memory_md = std::fs::read_to_string(user_dir.join("MEMORY.md")).unwrap_or_default();

    Ok(Json(UserDetail {
        id,
        user_md,
        memory_md,
        daily_log_count: count_daily_logs(&user_dir),
    }))
}

async fn update_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<FileContent>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    validate_user_id(&id)?;

    let user_dir = state.paths.users_dir.join(&id);
    if !user_dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("User '{}' not found", id)));
    }

    std::fs::write(user_dir.join("USER.md"), &body.content).map_err(|e| internal(e))?;

    Ok(ok_json())
}

async fn delete_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    validate_user_id(&id)?;

    let user_dir = state.paths.users_dir.join(&id);
    if !user_dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("User '{}' not found", id)));
    }

    std::fs::remove_dir_all(&user_dir).map_err(|e| internal(e))?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Skills ──────────────────────────────────────────────────────────────

fn skill_store(state: &AppState) -> Result<starpod_skills::SkillStore, (StatusCode, Json<ErrorResponse>)> {
    starpod_skills::SkillStore::new(&state.paths.skills_dir).map_err(|e| internal(e))
}

async fn list_skills(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Vec<SkillInfo>> {
    let auth_user = authenticate_request(&state, &headers).await?;
    // Settings routes require admin role when auth is active
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin role required"));
        }
    }
    let store = skill_store(&state)?;
    let skills = store.list().map_err(|e| internal(e))?;
    Ok(Json(
        skills
            .into_iter()
            .map(|s| SkillInfo {
                name: s.name,
                description: s.description,
                version: s.version,
                created_at: s.created_at,
            })
            .collect(),
    ))
}

async fn create_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateSkillRequest>,
) -> Result<(StatusCode, Json<SkillDetail>), (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin role required"));
        }
    }
    let store = skill_store(&state)?;
    store
        .create(&req.name, &req.description, None, &req.body)
        .map_err(|e| bad_request(e.to_string()))?;
    let skill = store
        .get(&req.name)
        .map_err(|e| internal(e))?
        .ok_or_else(|| internal("Skill created but not found"))?;
    Ok((
        StatusCode::CREATED,
        Json(SkillDetail {
            name: skill.name,
            description: skill.description,
            version: skill.version,
            body: skill.body,
            raw_content: skill.raw_content,
            created_at: skill.created_at,
        }),
    ))
}

async fn get_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult<SkillDetail> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin role required"));
        }
    }
    let store = skill_store(&state)?;
    let skill = store
        .get(&name)
        .map_err(|e| internal(e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("Skill '{}' not found", name)))?;
    Ok(Json(SkillDetail {
        name: skill.name,
        description: skill.description,
        version: skill.version,
        body: skill.body,
        raw_content: skill.raw_content,
        created_at: skill.created_at,
    }))
}

async fn update_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<UpdateSkillRequest>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin role required"));
        }
    }
    let store = skill_store(&state)?;
    store
        .update(&name, &req.description, None, &req.body)
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(ok_json())
}

async fn delete_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin role required"));
        }
    }
    let store = skill_store(&state)?;
    store.delete(&name).map_err(|e| bad_request(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

const SKILL_GEN_SYSTEM_PROMPT: &str = r#"You are a skill author for the AgentSkills open format (agentskills.io).

Given a natural language request, generate a skill definition with two fields:
- **description**: 1-2 sentences explaining what the skill does AND when to use it. Use imperative phrasing ("Use this skill when..."). Be "pushy" — explicitly list contexts where the skill applies, including indirect mentions. Max 1024 chars.
- **body**: Markdown instructions the agent follows when the skill is activated. Under 500 lines.

## Best practices for the body

- **Add what the agent lacks, omit what it knows.** Focus on project-specific conventions, domain procedures, non-obvious edge cases. Don't explain general knowledge.
- **Favor procedures over declarations.** Teach how to approach a class of problems, not what to produce for a specific instance.
- **Provide defaults, not menus.** Pick one recommended approach; mention alternatives briefly.
- **Match specificity to fragility.** Be prescriptive when operations are fragile or sequence matters; give freedom when multiple approaches are valid.
- **Use effective patterns:**
  - Gotchas sections for environment-specific facts that defy assumptions
  - Templates for output format (concrete structure, not prose)
  - Checklists for multi-step workflows with explicit step tracking
  - Validation loops: do work → run validator → fix issues → repeat
  - Plan-validate-execute: create plan → validate → execute
- **Design coherent units.** Not too narrow (needing multiple skills for one task), not too broad (hard to activate precisely).
- **Keep it actionable.** Concise stepwise guidance with working examples outperforms exhaustive documentation.

## Output

Return a JSON object with exactly: `description`, `body`.
"#;

/// Strip optional markdown code fences from an AI response.
fn strip_json_fence(raw: &str) -> &str {
    let s = raw.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    s.strip_suffix("```").unwrap_or(s).trim()
}

async fn generate_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<GenerateSkillRequest>,
) -> ApiResult<GenerateSkillResponse> {
    let auth_user = authenticate_request(&state, &headers).await?;
    if let Some(ref u) = auth_user {
        if u.role != starpod_auth::Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin role required"));
        }
    }

    // Build user prompt
    let mut user_prompt = format!("Create a skill named \"{}\".", req.name);
    if let Some(ref d) = req.description {
        if !d.is_empty() {
            user_prompt.push_str(&format!("\n\nThe skill description MUST be: {}", d));
        }
    }
    if let Some(ref p) = req.prompt {
        if !p.is_empty() {
            user_prompt.push_str(&format!("\n\nAdditional context:\n{}", p));
        }
    }

    let output_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "description": {
                "type": "string",
                "description": "1-2 sentence description of what the skill does and when to use it."
            },
            "body": {
                "type": "string",
                "description": "Markdown instructions for the agent to follow when the skill is activated."
            }
        },
        "required": ["description", "body"],
        "additionalProperties": false
    });

    let options = agent_sdk::Options::builder()
        .system_prompt(agent_sdk::options::SystemPrompt::Custom(
            SKILL_GEN_SYSTEM_PROMPT.to_string(),
        ))
        .output_format(output_schema)
        .max_turns(1)
        .persist_session(false)
        .permission_mode(agent_sdk::PermissionMode::Plan)
        .build();

    let mut stream = agent_sdk::query(&user_prompt, options);

    use futures::StreamExt;
    let mut result_msg = None;
    while let Some(msg_result) = stream.next().await {
        let msg = msg_result.map_err(|e| internal(e))?;
        if let agent_sdk::Message::Result(result) = msg {
            result_msg = Some(result);
        }
    }

    let result = result_msg
        .ok_or_else(|| internal("No result from AI"))?;

    if result.is_error {
        return Err(internal(result.errors.join("; ")));
    }

    let result_text = result.result.ok_or_else(|| {
        internal("No text returned from AI")
    })?;

    #[derive(serde::Deserialize)]
    struct SkillGen {
        description: String,
        body: String,
    }

    let json_str = strip_json_fence(&result_text);
    let gen: SkillGen = serde_json::from_str(json_str)
        .map_err(|e| internal(format!("Failed to parse AI response: {e}")))?;

    Ok(Json(GenerateSkillResponse {
        description: gen.description,
        body: gen.body,
    }))
}

// ── Auth user management ─────────────────────────────────────────────────

/// Require admin role, returning a 403 if the user is not an admin.
fn require_admin(user: &Option<starpod_auth::User>) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if let Some(ref u) = user {
        if u.role != Role::Admin {
            return Err(err(StatusCode::FORBIDDEN, "Admin access required"));
        }
    }
    Ok(())
}

async fn list_auth_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Vec<starpod_auth::User>> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    state
        .auth
        .list_users()
        .await
        .map(Json)
        .map_err(|e| internal(e))
}

async fn get_auth_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<starpod_auth::User> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    state
        .auth
        .get_user(&id)
        .await
        .map_err(|e| internal(e))?
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "User not found"))
}

async fn create_auth_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateAuthUserRequest>,
) -> Result<(StatusCode, Json<starpod_auth::User>), (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let user = state
        .auth
        .create_user(req.email.as_deref(), req.display_name.as_deref(), req.role)
        .await
        .map_err(|e| internal(e))?;

    Ok((StatusCode::CREATED, Json(user)))
}

async fn update_auth_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateAuthUserRequest>,
) -> ApiResult<starpod_auth::User> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    // Verify user exists
    state
        .auth
        .get_user(&id)
        .await
        .map_err(|e| internal(e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "User not found"))?;

    state
        .auth
        .update_user(&id, req.email.as_deref(), req.display_name.as_deref(), req.role)
        .await
        .map_err(|e| internal(e))?;

    // Return updated user
    state
        .auth
        .get_user(&id)
        .await
        .map_err(|e| internal(e))?
        .map(Json)
        .ok_or_else(|| internal("User disappeared after update"))
}

async fn deactivate_auth_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    // Prevent self-deactivation
    if let Some(ref u) = auth_user {
        if u.id == id {
            return Err(bad_request("Cannot deactivate yourself"));
        }
    }

    state
        .auth
        .deactivate_user(&id)
        .await
        .map_err(|e| internal(e))?;

    Ok(ok_json())
}

async fn activate_auth_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    state
        .auth
        .activate_user(&id)
        .await
        .map_err(|e| internal(e))?;

    Ok(ok_json())
}

async fn list_auth_api_keys(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> ApiResult<Vec<starpod_auth::ApiKeyMeta>> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    state
        .auth
        .list_api_keys(&user_id)
        .await
        .map(Json)
        .map_err(|e| internal(e))
}

async fn create_auth_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<starpod_auth::ApiKeyCreated>), (StatusCode, Json<ErrorResponse>)> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    // Verify user exists
    state
        .auth
        .get_user(&user_id)
        .await
        .map_err(|e| internal(e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "User not found"))?;

    let created = state
        .auth
        .create_api_key(&user_id, req.label.as_deref())
        .await
        .map_err(|e| internal(e))?;

    Ok((StatusCode::CREATED, Json(created)))
}

async fn revoke_auth_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    state
        .auth
        .revoke_api_key(&key_id)
        .await
        .map_err(|e| internal(e))?;

    Ok(ok_json())
}

// ── Compaction ──────────────────────────────────────────────────────────

async fn get_compaction(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<CompactionSettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let cfg = state.config.read().unwrap();
    Ok(Json(CompactionSettings {
        context_budget: cfg.compaction.context_budget,
        summary_max_tokens: cfg.compaction.summary_max_tokens,
        min_keep_messages: cfg.compaction.min_keep_messages,
        max_tool_result_bytes: cfg.compaction.max_tool_result_bytes,
        prune_threshold_pct: cfg.compaction.prune_threshold_pct,
        prune_tool_result_max_chars: cfg.compaction.prune_tool_result_max_chars,
        memory_flush: cfg.compaction.memory_flush,
    }))
}

async fn put_compaction(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<CompactionSettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let mut doc = read_agent_toml(&state)?;
    let table = doc
        .as_table_mut()
        .ok_or_else(|| internal("agent.toml is not a table"))?;

    let compaction = table
        .entry("compaction")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| internal("[compaction] is not a table"))?;

    compaction.insert("context_budget".into(), toml::Value::Integer(settings.context_budget as i64));
    compaction.insert("summary_max_tokens".into(), toml::Value::Integer(settings.summary_max_tokens as i64));
    compaction.insert("min_keep_messages".into(), toml::Value::Integer(settings.min_keep_messages as i64));
    compaction.insert("max_tool_result_bytes".into(), toml::Value::Integer(settings.max_tool_result_bytes as i64));
    compaction.insert("prune_threshold_pct".into(), toml::Value::Integer(settings.prune_threshold_pct as i64));
    compaction.insert("prune_tool_result_max_chars".into(), toml::Value::Integer(settings.prune_tool_result_max_chars as i64));
    compaction.insert("memory_flush".into(), toml::Value::Boolean(settings.memory_flush));

    write_agent_toml(&state, &doc)?;

    Ok(ok_json())
}

// ── Internet ────────────────────────────────────────────────────────────

async fn get_internet(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<InternetSettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let cfg = state.config.read().unwrap();
    let brave_api_key = read_env_var(&state.paths, "BRAVE_API_KEY");
    Ok(Json(InternetSettings {
        enabled: cfg.internet.enabled,
        timeout_secs: cfg.internet.timeout_secs,
        max_fetch_bytes: cfg.internet.max_fetch_bytes,
        max_text_chars: cfg.internet.max_text_chars,
        brave_api_key,
    }))
}

async fn put_internet(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<InternetSettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let mut doc = read_agent_toml(&state)?;
    let table = doc
        .as_table_mut()
        .ok_or_else(|| internal("agent.toml is not a table"))?;

    let internet = table
        .entry("internet")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| internal("[internet] is not a table"))?;

    internet.insert("enabled".into(), toml::Value::Boolean(settings.enabled));
    internet.insert(
        "timeout_secs".into(),
        toml::Value::Integer(settings.timeout_secs as i64),
    );
    internet.insert(
        "max_fetch_bytes".into(),
        toml::Value::Integer(settings.max_fetch_bytes as i64),
    );
    internet.insert(
        "max_text_chars".into(),
        toml::Value::Integer(settings.max_text_chars as i64),
    );

    write_agent_toml(&state, &doc)?;

    // Write Brave API key to .env
    if let Some(ref key) = settings.brave_api_key {
        write_env_var(
            &state.paths,
            "BRAVE_API_KEY",
            if key.is_empty() { None } else { Some(key) },
        )?;
    }

    Ok(ok_json())
}

// ── Channels ────────────────────────────────────────────────────────────

async fn get_channels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<ChannelsSettings> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let cfg = state.config.read().unwrap();
    let tg = cfg.channels.telegram.clone().unwrap_or_default();
    let bot_token = read_env_var(&state.paths, "TELEGRAM_BOT_TOKEN");
    Ok(Json(ChannelsSettings {
        telegram: TelegramChannelSettings {
            enabled: tg.enabled,
            gap_minutes: tg.gap_minutes,
            stream_mode: Some(tg.stream_mode),
            bot_token,
        },
    }))
}

async fn put_channels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<ChannelsSettings>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let mut doc = read_agent_toml(&state)?;
    let table = doc.as_table_mut().ok_or_else(|| internal("agent.toml is not a table"))?;

    let channels = table
        .entry("channels")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| internal("channels is not a table"))?;

    let tg = channels
        .entry("telegram")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| internal("channels.telegram is not a table"))?;

    tg.insert("enabled".into(), toml::Value::Boolean(settings.telegram.enabled));
    if let Some(gap) = settings.telegram.gap_minutes {
        tg.insert("gap_minutes".into(), toml::Value::Integer(gap));
    } else {
        tg.remove("gap_minutes");
    }
    if let Some(ref mode) = settings.telegram.stream_mode {
        tg.insert("stream_mode".into(), toml::Value::String(mode.clone()));
    }

    write_agent_toml(&state, &doc)?;

    // Write bot token to .env file
    if let Some(ref token) = settings.telegram.bot_token {
        write_env_var(&state.paths, "TELEGRAM_BOT_TOKEN", if token.is_empty() { None } else { Some(token) })?;
    }

    Ok(ok_json())
}

// ── Costs ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CostsQuery {
    /// Optional period filter: "7d", "30d", "90d", or "all". Defaults to "30d".
    #[serde(default = "default_period")]
    period: String,
}

fn default_period() -> String {
    "30d".into()
}

async fn get_costs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<CostsQuery>,
) -> ApiResult<starpod_session::CostOverview> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let since = match query.period.as_str() {
        "7d" => Some(chrono::Utc::now() - chrono::Duration::days(7)),
        "30d" => Some(chrono::Utc::now() - chrono::Duration::days(30)),
        "90d" => Some(chrono::Utc::now() - chrono::Duration::days(90)),
        "all" => None,
        _ => Some(chrono::Utc::now() - chrono::Duration::days(30)),
    };

    let since_str = since.map(|dt| dt.to_rfc3339());
    let overview = state
        .agent
        .session_mgr()
        .cost_overview(since_str.as_deref())
        .await
        .map_err(|e| internal(format!("Cost query failed: {}", e)))?;

    Ok(Json(overview))
}

// ── Telegram linking ────────────────────────────────────────────────────

async fn get_user_telegram(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    let link = state
        .auth
        .get_telegram_link_for_user(&user_id)
        .await
        .map_err(|e| internal(e))?;

    match link {
        Some(l) => Ok(Json(serde_json::json!({
            "telegram_id": l.telegram_id,
            "username": l.username,
            "linked_at": l.linked_at.to_rfc3339(),
        }))),
        None => Ok(Json(serde_json::json!({}))),
    }
}

async fn put_user_telegram(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(req): Json<LinkTelegramRequest>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    // Verify user exists
    state
        .auth
        .get_user(&user_id)
        .await
        .map_err(|e| internal(e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "User not found"))?;

    let link = state
        .auth
        .link_telegram(&user_id, req.telegram_id, req.username.as_deref())
        .await
        .map_err(|e| internal(e))?;

    Ok(Json(serde_json::json!({
        "telegram_id": link.telegram_id,
        "username": link.username,
        "linked_at": link.linked_at.to_rfc3339(),
    })))
}

async fn delete_user_telegram(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let auth_user = authenticate_request(&state, &headers).await?;
    require_admin(&auth_user)?;

    state
        .auth
        .unlink_telegram_by_user(&user_id)
        .await
        .map_err(|e| internal(e))?;

    Ok(ok_json())
}

// ── TOML helpers ────────────────────────────────────────────────────────

fn read_agent_toml(state: &AppState) -> Result<toml::Value, (StatusCode, Json<ErrorResponse>)> {
    let path = &state.paths.agent_toml;
    let content = std::fs::read_to_string(path)
        .map_err(|e| internal(format!("Failed to read {}: {}", path.display(), e)))?;
    toml::from_str(&content)
        .map_err(|e| internal(format!("Failed to parse {}: {}", path.display(), e)))
}

fn write_agent_toml(
    state: &AppState,
    doc: &toml::Value,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let toml_str = toml::to_string_pretty(doc)
        .map_err(|e| internal(format!("Failed to serialize TOML: {}", e)))?;
    std::fs::write(&state.paths.agent_toml, &toml_str)
        .map_err(|e| internal(format!("Failed to write {}: {}", state.paths.agent_toml.display(), e)))?;

    // Update in-memory config immediately so subsequent reads don't return
    // stale data (the file-watcher has a 2-second debounce).
    if let Ok(agent_cfg) = reload_agent_config(&state.paths) {
        let new_config = agent_cfg.into_starpod_config(&state.paths);
        state.agent.reload_config(new_config.clone());
        *state.config.write().unwrap() = new_config;
    }

    Ok(())
}

/// Insert a string value into a TOML table, or remove the key if the value is `None` or empty.
fn set_or_remove_string(table: &mut toml::map::Map<String, toml::Value>, key: &str, val: Option<String>) {
    match val {
        Some(v) if !v.is_empty() => { table.insert(key.into(), toml::Value::String(v)); }
        _ => { table.remove(key); }
    }
}

/// Read a variable from the `.env` file (raw file parsing, not `std::env`).
fn read_env_var(paths: &ResolvedPaths, key: &str) -> Option<String> {
    let env_path = paths.agent_home.join(".env");
    let content = std::fs::read_to_string(&env_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Write (or remove) a variable in the `.env` file.
fn write_env_var(
    paths: &ResolvedPaths,
    key: &str,
    value: Option<&str>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let env_path = paths.agent_home.join(".env");
    let content = std::fs::read_to_string(&env_path).unwrap_or_default();

    let mut found = false;
    let mut lines: Vec<String> = content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with(&format!("{key}=")) {
                found = true;
                // Replace or remove
                value.map(|v| format!("{key}={v}"))
            } else {
                Some(line.to_string())
            }
        })
        .collect();

    if !found {
        if let Some(v) = value {
            lines.push(format!("{key}={v}"));
        }
    }

    // Ensure trailing newline
    let mut output = lines.join("\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }

    std::fs::write(&env_path, output)
        .map_err(|e| internal(format!("Failed to write {}: {}", env_path.display(), e)))?;

    // Also update the process env so config reload picks it up
    match value {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

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

    // ── Test fixtures ───────────────────────────────────────────────────

    /// Build a fully wired AppState with temp directories for isolated tests.
    async fn test_app_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::TempDir::new().unwrap();
        let starpod_dir = tmp.path().join(".starpod");
        let config_dir = starpod_dir.join("config");
        let db_dir = starpod_dir.join("db");
        let users_dir = starpod_dir.join("users");
        let skills_dir = starpod_dir.join("skills");
        let agent_toml = config_dir.join("agent.toml");

        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::create_dir_all(&users_dir).unwrap();
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Write a minimal agent.toml
        std::fs::write(&agent_toml, "model = \"test-model\"\nagent_name = \"TestBot\"\n").unwrap();

        let config = StarpodConfig {
            db_dir: db_dir.clone(),
            db_path: Some(db_dir.join("memory.db")),
            project_root: tmp.path().to_path_buf(),
            model: "test-model".into(),
            agent_name: "TestBot".into(),
            ..StarpodConfig::default()
        };

        let agent = StarpodAgent::new(config.clone()).await.unwrap();
        let (events_tx, _) = tokio::sync::broadcast::channel::<GatewayEvent>(16);

        let paths = ResolvedPaths {
            mode: Mode::SingleAgent { starpod_dir: starpod_dir.clone() },
            agent_toml,
            agent_home: starpod_dir.clone(),
            config_dir,
            db_dir,
            skills_dir,
            project_root: tmp.path().join("home"),
            instance_root: tmp.path().to_path_buf(),
            home_dir: tmp.path().join("home"),
            users_dir,
            env_file: None,
        };

        let auth_db_path = paths.db_dir.join("users.db");
        let auth = Arc::new(AuthStore::new(&auth_db_path).await.unwrap());
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

    /// Make a GET request and return (status, body json).
    async fn get_json(state: Arc<AppState>, path: &str) -> (StatusCode, serde_json::Value) {
        let app = build_router(state);
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        (status, json)
    }

    /// Make a PUT request with JSON body and return (status, body json).
    async fn put_json(state: Arc<AppState>, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let app = build_router(state);
        let req = Request::builder()
            .method("PUT")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, json)
    }

    /// Make a POST request with JSON body and return (status, body json).
    async fn post_json(state: Arc<AppState>, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let app = build_router(state);
        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, json)
    }

    /// Make a DELETE request and return status code.
    async fn delete_req(state: Arc<AppState>, path: &str) -> StatusCode {
        let app = build_router(state);
        let req = Request::builder()
            .method("DELETE")
            .uri(path)
            .body(Body::empty())
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    // ── validate_user_id ────────────────────────────────────────────────

    #[test]
    fn valid_user_ids() {
        assert!(validate_user_id("alice").is_ok());
        assert!(validate_user_id("user-1").is_ok());
        assert!(validate_user_id("admin_2").is_ok());
        assert!(validate_user_id("a").is_ok());
        assert!(validate_user_id("A1-b2_C3").is_ok());
    }

    #[test]
    fn user_id_empty_rejected() {
        assert!(validate_user_id("").is_err());
    }

    #[test]
    fn user_id_too_long_rejected() {
        let long_id = "a".repeat(33);
        assert!(validate_user_id(&long_id).is_err());
    }

    #[test]
    fn user_id_max_length_accepted() {
        let id = "a".repeat(32);
        assert!(validate_user_id(&id).is_ok());
    }

    #[test]
    fn user_id_special_chars_rejected() {
        assert!(validate_user_id("user@name").is_err());
        assert!(validate_user_id("user name").is_err());
        assert!(validate_user_id("user.name").is_err());
        assert!(validate_user_id("user!").is_err());
    }

    #[test]
    fn user_id_traversal_rejected() {
        assert!(validate_user_id("..").is_err());
        assert!(validate_user_id("../etc").is_err());
        assert!(validate_user_id("foo/bar").is_err());
        assert!(validate_user_id("foo\\bar").is_err());
    }

    // ── count_daily_logs ────────────────────────────────────────────────

    #[test]
    fn count_daily_logs_no_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(count_daily_logs(tmp.path()), 0);
    }

    #[test]
    fn count_daily_logs_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("memory")).unwrap();
        assert_eq!(count_daily_logs(tmp.path()), 0);
    }

    #[test]
    fn count_daily_logs_with_md_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mem = tmp.path().join("memory");
        std::fs::create_dir_all(&mem).unwrap();
        std::fs::write(mem.join("2026-03-17.md"), "log1").unwrap();
        std::fs::write(mem.join("2026-03-18.md"), "log2").unwrap();
        std::fs::write(mem.join("2026-03-19.md"), "log3").unwrap();
        assert_eq!(count_daily_logs(tmp.path()), 3);
    }

    #[test]
    fn count_daily_logs_ignores_non_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mem = tmp.path().join("memory");
        std::fs::create_dir_all(&mem).unwrap();
        std::fs::write(mem.join("2026-03-17.md"), "log").unwrap();
        std::fs::write(mem.join("notes.txt"), "txt").unwrap();
        std::fs::write(mem.join("data.json"), "{}").unwrap();
        assert_eq!(count_daily_logs(tmp.path()), 1);
    }

    // ── set_or_remove_string ────────────────────────────────────────────

    #[test]
    fn set_or_remove_inserts_value() {
        let mut table = toml::map::Map::new();
        set_or_remove_string(&mut table, "key", Some("value".into()));
        assert_eq!(table.get("key").unwrap().as_str(), Some("value"));
    }

    #[test]
    fn set_or_remove_removes_on_none() {
        let mut table = toml::map::Map::new();
        table.insert("key".into(), toml::Value::String("old".into()));
        set_or_remove_string(&mut table, "key", None);
        assert!(!table.contains_key("key"));
    }

    #[test]
    fn set_or_remove_removes_on_empty() {
        let mut table = toml::map::Map::new();
        table.insert("key".into(), toml::Value::String("old".into()));
        set_or_remove_string(&mut table, "key", Some(String::new()));
        assert!(!table.contains_key("key"));
    }

    #[test]
    fn set_or_remove_overwrites_existing() {
        let mut table = toml::map::Map::new();
        table.insert("key".into(), toml::Value::String("old".into()));
        set_or_remove_string(&mut table, "key", Some("new".into()));
        assert_eq!(table.get("key").unwrap().as_str(), Some("new"));
    }

    // ── ALLOWED_FILES ───────────────────────────────────────────────────

    #[test]
    fn allowed_files_contains_expected() {
        assert!(ALLOWED_FILES.contains(&"SOUL.md"));
        assert!(ALLOWED_FILES.contains(&"HEARTBEAT.md"));
        assert!(ALLOWED_FILES.contains(&"BOOT.md"));
        assert!(ALLOWED_FILES.contains(&"BOOTSTRAP.md"));
        assert!(!ALLOWED_FILES.contains(&"USER.md"));
        assert!(!ALLOWED_FILES.contains(&"agent.toml"));
        assert!(!ALLOWED_FILES.contains(&"../etc/passwd"));
    }

    // ── Serialization round-trips ───────────────────────────────────────

    #[test]
    fn general_settings_serializes() {
        let settings = GeneralSettings {
            provider: "anthropic".into(),
            model: "claude-haiku-4-5".into(),
            max_turns: 30,
            max_tokens: 16384,
            agent_name: "Aster".into(),
            timezone: Some("Europe/Rome".into()),
            reasoning_effort: Some(ReasoningEffort::High),
            compaction_model: None,
            compaction_provider: None,
            followup_mode: FollowupMode::Inject,
            server_addr: "127.0.0.1:3000".into(),
            self_improve: false,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: GeneralSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, "anthropic");
        assert_eq!(back.model, "claude-haiku-4-5");
        assert_eq!(back.timezone.as_deref(), Some("Europe/Rome"));
        assert!(back.compaction_model.is_none());
        assert!(back.compaction_provider.is_none());
    }

    #[test]
    fn general_settings_deserializes_with_defaults() {
        // Missing optional fields should default
        let json = r#"{"provider":"openai","model":"gpt-4","max_turns":10,"max_tokens":4096,"agent_name":"Bot","server_addr":"0.0.0.0:8080"}"#;
        let s: GeneralSettings = serde_json::from_str(json).unwrap();
        assert!(s.timezone.is_none());
        assert!(s.reasoning_effort.is_none());
        assert!(s.compaction_model.is_none());
        assert_eq!(s.followup_mode, FollowupMode::Inject); // default
    }

    #[test]
    fn model_registry_all_providers_present() {
        let reg = agent_sdk::models::ModelRegistry::with_defaults();
        let m = reg.models_by_provider();
        assert!(m.contains_key("anthropic"));
        assert!(m.contains_key("openai"));
        assert!(m.contains_key("gemini"));
        assert!(m.contains_key("groq"));
        assert!(m.contains_key("deepseek"));
        assert!(m.contains_key("openrouter"));
        assert!(m.contains_key("ollama"));
        assert!(m["anthropic"].contains(&"claude-sonnet-4-6".to_string()));
        assert!(m["ollama"].is_empty()); // user must type for ollama
    }

    #[tokio::test]
    async fn get_models_returns_all_providers() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/models").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["models"]["anthropic"].as_array().unwrap().len() >= 3);
        assert!(json["models"]["ollama"].as_array().unwrap().is_empty());
    }

    #[test]
    fn memory_settings_round_trip() {
        let settings = MemorySettings {
            half_life_days: 14.0,
            mmr_lambda: 0.5,
            vector_search: false,
            chunk_size: 800,
            chunk_overlap: 160,
            export_sessions: true,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: MemorySettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.half_life_days, 14.0);
        assert!(!back.vector_search);
    }

    #[test]
    fn cron_settings_round_trip() {
        let settings = CronSettings {
            default_max_retries: 5,
            default_timeout_secs: 3600,
            max_concurrent_runs: 2,
            heartbeat_interval_minutes: 15,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: CronSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.default_max_retries, 5);
        assert_eq!(back.default_timeout_secs, 3600);
    }

    #[test]
    fn frontend_settings_round_trip() {
        let settings = FrontendSettings {
            greeting: Some("Hello!".into()),
            prompts: vec!["help".into(), "joke".into()],
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: FrontendSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.greeting.as_deref(), Some("Hello!"));
        assert_eq!(back.prompts.len(), 2);
    }

    #[test]
    fn user_info_serializes() {
        let info = UserInfo {
            id: "alice".into(),
            has_user_md: true,
            has_memory_md: true,
            daily_log_count: 5,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"id\":\"alice\""));
        assert!(json.contains("\"daily_log_count\":5"));
    }

    // ── Integration tests (full HTTP round-trip) ────────────────────────

    #[tokio::test]
    async fn get_general_returns_config() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/general").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["model"], "test-model");
        assert_eq!(json["agent_name"], "TestBot");
    }

    #[tokio::test]
    async fn put_general_updates_agent_toml() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = put_json(
            Arc::clone(&state),
            "/api/settings/general",
            serde_json::json!({
                "provider": "openai",
                "model": "gpt-4o",
                "max_turns": 50,
                "max_tokens": 8192,
                "agent_name": "Nova",
                "timezone": "US/Pacific",
                "server_addr": "0.0.0.0:8080",
                "followup_mode": "queue"
            }),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");

        // Verify the file was written
        let content = std::fs::read_to_string(&state.paths.agent_toml).unwrap();
        assert!(content.contains("gpt-4o"));
        assert!(content.contains("Nova"));
        assert!(content.contains("US/Pacific"));

        // Verify round-trip: read it back
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(parsed["model"].as_str(), Some("gpt-4o"));
        assert_eq!(parsed["max_turns"].as_integer(), Some(50));
    }

    #[tokio::test]
    async fn put_general_rejects_empty_model() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = put_json(
            state,
            "/api/settings/general",
            serde_json::json!({
                "provider": "anthropic", "model": "", "max_turns": 1,
                "max_tokens": 1024, "agent_name": "x", "server_addr": "x"
            }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("model"));
    }

    #[tokio::test]
    async fn put_general_rejects_zero_max_turns() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = put_json(
            state,
            "/api/settings/general",
            serde_json::json!({
                "provider": "anthropic", "model": "m", "max_turns": 0,
                "max_tokens": 1024, "agent_name": "x", "server_addr": "x"
            }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_general_optional_fields_cleared() {
        let (_tmp, state) = test_app_state().await;

        // First set timezone
        let _ = put_json(
            Arc::clone(&state),
            "/api/settings/general",
            serde_json::json!({
                "provider": "anthropic", "model": "m", "max_turns": 1,
                "max_tokens": 1024, "agent_name": "x", "server_addr": "x",
                "timezone": "UTC"
            }),
        ).await;

        // Then clear it
        let _ = put_json(
            Arc::clone(&state),
            "/api/settings/general",
            serde_json::json!({
                "provider": "anthropic", "model": "m", "max_turns": 1,
                "max_tokens": 1024, "agent_name": "x", "server_addr": "x",
                "timezone": null
            }),
        ).await;

        let content = std::fs::read_to_string(&state.paths.agent_toml).unwrap();
        assert!(!content.contains("timezone"));
    }

    #[tokio::test]
    async fn get_memory_returns_defaults() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/memory").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["half_life_days"], 30.0);
        assert_eq!(json["mmr_lambda"], 0.7);
        assert_eq!(json["vector_search"], true);
    }

    #[tokio::test]
    async fn put_memory_updates_toml() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/memory",
            serde_json::json!({
                "half_life_days": 14.0, "mmr_lambda": 0.5, "vector_search": false,
                "chunk_size": 800, "chunk_overlap": 160, "export_sessions": false
            }),
        ).await;
        assert_eq!(status, StatusCode::OK);

        let content = std::fs::read_to_string(&state.paths.agent_toml).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(parsed["memory"]["half_life_days"].as_float(), Some(14.0));
        assert_eq!(parsed["memory"]["vector_search"].as_bool(), Some(false));
    }

    #[tokio::test]
    async fn get_cron_returns_defaults() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/cron").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["default_max_retries"], 3);
        assert_eq!(json["default_timeout_secs"], 7200);
    }

    #[tokio::test]
    async fn put_cron_updates_toml() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/cron",
            serde_json::json!({
                "default_max_retries": 5, "default_timeout_secs": 3600, "max_concurrent_runs": 4, "heartbeat_interval_minutes": 15
            }),
        ).await;
        assert_eq!(status, StatusCode::OK);

        let content = std::fs::read_to_string(&state.paths.agent_toml).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(parsed["cron"]["default_max_retries"].as_integer(), Some(5));
    }

    // ── Heartbeat settings tests ─────────────────────────────────────────

    #[test]
    fn heartbeat_settings_round_trip() {
        let settings = HeartbeatSettings {
            enabled: true,
            interval_minutes: 15,
            content: "Check for new tasks".into(),
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: HeartbeatSettings = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.interval_minutes, 15);
        assert_eq!(back.content, "Check for new tasks");
    }

    #[tokio::test]
    async fn get_heartbeat_returns_disabled_when_empty() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/heartbeat").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["enabled"], false);
        assert_eq!(json["interval_minutes"], 30); // default
        assert_eq!(json["content"], "");
    }

    #[tokio::test]
    async fn put_heartbeat_enable_and_disable() {
        let (_tmp, state) = test_app_state().await;

        // Enable heartbeat
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/heartbeat",
            serde_json::json!({
                "enabled": true,
                "interval_minutes": 15,
                "content": "Do something"
            }),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // Verify it reads back as enabled
        let (status, json) = get_json(Arc::clone(&state), "/api/settings/heartbeat").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["enabled"], true);
        assert_eq!(json["content"], "Do something");

        // Verify interval was persisted to TOML
        let content = std::fs::read_to_string(&state.paths.agent_toml).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(parsed["cron"]["heartbeat_interval_minutes"].as_integer(), Some(15));

        // Verify cron job was created
        let job = state.agent.cron().get_job_by_name("__heartbeat__").await.unwrap();
        assert!(job.is_some());

        // Disable heartbeat
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/heartbeat",
            serde_json::json!({
                "enabled": false,
                "interval_minutes": 15,
                "content": ""
            }),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // Verify it reads back as disabled
        let (status, json) = get_json(Arc::clone(&state), "/api/settings/heartbeat").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["enabled"], false);

        // Verify cron job was removed
        let job = state.agent.cron().get_job_by_name("__heartbeat__").await.unwrap();
        assert!(job.is_none());
    }

    #[tokio::test]
    async fn put_heartbeat_enabled_empty_content_rejected() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = put_json(
            state,
            "/api/settings/heartbeat",
            serde_json::json!({
                "enabled": true,
                "interval_minutes": 30,
                "content": "   "
            }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("empty"));
    }

    #[tokio::test]
    async fn frontend_round_trip() {
        let (_tmp, state) = test_app_state().await;

        // Write
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/frontend",
            serde_json::json!({ "greeting": "Hi!", "prompts": ["help", "joke"] }),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // Read back
        let (status, json) = get_json(state, "/api/settings/frontend").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["greeting"], "Hi!");
        assert_eq!(json["prompts"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn get_file_soul_md() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/files/SOUL.md").await;
        assert_eq!(status, StatusCode::OK);
        // Default SOUL.md contains "Aster"
        assert!(json["content"].as_str().unwrap().contains("Aster"));
    }

    #[tokio::test]
    async fn put_file_soul_md() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/files/SOUL.md",
            serde_json::json!({ "content": "# Soul\nYou are Nova." }),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // Read back
        let (_, json) = get_json(state, "/api/settings/files/SOUL.md").await;
        assert!(json["content"].as_str().unwrap().contains("Nova"));
    }

    #[tokio::test]
    async fn get_file_rejects_unknown() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/files/secret.md").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("not editable"));
    }

    // ── User CRUD integration tests ─────────────────────────────────────

    #[tokio::test]
    async fn user_crud_lifecycle() {
        let (_tmp, state) = test_app_state().await;

        // List: initially may have default users (admin, user) from agent setup
        let (status, json) = get_json(Arc::clone(&state), "/api/settings/users").await;
        assert_eq!(status, StatusCode::OK);
        let initial_count = json.as_array().unwrap().len();

        // Create
        let (status, json) = post_json(
            Arc::clone(&state),
            "/api/settings/users",
            serde_json::json!({ "id": "testuser" }),
        ).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(json["id"], "testuser");
        assert_eq!(json["has_user_md"], true);
        assert_eq!(json["has_memory_md"], true);

        // Verify filesystem
        assert!(state.paths.users_dir.join("testuser").join("USER.md").exists());
        assert!(state.paths.users_dir.join("testuser").join("MEMORY.md").exists());
        assert!(state.paths.users_dir.join("testuser").join("memory").is_dir());

        // List again
        let (_, json) = get_json(Arc::clone(&state), "/api/settings/users").await;
        assert_eq!(json.as_array().unwrap().len(), initial_count + 1);

        // Get user detail
        let (status, json) = get_json(Arc::clone(&state), "/api/settings/users/testuser").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["id"], "testuser");
        assert!(json["user_md"].as_str().unwrap().contains("User Profile"));

        // Update USER.md
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/users/testuser",
            serde_json::json!({ "content": "# User\nAlice is a developer." }),
        ).await;
        assert_eq!(status, StatusCode::OK);

        // Read back
        let content = std::fs::read_to_string(
            state.paths.users_dir.join("testuser").join("USER.md"),
        ).unwrap();
        assert!(content.contains("Alice is a developer"));

        // Delete
        let status = delete_req(Arc::clone(&state), "/api/settings/users/testuser").await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!state.paths.users_dir.join("testuser").exists());

        // List again — back to original count
        let (_, json) = get_json(state, "/api/settings/users").await;
        assert_eq!(json.as_array().unwrap().len(), initial_count);
    }

    #[tokio::test]
    async fn create_user_duplicate_rejected() {
        let (_tmp, state) = test_app_state().await;

        let _ = post_json(
            Arc::clone(&state),
            "/api/settings/users",
            serde_json::json!({ "id": "dup" }),
        ).await;

        let (status, json) = post_json(
            state,
            "/api/settings/users",
            serde_json::json!({ "id": "dup" }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("already exists"));
    }

    #[tokio::test]
    async fn create_user_invalid_id_rejected() {
        let (_tmp, state) = test_app_state().await;

        let (status, _) = post_json(
            Arc::clone(&state),
            "/api/settings/users",
            serde_json::json!({ "id": "bad user!" }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = post_json(
            state,
            "/api/settings/users",
            serde_json::json!({ "id": "" }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_user_not_found() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = get_json(state, "/api/settings/users/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_user_not_found() {
        let (_tmp, state) = test_app_state().await;
        let status = delete_req(state, "/api/settings/users/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // ── Skill CRUD integration tests ────────────────────────────────────

    #[tokio::test]
    async fn skill_crud_lifecycle() {
        let (_tmp, state) = test_app_state().await;

        // List: initially empty
        let (status, json) = get_json(Arc::clone(&state), "/api/settings/skills").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.as_array().unwrap().len(), 0);

        // Create
        let (status, json) = post_json(
            Arc::clone(&state),
            "/api/settings/skills",
            serde_json::json!({ "name": "greet", "description": "A greeting skill", "body": "Say hello!" }),
        ).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(json["name"], "greet");
        assert_eq!(json["description"], "A greeting skill");
        assert!(json["body"].as_str().unwrap().contains("Say hello!"));

        // List again
        let (_, json) = get_json(Arc::clone(&state), "/api/settings/skills").await;
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0]["name"], "greet");
        assert_eq!(json[0]["description"], "A greeting skill");

        // Get detail
        let (status, json) = get_json(Arc::clone(&state), "/api/settings/skills/greet").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["name"], "greet");
        assert!(json["body"].as_str().unwrap().contains("Say hello!"));
        assert!(json["raw_content"].as_str().unwrap().contains("Say hello!"));

        // Update
        let (status, json) = put_json(
            Arc::clone(&state),
            "/api/settings/skills/greet",
            serde_json::json!({ "description": "Updated desc", "body": "Say hi!" }),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");

        // Verify update
        let (_, json) = get_json(Arc::clone(&state), "/api/settings/skills/greet").await;
        assert_eq!(json["description"], "Updated desc");
        assert!(json["body"].as_str().unwrap().contains("Say hi!"));

        // Delete
        let status = delete_req(Arc::clone(&state), "/api/settings/skills/greet").await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // Verify deleted
        let (_, json) = get_json(state, "/api/settings/skills").await;
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn create_skill_duplicate_rejected() {
        let (_tmp, state) = test_app_state().await;

        let _ = post_json(
            Arc::clone(&state),
            "/api/settings/skills",
            serde_json::json!({ "name": "dup", "description": "", "body": "" }),
        ).await;

        let (status, json) = post_json(
            state,
            "/api/settings/skills",
            serde_json::json!({ "name": "dup", "description": "", "body": "" }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("already exists"));
    }

    #[tokio::test]
    async fn get_skill_not_found() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = get_json(state, "/api/settings/skills/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_skill_not_found() {
        let (_tmp, state) = test_app_state().await;
        let status = delete_req(state, "/api/settings/skills/nonexistent").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_skill_not_found() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = put_json(
            state,
            "/api/settings/skills/nonexistent",
            serde_json::json!({ "description": "x", "body": "y" }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("does not exist"));
    }

    #[tokio::test]
    async fn create_skill_invalid_name_rejected() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = post_json(
            state,
            "/api/settings/skills",
            serde_json::json!({ "name": "../evil", "description": "", "body": "" }),
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().to_lowercase().contains("invalid"));
    }

    #[test]
    fn skill_info_serializes() {
        let info = SkillInfo {
            name: "test-skill".into(),
            description: "A test".into(),
            version: Some("0.1.0".into()),
            created_at: "2026-03-20T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"test-skill\""));
        assert!(json.contains("\"version\":\"0.1.0\""));
    }

    #[test]
    fn skill_detail_serializes() {
        let detail = SkillDetail {
            name: "test".into(),
            description: "desc".into(),
            version: None,
            body: "body content".into(),
            raw_content: "---\nname: test\n---\nbody content".into(),
            created_at: "2026-03-20T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(json.contains("\"body\":\"body content\""));
        assert!(json.contains("\"version\":null"));
    }

    #[test]
    fn generate_skill_request_deserializes() {
        // All fields
        let req: GenerateSkillRequest = serde_json::from_value(serde_json::json!({
            "name": "my-skill",
            "description": "Do stuff",
            "prompt": "Extra context"
        })).unwrap();
        assert_eq!(req.name, "my-skill");
        assert_eq!(req.description.as_deref(), Some("Do stuff"));
        assert_eq!(req.prompt.as_deref(), Some("Extra context"));

        // Only required field
        let req: GenerateSkillRequest = serde_json::from_value(serde_json::json!({
            "name": "minimal"
        })).unwrap();
        assert_eq!(req.name, "minimal");
        assert!(req.description.is_none());
        assert!(req.prompt.is_none());
    }

    #[test]
    fn generate_skill_response_serializes() {
        let resp = GenerateSkillResponse {
            description: "A skill description".into(),
            body: "# Instructions\nDo things.".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"description\":\"A skill description\""));
        assert!(json.contains("\"body\":\"# Instructions\\nDo things.\""));
    }

    #[test]
    fn strip_json_fence_works() {
        // Plain JSON
        assert_eq!(strip_json_fence(r#"{"a": 1}"#), r#"{"a": 1}"#);
        // With ```json fence
        assert_eq!(strip_json_fence("```json\n{\"a\": 1}\n```"), "{\"a\": 1}");
        // With bare ``` fence
        assert_eq!(strip_json_fence("```\n{\"a\": 1}\n```"), "{\"a\": 1}");
        // With whitespace
        assert_eq!(strip_json_fence("  ```json\n{\"a\": 1}\n```  "), "{\"a\": 1}");
    }

    // ── TOML preservation tests ─────────────────────────────────────────

    #[tokio::test]
    async fn put_general_preserves_other_sections() {
        let (_tmp, state) = test_app_state().await;

        // Write a TOML with extra sections
        std::fs::write(
            &state.paths.agent_toml,
            "model = \"old\"\nagent_name = \"Old\"\n\n[memory]\nhalf_life_days = 7.0\n",
        ).unwrap();

        // Update general only
        let _ = put_json(
            Arc::clone(&state),
            "/api/settings/general",
            serde_json::json!({
                "provider": "anthropic", "model": "new-model", "max_turns": 10,
                "max_tokens": 4096, "agent_name": "New", "server_addr": "x"
            }),
        ).await;

        // [memory] section should be preserved
        let content = std::fs::read_to_string(&state.paths.agent_toml).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(parsed["model"].as_str(), Some("new-model"));
        assert_eq!(parsed["memory"]["half_life_days"].as_float(), Some(7.0));
    }

    // ── API key auth test ───────────────────────────────────────────────

    #[tokio::test]
    async fn settings_require_api_key_when_users_exist() {
        let (_tmp, state) = test_app_state().await;

        // Create an admin user and API key
        let admin = state.auth.create_user(None, Some("Admin"), starpod_auth::Role::Admin).await.unwrap();
        let created = state.auth.create_api_key(&admin.id, Some("test key")).await.unwrap();

        // Request without key → 401
        let (status, _) = get_json(Arc::clone(&state), "/api/settings/general").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Request with correct key → 200
        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/settings/general")
            .header("x-api-key", &created.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn settings_reject_non_admin_users() {
        let (_tmp, state) = test_app_state().await;

        // Create a regular user and API key
        let user = state.auth.create_user(None, Some("User"), starpod_auth::Role::User).await.unwrap();
        let created = state.auth.create_api_key(&user.id, None).await.unwrap();

        // Request with non-admin key → 403
        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/settings/general")
            .header("x-api-key", &created.key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ── Channels ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_channels_returns_defaults() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/channels").await;
        assert_eq!(status, StatusCode::OK);
        // No [channels.telegram] in test config → defaults (enabled: false by default)
        assert_eq!(json["telegram"]["enabled"], false);
        assert_eq!(json["telegram"]["gap_minutes"], 360);
        assert_eq!(json["telegram"]["stream_mode"], "final_only");
    }

    #[tokio::test]
    async fn put_channels_updates_toml() {
        let (_tmp, state) = test_app_state().await;
        let body = serde_json::json!({
            "telegram": {
                "enabled": true,
                "gap_minutes": 120,
                "stream_mode": "all_messages"
            }
        });
        let (status, _) = put_json(Arc::clone(&state), "/api/settings/channels", body).await;
        assert_eq!(status, StatusCode::OK);

        // Verify it was written to the file
        let content = std::fs::read_to_string(&state.paths.agent_toml).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(parsed["channels"]["telegram"]["enabled"].as_bool(), Some(true));
        assert_eq!(parsed["channels"]["telegram"]["gap_minutes"].as_integer(), Some(120));
        assert_eq!(parsed["channels"]["telegram"]["stream_mode"].as_str(), Some("all_messages"));
    }

    #[tokio::test]
    async fn put_channels_preserves_other_sections() {
        let (_tmp, state) = test_app_state().await;
        // Verify existing config is preserved after writing channels
        let (_, before) = get_json(Arc::clone(&state), "/api/settings/general").await;
        let body = serde_json::json!({
            "telegram": { "enabled": true, "gap_minutes": 60, "stream_mode": "final_only" }
        });
        put_json(Arc::clone(&state), "/api/settings/channels", body).await;
        let (_, after) = get_json(state, "/api/settings/general").await;
        assert_eq!(before["model"], after["model"]);
        assert_eq!(before["agent_name"], after["agent_name"]);
    }

    // ── Telegram linking ────────────────────────────────────────────────

    /// Helper: make an authenticated GET request with API key header.
    async fn get_json_authed(state: Arc<AppState>, path: &str, key: &str) -> (StatusCode, serde_json::Value) {
        let app = build_router(state);
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("x-api-key", key)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        (status, json)
    }

    /// Helper: make an authenticated PUT request with API key header.
    async fn put_json_authed(state: Arc<AppState>, path: &str, key: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
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

    /// Helper: make an authenticated DELETE request with API key header.
    async fn delete_authed(state: Arc<AppState>, path: &str, key: &str) -> StatusCode {
        let app = build_router(state);
        let req = Request::builder()
            .method("DELETE")
            .uri(path)
            .header("x-api-key", key)
            .body(Body::empty())
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn telegram_link_crud() {
        let (_tmp, state) = test_app_state().await;
        // Create an admin user with API key (this enables auth)
        let admin = state.auth.create_user(None, Some("Admin"), starpod_auth::Role::Admin).await.unwrap();
        let admin_key = state.auth.create_api_key(&admin.id, None).await.unwrap();
        let key = &admin_key.key;
        // Create a regular user to link
        let user = state.auth.create_user(None, Some("Alice"), starpod_auth::Role::User).await.unwrap();
        let uid = user.id.clone();

        // GET: no link yet
        let (status, json) = get_json_authed(Arc::clone(&state), &format!("/api/settings/auth/users/{}/telegram", uid), key).await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.get("telegram_id").is_none(), "No link initially");

        // PUT: link telegram
        let (status, json) = put_json_authed(
            Arc::clone(&state),
            &format!("/api/settings/auth/users/{}/telegram", uid),
            key,
            serde_json::json!({ "telegram_id": 12345, "username": "alice_tg" }),
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["telegram_id"], 12345);
        assert_eq!(json["username"], "alice_tg");

        // GET: link exists
        let (status, json) = get_json_authed(Arc::clone(&state), &format!("/api/settings/auth/users/{}/telegram", uid), key).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["telegram_id"], 12345);

        // DELETE: unlink
        let status = delete_authed(Arc::clone(&state), &format!("/api/settings/auth/users/{}/telegram", uid), key).await;
        assert_eq!(status, StatusCode::OK);

        // GET: link gone
        let (_, json) = get_json_authed(state, &format!("/api/settings/auth/users/{}/telegram", uid), key).await;
        assert!(json.get("telegram_id").is_none());
    }

    #[tokio::test]
    async fn telegram_link_nonexistent_user() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = put_json(
            state,
            "/api/settings/auth/users/nonexistent/telegram",
            serde_json::json!({ "telegram_id": 999 }),
        ).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn channels_settings_round_trip() {
        let settings = ChannelsSettings {
            telegram: TelegramChannelSettings {
                enabled: true,
                gap_minutes: Some(120),
                stream_mode: Some("all_messages".into()),
                bot_token: None,
            },
        };
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: ChannelsSettings = serde_json::from_str(&json).unwrap();
        assert!(parsed.telegram.enabled);
        assert_eq!(parsed.telegram.gap_minutes, Some(120));
        assert_eq!(parsed.telegram.stream_mode.as_deref(), Some("all_messages"));
    }

    #[tokio::test]
    async fn channels_settings_deserializes_with_defaults() {
        let json = r#"{ "telegram": { "enabled": false } }"#;
        let parsed: ChannelsSettings = serde_json::from_str(json).unwrap();
        assert!(!parsed.telegram.enabled);
        assert_eq!(parsed.telegram.gap_minutes, None);
        assert_eq!(parsed.telegram.stream_mode, None);
    }

    // ── Compaction ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_compaction_returns_defaults() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/compaction").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["context_budget"], 160_000);
        assert_eq!(json["summary_max_tokens"], 4096);
        assert_eq!(json["min_keep_messages"], 4);
        assert_eq!(json["max_tool_result_bytes"], 50_000);
        assert_eq!(json["prune_threshold_pct"], 70);
        assert_eq!(json["prune_tool_result_max_chars"], 2_000);
        assert_eq!(json["memory_flush"], true);
    }

    #[tokio::test]
    async fn put_compaction_updates_toml() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/compaction",
            serde_json::json!({
                "context_budget": 200000,
                "summary_max_tokens": 8192,
                "min_keep_messages": 6,
                "max_tool_result_bytes": 75000,
                "prune_threshold_pct": 80,
                "prune_tool_result_max_chars": 5000,
                "memory_flush": false,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, json) = get_json(state, "/api/settings/compaction").await;
        assert_eq!(json["context_budget"], 200_000);
        assert_eq!(json["summary_max_tokens"], 8192);
        assert_eq!(json["min_keep_messages"], 6);
        assert_eq!(json["max_tool_result_bytes"], 75_000);
        assert_eq!(json["prune_threshold_pct"], 80);
        assert_eq!(json["prune_tool_result_max_chars"], 5_000);
        assert_eq!(json["memory_flush"], false);
    }

    #[tokio::test]
    async fn compaction_settings_serde_roundtrip() {
        let settings = CompactionSettings {
            context_budget: 200_000,
            summary_max_tokens: 8192,
            min_keep_messages: 6,
            max_tool_result_bytes: 75_000,
            prune_threshold_pct: 80,
            prune_tool_result_max_chars: 5_000,
            memory_flush: false,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: CompactionSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.context_budget, 200_000);
        assert_eq!(back.max_tool_result_bytes, 75_000);
        assert_eq!(back.prune_threshold_pct, 80);
        assert_eq!(back.prune_tool_result_max_chars, 5_000);
    }

    #[tokio::test]
    async fn put_compaction_preserves_other_sections() {
        let (_tmp, state) = test_app_state().await;
        let (_, before) = get_json(Arc::clone(&state), "/api/settings/general").await;
        put_json(
            Arc::clone(&state),
            "/api/settings/compaction",
            serde_json::json!({
                "context_budget": 200000,
                "summary_max_tokens": 8192,
                "min_keep_messages": 6,
                "max_tool_result_bytes": 75000,
                "prune_threshold_pct": 80,
                "prune_tool_result_max_chars": 5000,
                "memory_flush": false,
            }),
        )
        .await;
        let (_, after) = get_json(state, "/api/settings/general").await;
        assert_eq!(before["model"], after["model"]);
        assert_eq!(before["provider"], after["provider"]);
    }

    // ── Internet ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_internet_returns_defaults() {
        let (_tmp, state) = test_app_state().await;
        let (status, json) = get_json(state, "/api/settings/internet").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["enabled"], true);
        assert_eq!(json["timeout_secs"], 15);
        assert_eq!(json["max_fetch_bytes"], 2 * 1024 * 1024);
        assert_eq!(json["max_text_chars"], 50_000);
    }

    #[tokio::test]
    async fn put_internet_updates_toml() {
        let (_tmp, state) = test_app_state().await;
        let (status, _) = put_json(
            Arc::clone(&state),
            "/api/settings/internet",
            serde_json::json!({
                "enabled": false,
                "timeout_secs": 30,
                "max_fetch_bytes": 1048576,
                "max_text_chars": 25000,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, json) = get_json(state, "/api/settings/internet").await;
        assert_eq!(json["enabled"], false);
        assert_eq!(json["timeout_secs"], 30);
        assert_eq!(json["max_fetch_bytes"], 1048576);
        assert_eq!(json["max_text_chars"], 25000);
    }

    #[tokio::test]
    async fn put_internet_preserves_other_sections() {
        let (_tmp, state) = test_app_state().await;
        let (_, before) = get_json(Arc::clone(&state), "/api/settings/general").await;
        put_json(
            Arc::clone(&state),
            "/api/settings/internet",
            serde_json::json!({
                "enabled": false,
                "timeout_secs": 20,
                "max_fetch_bytes": 262144,
                "max_text_chars": 30000,
            }),
        )
        .await;
        let (_, after) = get_json(state, "/api/settings/general").await;
        assert_eq!(before["model"], after["model"]);
        assert_eq!(before["provider"], after["provider"]);
    }

    #[tokio::test]
    async fn internet_settings_serde_roundtrip() {
        let settings = InternetSettings {
            enabled: true,
            timeout_secs: 30,
            max_fetch_bytes: 1_048_576,
            max_text_chars: 50_000,
            brave_api_key: None,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: InternetSettings = serde_json::from_str(&json).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.timeout_secs, 30);
        assert_eq!(parsed.max_fetch_bytes, 1_048_576);
        assert!(parsed.brave_api_key.is_none());
    }
}
