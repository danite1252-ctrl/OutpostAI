//! Managing webhooks: my URLs that Outpost tells when pictures, trainings or
//! characters are done.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use outpost_core::webhooks::{self, Webhook};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use crate::error::{ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WebhookList {
    pub webhooks: Vec<Webhook>,
    /// Events a webhook can ask for.
    pub events: Vec<String>,
}

/// My webhooks, and the events there are.
#[utoipa::path(get, path = "/api/webhooks", tag = "webhooks", security(("bearer" = [])),
    responses((status = 200, description = "Webhooks", body = WebhookList)))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<WebhookList>> {
    Ok(Json(WebhookList {
        webhooks: webhooks::list(&state.pool, &auth.account.id).await?,
        events: webhooks::EVENTS.iter().map(|e| e.to_string()).collect(),
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NewWebhook {
    /// Where to POST, e.g. `http://192.168.1.30:5678/webhook/outpost`.
    pub url: String,
    /// Events to send: `job.finished`, `training.finished`, `character.created`.
    pub events: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreatedWebhook {
    pub webhook: Webhook,
    /// For checking `X-Outpost-Signature`. Shown only now.
    pub secret: String,
}

/// Add a webhook (desktop app, or a phone with the PIN). Its secret is shown once.
#[utoipa::path(post, path = "/api/webhooks", tag = "webhooks", security(("bearer" = [])),
    request_body = NewWebhook,
    responses((status = 201, description = "Made", body = CreatedWebhook), (status = 400, description = "Not accepted", body = ErrorBody), (status = 403, description = "App keys can't", body = ErrorBody)))]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Json(req): Json<NewWebhook>,
) -> ApiResult<(StatusCode, Json<CreatedWebhook>)> {
    // Webhooks send pictures' details out of Outpost: a person sets them up.
    auth.require_fresh_pin(&state, &headers).await?;
    let (webhook, secret) = webhooks::create(&state.pool, &auth.account.id, &req.url, &req.events).await?;
    outpost_core::audit::log_quiet(&state.pool, Some(&auth.account.id), Some(&auth.session.id), "webhook_create", Some(&webhook.url)).await;
    Ok((StatusCode::CREATED, Json(CreatedWebhook { webhook, secret })))
}

/// Remove one of my webhooks.
#[utoipa::path(delete, path = "/api/webhooks/{id}", tag = "webhooks", security(("bearer" = [])),
    params(("id" = String, Path, description = "Webhook id")),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    webhooks::delete(&state.pool, &auth.account.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TestResult {
    /// HTTP status it answered with; 0 if it couldn't be reached.
    pub status: i64,
    pub error: Option<String>,
}

/// Send a `ping` event now, once, and say how it went.
#[utoipa::path(post, path = "/api/webhooks/{id}/test", tag = "webhooks", security(("bearer" = [])),
    params(("id" = String, Path, description = "Webhook id")),
    responses((status = 200, description = "Sent", body = TestResult), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn test(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<TestResult>> {
    let hook = webhooks::get(&state.pool, &auth.account.id, &id).await?;
    let message = crate::webhooks::envelope("ping", json!({ "message": "Hello from Outpost." }));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| crate::error::ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let (status, error) = crate::webhooks::post_once(&client, &hook, &message).await;
    webhooks::record(&state.pool, &hook.id, status, error.as_deref()).await?;
    Ok(Json(TestResult { status, error }))
}
