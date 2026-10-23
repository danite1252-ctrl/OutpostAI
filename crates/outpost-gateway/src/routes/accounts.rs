use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts::{self, Account, NewAccount, Role};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// Every account on this Outpost (owner only).
#[utoipa::path(get, path = "/api/accounts", tag = "accounts", security(("bearer" = [])),
    responses((status = 200, description = "OK", body = [Account]), (status = 403, description = "Not allowed", body = ErrorBody)))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<Account>>> {
    auth.require_owner()?;
    Ok(Json(accounts::list(&state.pool).await?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateAccountRequest {
    pub name: String,
    pub role: Role,
    /// Required for every role except `service`.
    pub secret: Option<String>,
}

/// Add an account (owner only).
#[utoipa::path(post, path = "/api/accounts", tag = "accounts", security(("bearer" = [])),
    request_body = CreateAccountRequest,
    responses(
        (status = 201, description = "Created", body = Account),
        (status = 400, description = "Not accepted", body = ErrorBody), (status = 403, description = "Not allowed", body = ErrorBody), (status = 409, description = "Already exists", body = ErrorBody),
    ))]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateAccountRequest>,
) -> ApiResult<(StatusCode, Json<Account>)> {
    auth.require_owner()?;
    let account = accounts::create(
        &state.pool,
        NewAccount {
            name: req.name,
            role: req.role,
            secret: req.secret,
        },
    )
    .await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "account_created",
        Some(&account.id),
    )
    .await;
    Ok((StatusCode::CREATED, Json(account)))
}

/// Delete an account and everything it signed in with (owner only).
/// You cannot delete the account you are using, or the last owner.
#[utoipa::path(delete, path = "/api/accounts/{id}", tag = "accounts", security(("bearer" = [])),
    params(("id" = String, Path, description = "Account id")),
    responses((status = 204, description = "Done"), (status = 403, description = "Not allowed", body = ErrorBody), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    if id == auth.account.id {
        return Err(ApiError::forbidden(
            "You can't delete the account you're signed in with.",
        ));
    }
    accounts::delete(&state.pool, &id).await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "account_deleted",
        Some(&id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ResetSecretRequest {
    pub secret: String,
}

/// Set a new PIN or password for someone who forgot theirs (owner only).
/// Signs that account out everywhere.
#[utoipa::path(put, path = "/api/accounts/{id}/secret", tag = "accounts", security(("bearer" = [])),
    params(("id" = String, Path, description = "Account id")),
    request_body = ResetSecretRequest,
    responses((status = 204, description = "Done"), (status = 400, description = "Not accepted", body = ErrorBody), (status = 403, description = "Not allowed", body = ErrorBody), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn reset_secret(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<ResetSecretRequest>,
) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    accounts::set_secret(&state.pool, &id, req.secret).await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "secret_reset",
        Some(&id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
