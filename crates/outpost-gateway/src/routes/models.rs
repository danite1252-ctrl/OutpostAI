//! Models: the OpenAI-compatible list, plus hardware, catalogue and downloads.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// OpenAI-compatible model list.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModelList {
    /// Always `list`.
    pub object: String,
    pub data: Vec<ModelEntry>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModelEntry {
    /// Model id to pass as `model`, e.g. `qwen3:14b`.
    pub id: String,
    /// Always `model`.
    pub object: String,
    /// Unix seconds; 0 when unknown.
    pub created: i64,
    /// Engine that serves it, e.g. `ollama`.
    pub owned_by: String,
}

#[derive(Deserialize)]
struct OllamaTags {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
}

/// List the chat models this Outpost can use (OpenAI format).
/// Returns an empty list when Ollama is not running.
#[utoipa::path(get, path = "/v1/models", tag = "openai", security(("bearer" = [])),
    responses(
        (status = 200, description = "Models", body = ModelList),
        (status = 401, description = "Not signed in", body = ErrorBody),
    ))]
pub async fn list_models(State(state): State<AppState>, _auth: Auth) -> ApiResult<Json<ModelList>> {
    let url = format!("{}/api/tags", state.config.ollama_url.trim_end_matches('/'));
    let data = match state.http.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<OllamaTags>().await {
            Ok(tags) => tags
                .models
                .into_iter()
                .map(|m| ModelEntry {
                    id: m.name,
                    object: "model".into(),
                    created: 0,
                    owned_by: "ollama".into(),
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "could not read Ollama model list");
                Vec::new()
            }
        },
        Ok(resp) => {
            tracing::warn!(status = %resp.status(), "Ollama refused the model list");
            Vec::new()
        }
        Err(_) => Vec::new(),
    };
    Ok(Json(ModelList {
        object: "list".into(),
        data,
    }))
}

// ---------------------------------------------------------------------------
// Outpost model management: hardware, catalogue, downloads.

use axum::http::StatusCode;
use outpost_engines::catalog::{self, CatalogEntry};
use outpost_engines::hardware::{self, Hardware};
use outpost_engines::models::{InstalledModel, ModelError, PullState};

use crate::error::ApiError;

fn model_error(e: ModelError) -> ApiError {
    match e {
        ModelError::Offline => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "ollama_offline", e.to_string()),
        ModelError::Busy => ApiError::conflict(e.to_string()),
        ModelError::Invalid(_) => ApiError::new(StatusCode::BAD_REQUEST, "invalid", e.to_string()),
        ModelError::Ollama(_) => ApiError::new(StatusCode::BAD_GATEWAY, "ollama_error", e.to_string()),
    }
}

async fn measure(state: &AppState) -> Hardware {
    let gpus = state.engines.gpus().await;
    let dir = state.config.data_dir.clone();
    tokio::task::spawn_blocking(move || hardware::measure(gpus, &dir))
        .await
        .unwrap_or_else(|_| hardware::measure(Vec::new(), std::path::Path::new(".")))
}

/// Graphics cards, memory, free disk and the tier used for recommendations.
#[utoipa::path(get, path = "/api/hardware", tag = "models", security(("bearer" = [])),
    responses(
        (status = 200, description = "This PC", body = Hardware),
        (status = 401, description = "Not signed in", body = ErrorBody),
    ))]
pub async fn hardware(State(state): State<AppState>, _auth: Auth) -> Json<Hardware> {
    Json(measure(&state).await)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModelsOverview {
    pub hardware: Hardware,
    /// Curated models scored for this PC. Unfiltered Qwen models come first.
    pub catalog: Vec<CatalogEntry>,
    /// What Ollama has installed, with its own capability badges.
    pub installed: Vec<InstalledModel>,
    /// Downloads running now, plus finished or failed ones since Outpost started.
    pub pulls: Vec<PullState>,
    pub ollama_running: bool,
}

/// Everything the Models screen needs in one call.
#[utoipa::path(get, path = "/api/models", tag = "models", security(("bearer" = [])),
    responses(
        (status = 200, description = "Catalogue, installed models and downloads", body = ModelsOverview),
        (status = 401, description = "Not signed in", body = ErrorBody),
    ))]
pub async fn overview(State(state): State<AppState>, _auth: Auth) -> Json<ModelsOverview> {
    let hardware = measure(&state).await;
    let (installed, ollama_running) = match state.models.installed().await {
        Ok(list) => (list, true),
        Err(_) => (Vec::new(), false),
    };
    Json(ModelsOverview {
        catalog: catalog::build(&hardware),
        hardware,
        installed,
        pulls: state.models.pull_states(),
        ollama_running,
    })
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ModelRequest {
    /// Ollama model name, e.g. `huihui_ai/qwen3.5-abliterated:9b`.
    pub model: String,
}

/// Download a model into Ollama (owner only). Runs in the background; follow it
/// with `GET /api/models`.
#[utoipa::path(post, path = "/api/models/pull", tag = "models", security(("bearer" = [])),
    request_body = ModelRequest,
    responses(
        (status = 202, description = "Download started"),
        (status = 400, description = "Not a valid model name", body = ErrorBody),
        (status = 403, description = "Not allowed", body = ErrorBody),
        (status = 409, description = "Already downloading", body = ErrorBody),
    ))]
pub async fn pull(State(state): State<AppState>, auth: Auth, Json(req): Json<ModelRequest>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    state.models.pull(&req.model).map_err(model_error)?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "model_pull",
        Some(&req.model),
    )
    .await;
    Ok(StatusCode::ACCEPTED)
}

/// Remove a model from Ollama (owner only).
#[utoipa::path(post, path = "/api/models/remove", tag = "models", security(("bearer" = [])),
    request_body = ModelRequest,
    responses(
        (status = 204, description = "Removed"),
        (status = 400, description = "Not installed or not a valid name", body = ErrorBody),
        (status = 403, description = "Not allowed", body = ErrorBody),
        (status = 503, description = "Ollama isn't running", body = ErrorBody),
    ))]
pub async fn remove(State(state): State<AppState>, auth: Auth, Json(req): Json<ModelRequest>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    state.models.remove(&req.model).await.map_err(model_error)?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "model_remove",
        Some(&req.model),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
