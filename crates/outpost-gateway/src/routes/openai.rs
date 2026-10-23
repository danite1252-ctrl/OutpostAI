//! OpenAI-compatible chat for apps like Hermes, SillyTavern and Open WebUI.
//! Requests go to Ollama's own OpenAI-compatible endpoint; Outpost adds personas:
//! use `persona:<slug>` as the model to get that persona's model, prompt and memory.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use outpost_core::personas;
use serde_json::{json, Value};

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::{llm, AppState};

pub const PERSONA_PREFIX: &str = "persona:";

/// Fill in a persona's model, system prompt and temperature.
async fn apply_persona(state: &AppState, auth: &Auth, body: &mut Value, slug: &str) -> Result<(), ApiError> {
    let persona = personas::get_by_slug(&state.pool, &auth.account.id, slug)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("No persona \"{slug}\" on this account. See GET /v1/models."),
            )
        })?;
    let (model, _) = llm::resolve_model(state, None, Some(&persona)).await?;
    let system = llm::system_prompt(state, &auth.account.id, Some(&persona)).await?;
    body["model"] = json!(model);
    if !system.is_empty() {
        if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
            messages.insert(0, json!({ "role": "system", "content": system }));
        }
    }
    if body.get("temperature").is_none() {
        if let Some(t) = persona.temperature {
            body["temperature"] = json!(t);
        }
    }
    Ok(())
}

/// Chat completions, OpenAI format, streaming or not. `model` may be any installed
/// model or `persona:<slug>` (e.g. `persona:assistant`).
#[utoipa::path(post, path = "/v1/chat/completions", tag = "openai", security(("bearer" = [])),
    request_body(content = Object, description = "OpenAI chat completion request"),
    responses(
        (status = 200, description = "OpenAI chat completion (JSON, or SSE when stream is true)", body = Object),
        (status = 400, description = "Missing model or messages", body = ErrorBody),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 404, description = "Unknown persona", body = ErrorBody),
        (status = 503, description = "Ollama isn't running", body = ErrorBody),
    ))]
pub async fn chat_completions(
    State(state): State<AppState>,
    auth: Auth,
    Json(mut body): Json<Value>,
) -> ApiResult<Response> {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid", "Add a \"model\"."))?;
    if !body.get("messages").is_some_and(Value::is_array) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", "Add \"messages\"."));
    }
    if let Some(slug) = model.strip_prefix(PERSONA_PREFIX) {
        apply_persona(&state, &auth, &mut body, slug).await?;
    }

    let url = format!("{}/v1/chat/completions", state.config.ollama_url.trim_end_matches('/'));
    let resp = state.llm.post(&url).json(&body).send().await.map_err(|_| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ollama_offline",
            "Ollama isn't running. Start it in Settings → Engines.",
        )
    })?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from_stream(resp.bytes_stream()))
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))
}
