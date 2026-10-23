//! App keys: long-lived bearer tokens for apps like Hermes. A key acts as the
//! account it belongs to, so its personas and memory are that account's.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts;
use outpost_core::sessions::{self, Session, SessionKind};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::SignInResponse;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KeysQuery {
    /// Another account's keys (owner only), e.g. a service account for Hermes.
    pub account_id: Option<String>,
}

fn target(auth: &Auth, account_id: Option<String>) -> Result<String, ApiError> {
    match account_id {
        Some(id) if id != auth.account.id => {
            auth.require_owner()?;
            Ok(id)
        }
        _ => Ok(auth.account.id.clone()),
    }
}

/// App keys for my account, or for another account (owner only).
#[utoipa::path(get, path = "/api/keys", tag = "keys", security(("bearer" = [])),
    params(KeysQuery),
    responses((status = 200, description = "Keys (tokens are never shown again)", body = [Session]), (status = 403, description = "Not allowed", body = ErrorBody)))]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<KeysQuery>,
) -> ApiResult<Json<Vec<Session>>> {
    let account_id = target(&auth, q.account_id)?;
    Ok(Json(
        sessions::list_kind(&state.pool, &account_id, SessionKind::App).await?,
    ))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NewKeyRequest {
    /// What uses it, e.g. "Hermes on the NAS".
    pub label: String,
    /// Make the key for another account (owner only), e.g. a service account.
    pub account_id: Option<String>,
}

/// Create an app key. The token is in the response once; store it in the app.
#[utoipa::path(post, path = "/api/keys", tag = "keys", security(("bearer" = [])),
    request_body = NewKeyRequest,
    responses((status = 201, description = "Created", body = SignInResponse), (status = 403, description = "Not allowed", body = ErrorBody), (status = 404, description = "No such account", body = ErrorBody)))]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<NewKeyRequest>,
) -> ApiResult<(StatusCode, Json<SignInResponse>)> {
    let account_id = target(&auth, req.account_id)?;
    let account = accounts::get(&state.pool, &account_id).await?;
    let label = req.label.trim();
    if label.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid",
            "Name the key after the app that uses it.",
        ));
    }
    let (token, session) = sessions::create(&state.pool, &account.id, SessionKind::App, label).await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "key_created",
        Some(&session.id),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(SignInResponse {
            token,
            account,
            session,
        }),
    ))
}

/// Revoke an app key. It stops working immediately.
#[utoipa::path(delete, path = "/api/keys/{id}", tag = "keys", security(("bearer" = [])),
    params(("id" = String, Path, description = "Key (session) id")),
    responses((status = 204, description = "Revoked"), (status = 403, description = "Not allowed", body = ErrorBody), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn revoke(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let key = sessions::get(&state.pool, &id).await?;
    if key.kind != SessionKind::App {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "Not found."));
    }
    if key.account_id != auth.account.id {
        auth.require_owner()?;
    }
    sessions::revoke(&state.pool, &key.id).await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "key_revoked",
        Some(&id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
