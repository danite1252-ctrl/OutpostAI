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
    /// From another device: the setup code the server shows in its log (and in
    /// `setup-code.txt` in its data folder). Not needed on the PC Outpost runs on.
    #[serde(default)]
    pub setup_code: Option<String>,
    /// From another device: what to call it in the paired devices list.
    #[serde(default)]
    pub device: Option<String>,
}

/// Create the first owner account and sign it in. Works once: on the PC Outpost runs on,
/// or from another device with the server's one-time setup code (a server with no screen).
#[utoipa::path(post, path = "/api/setup/owner", tag = "setup",
    request_body = SetupOwnerRequest,
    responses(
        (status = 201, description = "Owner created and signed in", body = SignInResponse),
        (status = 400, description = "Name or PIN not accepted", body = ErrorBody),
        (status = 403, description = "Not on this PC, and no valid setup code", body = ErrorBody),
        (status = 409, description = "Outpost already has an owner", body = ErrorBody),
    ))]
pub async fn create_owner(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<SetupOwnerRequest>,
) -> ApiResult<(StatusCode, Json<SignInResponse>)> {
    let local = require_loopback(&addr, "Setting up Outpost").is_ok();
    let _guard = state.setup_lock.lock().await;
    if accounts::owner_exists(&state.pool).await? {
        return Err(ApiError::conflict("Outpost is already set up."));
    }
    if !local {
        // Check the PIN's shape first, so a typo there doesn't use up the code.
        outpost_core::secret::validate_secret(&req.secret)?;
        let code = req.setup_code.as_deref().unwrap_or("");
        if !state.setup.take(&state, code).await {
            return Err(ApiError::forbidden(
                "That setup code isn't right, or it ran out. Look for the current one on the server: \
                 in its log, or in setup-code.txt in its data folder.",
            ));
        }
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
    let signed_in = if local {
        auth::sign_in(&state.pool, &owner.id, &secret, SessionKind::Desktop, "Outpost desktop").await?
    } else {
        // Another device: it becomes a paired device that stays signed in, like a phone.
        let label = req.device.as_deref().map(str::trim).filter(|d| !d.is_empty()).unwrap_or("Paired device");
        let label: String = label.chars().take(60).collect();
        auth::sign_in(&state.pool, &owner.id, &secret, SessionKind::Device, &label).await?
    };
    state.setup.refresh(&state).await;
    Ok((StatusCode::CREATED, Json(signed_in.into())))
}
