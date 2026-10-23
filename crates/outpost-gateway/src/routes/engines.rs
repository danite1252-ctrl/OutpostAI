//! Install, start and stop the engines Outpost manages (Ollama, ComfyUI).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_engines::{EngineError, EngineId, EnginesStatus};

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

fn engine_error(e: EngineError) -> ApiError {
    match e {
        EngineError::Busy | EngineError::External => ApiError::conflict(e.to_string()),
        EngineError::NotInstalled => ApiError::new(StatusCode::BAD_REQUEST, "invalid", e.to_string()),
    }
}

/// Engines, their state and install progress, plus the graphics cards found.
#[utoipa::path(get, path = "/api/engines", tag = "engines", security(("bearer" = [])),
    responses(
        (status = 200, description = "Engine status", body = EnginesStatus),
        (status = 401, description = "Not signed in", body = ErrorBody),
    ))]
pub async fn status(State(state): State<AppState>, _auth: Auth) -> Json<EnginesStatus> {
    Json(state.engines.status().await)
}

/// Download and set up an engine (owner only). Runs in the background; follow it
/// with `GET /api/engines`. Reinstalls if it is already installed; models are kept.
#[utoipa::path(post, path = "/api/engines/{id}/install", tag = "engines", security(("bearer" = [])),
    params(("id" = EngineId, Path, description = "`ollama` or `comfyui`")),
    responses(
        (status = 202, description = "Install started"),
        (status = 403, description = "Not allowed", body = ErrorBody),
        (status = 409, description = "Busy, or already running outside Outpost", body = ErrorBody),
    ))]
pub async fn install(State(state): State<AppState>, auth: Auth, Path(id): Path<EngineId>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    state.engines.install(id).map_err(engine_error)?;
    log(&state, &auth, "engine_install", id).await;
    Ok(StatusCode::ACCEPTED)
}

/// Start an installed engine (owner only).
#[utoipa::path(post, path = "/api/engines/{id}/start", tag = "engines", security(("bearer" = [])),
    params(("id" = EngineId, Path, description = "`ollama` or `comfyui`")),
    responses(
        (status = 202, description = "Starting"),
        (status = 400, description = "Not installed", body = ErrorBody),
        (status = 403, description = "Not allowed", body = ErrorBody),
        (status = 409, description = "Busy, or already running outside Outpost", body = ErrorBody),
    ))]
pub async fn start(State(state): State<AppState>, auth: Auth, Path(id): Path<EngineId>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    state.engines.start(id).map_err(engine_error)?;
    log(&state, &auth, "engine_start", id).await;
    Ok(StatusCode::ACCEPTED)
}

/// Stop an engine Outpost started (owner only).
#[utoipa::path(post, path = "/api/engines/{id}/stop", tag = "engines", security(("bearer" = [])),
    params(("id" = EngineId, Path, description = "`ollama` or `comfyui`")),
    responses(
        (status = 204, description = "Stopped"),
        (status = 403, description = "Not allowed", body = ErrorBody),
        (status = 409, description = "Busy, or running outside Outpost", body = ErrorBody),
    ))]
pub async fn stop(State(state): State<AppState>, auth: Auth, Path(id): Path<EngineId>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    state.engines.stop(id).map_err(engine_error)?;
    log(&state, &auth, "engine_stop", id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn log(state: &AppState, auth: &Auth, action: &str, id: EngineId) {
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        action,
        Some(id.as_str()),
    )
    .await;
}
