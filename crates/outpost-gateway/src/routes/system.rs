//! Diagnostics (logs and crash reports kept on this PC) and update checks.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::diagnostics::{self, CrashSummary};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::updates::{self, UpdateStatus};
use crate::AppState;

/// Setting: Unix seconds up to which crash reports have been looked at.
const SEEN_UNTIL: &str = "diagnostics.seen_until";

#[derive(Debug, Serialize, ToSchema)]
pub struct Diagnostics {
    /// The log folder (`outpost.log` and older ones).
    pub logs_dir: String,
    /// The crash report folder.
    pub crash_dir: String,
    /// Saved crash reports, newest first.
    pub crashes: Vec<CrashSummary>,
    /// Reports newer than the last time someone looked.
    pub unseen: usize,
}

/// Logs and crash reports on this PC. Nothing is sent anywhere. (Owner.)
#[utoipa::path(get, path = "/api/diagnostics", tag = "system", security(("bearer" = [])),
    responses((status = 200, description = "Where logs are, and crash reports", body = Diagnostics), (status = 403, body = ErrorBody)))]
pub async fn diagnostics(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Diagnostics>> {
    auth.require_owner()?;
    let data = &state.config.data_dir;
    let crashes = diagnostics::list_crashes(data);
    let seen: i64 = outpost_core::settings::get(&state.pool, SEEN_UNTIL)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    Ok(Json(Diagnostics {
        logs_dir: diagnostics::logs_dir(data).to_string_lossy().to_string(),
        crash_dir: diagnostics::crash_dir(data).to_string_lossy().to_string(),
        unseen: crashes.iter().filter(|c| c.at > seen).count(),
        crashes,
    }))
}

/// Mark every crash report as looked at.
#[utoipa::path(post, path = "/api/diagnostics/seen", tag = "system", security(("bearer" = [])),
    responses((status = 204, description = "Marked")))]
pub async fn mark_seen(State(state): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    outpost_core::settings::set(&state.pool, SEEN_UNTIL, &outpost_core::now().to_string()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CrashReport {
    pub id: String,
    /// The whole report, to read or copy.
    pub text: String,
}

/// One crash report.
#[utoipa::path(get, path = "/api/diagnostics/crashes/{id}", tag = "system", security(("bearer" = [])),
    params(("id" = String, Path)),
    responses((status = 200, body = CrashReport), (status = 404, body = ErrorBody)))]
pub async fn crash(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<CrashReport>> {
    auth.require_owner()?;
    let text = diagnostics::read_crash(&state.config.data_dir, &id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That crash report isn't there."))?;
    Ok(Json(CrashReport { id, text }))
}

/// Delete one crash report.
#[utoipa::path(delete, path = "/api/diagnostics/crashes/{id}", tag = "system", security(("bearer" = [])),
    params(("id" = String, Path)),
    responses((status = 204, description = "Deleted"), (status = 404, body = ErrorBody)))]
pub async fn delete_crash(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    if diagnostics::delete_crash(&state.config.data_dir, &id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "That crash report isn't there."))
    }
}

/// Delete every crash report.
#[utoipa::path(delete, path = "/api/diagnostics/crashes", tag = "system", security(("bearer" = [])),
    responses((status = 204, description = "Deleted")))]
pub async fn delete_all_crashes(State(state): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    for c in diagnostics::list_crashes(&state.config.data_dir) {
        diagnostics::delete_crash(&state.config.data_dir, &c.id);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecentLog {
    /// The last few hundred log lines of this run.
    pub text: String,
}

/// The latest log lines, for troubleshooting.
#[utoipa::path(get, path = "/api/diagnostics/log", tag = "system", security(("bearer" = [])),
    responses((status = 200, body = RecentLog)))]
pub async fn recent_log(auth: Auth) -> ApiResult<Json<RecentLog>> {
    auth.require_owner()?;
    Ok(Json(RecentLog { text: diagnostics::recent_log(400) }))
}

/// Whether a newer Outpost is out (from the last check), and whether checks are on.
#[utoipa::path(get, path = "/api/updates", tag = "system", security(("bearer" = [])),
    responses((status = 200, body = UpdateStatus)))]
pub async fn update_status(State(state): State<AppState>, _auth: Auth) -> Json<UpdateStatus> {
    Json(updates::status(&state).await)
}

/// Check GitHub for a newer release now. (Owner.)
#[utoipa::path(post, path = "/api/updates/check", tag = "system", security(("bearer" = [])),
    responses((status = 200, body = UpdateStatus)))]
pub async fn check_updates(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<UpdateStatus>> {
    auth.require_owner()?;
    updates::check(&state).await;
    Ok(Json(updates::status(&state).await))
}

/// Install the newest release on this server and restart (owner, with the PIN from a
/// paired device). Only a server started with `--server` updates itself; it checks the
/// download's signature first. Poll `GET /api/updates` (`installing`), then `/api/health`.
#[utoipa::path(post, path = "/api/updates/install", tag = "system", security(("bearer" = [])),
    responses((status = 202, description = "Installing"),
        (status = 403, description = "Not an owner, or no PIN", body = ErrorBody),
        (status = 409, description = "Not a server, nothing to install, or already installing", body = ErrorBody)))]
pub async fn install_server_update(
    State(state): State<AppState>,
    auth: Auth,
    headers: axum::http::HeaderMap,
) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    auth.require_person("Updating the server")?;
    auth.require_fresh_pin(&state, &headers).await?;
    updates::install_server(&state).map_err(ApiError::conflict)?;
    outpost_core::audit::log_quiet(&state.pool, Some(&auth.account.id), Some(&auth.session.id), "server_update", None).await;
    Ok(StatusCode::ACCEPTED)
}

/// Restart Outpost on this server (owner, with the PIN from a paired device), e.g. after
/// changing network access. Only a server started with `--server`; it's back in a few seconds.
#[utoipa::path(post, path = "/api/system/restart", tag = "system", security(("bearer" = [])),
    responses((status = 202, description = "Restarting"),
        (status = 403, description = "Not an owner, or no PIN", body = ErrorBody),
        (status = 409, description = "Not a server", body = ErrorBody)))]
pub async fn restart(State(state): State<AppState>, auth: Auth, headers: axum::http::HeaderMap) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    auth.require_person("Restarting the server")?;
    auth.require_fresh_pin(&state, &headers).await?;
    if !state.config.server {
        return Err(ApiError::conflict("Only a server restarts this way. On a PC, close and reopen the Outpost app."));
    }
    outpost_core::audit::log_quiet(&state.pool, Some(&auth.account.id), Some(&auth.session.id), "server_restart", None).await;
    tracing::info!(by = %auth.account.name, "restarting on request");
    let s = state.clone();
    tokio::spawn(async move { updates::restart(&s).await });
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSettings {
    /// Check GitHub for updates (at start and once a day; a server every minute).
    pub enabled: bool,
    /// A server installs updates by itself at a quiet moment. Left out: unchanged.
    #[serde(default)]
    pub auto_install: Option<bool>,
}

/// Turn update checks on or off. (Owner.)
#[utoipa::path(put, path = "/api/updates/settings", tag = "system", security(("bearer" = [])),
    request_body = UpdateSettings,
    responses((status = 200, body = UpdateStatus)))]
pub async fn update_settings(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<UpdateSettings>,
) -> ApiResult<Json<UpdateStatus>> {
    auth.require_owner()?;
    outpost_core::settings::set_bool(&state.pool, updates::UPDATES_OFF, !req.enabled).await?;
    if let Some(on) = req.auto_install {
        outpost_core::settings::set_bool(&state.pool, updates::AUTO_INSTALL, on).await?;
    }
    Ok(Json(updates::status(&state).await))
}
