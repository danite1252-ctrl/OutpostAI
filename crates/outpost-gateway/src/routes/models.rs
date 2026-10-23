//! Models and engines. Phase 0 only lists what Ollama already has installed.

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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EngineStatus {
    pub reachable: bool,
    pub url: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Engines {
    pub ollama: EngineStatus,
}

#[derive(Deserialize)]
struct OllamaVersion {
    version: String,
}

/// Which engines are running. ComfyUI, voice and the trainer join in later phases.
#[utoipa::path(get, path = "/api/engines", tag = "system", security(("bearer" = [])),
    responses(
        (status = 200, description = "Engine status", body = Engines),
        (status = 401, description = "Not signed in", body = ErrorBody),
    ))]
pub async fn engines(State(state): State<AppState>, _auth: Auth) -> ApiResult<Json<Engines>> {
    let base = state.config.ollama_url.trim_end_matches('/').to_string();
    let version = match state.http.get(format!("{base}/api/version")).send().await {
        Ok(resp) if resp.status().is_success() => resp.json::<OllamaVersion>().await.ok().map(|v| v.version),
        _ => None,
    };
    Ok(Json(Engines {
        ollama: EngineStatus {
            reachable: version.is_some(),
            url: base,
            version,
        },
    }))
}
