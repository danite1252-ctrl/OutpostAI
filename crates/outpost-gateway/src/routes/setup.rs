use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts::{self, NewAccount, Role};
use outpost_core::auth;
use outpost_core::sessions::SessionKind;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{require_loopback, SignInResponse};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetupOwnerRequest {
    pub name: String,
    /// A 4–12 digit PIN, or a password of 6+ characters.
    pub secret: String,
}

/// Create the first owner account and sign it in on this desktop.
/// Only works once, and only from the PC Outpost runs on.
#[utoipa::path(post, path = "/api/setup/owner", tag = "setup",
    request_body = SetupOwnerRequest,
    responses(
        (status = 201, description = "Owner created and signed in", body = SignInResponse),
        (status = 400, description = "Name or PIN not accepted", body = ErrorBody),
        (status = 403, description = "Not called from this PC", body = ErrorBody),
        (status = 409, description = "Outpost already has an owner", body = ErrorBody),
    ))]
pub async fn create_owner(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<SetupOwnerRequest>,
) -> ApiResult<(StatusCode, Json<SignInResponse>)> {
    require_loopback(&addr, "Setting up Outpost")?;
    let _guard = state.setup_lock.lock().await;
    if accounts::owner_exists(&state.pool).await? {
        return Err(ApiError::conflict("Outpost is already set up."));
    }
    let secret = req.secret.clone();
    let owner = accounts::create(
        &state.pool,
        NewAccount {
            name: req.name,
            role: Role::Owner,
            secret: Some(req.secret),
        },
    )
    .await?;
    outpost_core::audit::log_quiet(&state.pool, Some(&owner.id), None, "setup_owner", None).await;
    let signed_in = auth::sign_in(&state.pool, &owner.id, &secret, SessionKind::Desktop, "Outpost desktop").await?;
    Ok((StatusCode::CREATED, Json(signed_in.into())))
}
