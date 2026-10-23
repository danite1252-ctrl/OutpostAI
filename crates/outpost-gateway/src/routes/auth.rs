use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts::{self, Account, Role};
use outpost_core::sessions::{self, Session, SessionKind};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{require_loopback, SignInResponse};
use crate::error::{ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// One tile on the "Who's using Outpost?" screen.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DirectoryEntry {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub avatar_color: String,
}

/// The accounts people can sign in to (service accounts are left out). This PC only,
/// so the household's names aren't listed to the whole network.
#[utoipa::path(get, path = "/api/accounts/directory", tag = "auth",
    responses((status = 200, description = "Accounts for the sign-in screen", body = [DirectoryEntry]),
        (status = 403, description = "Asked from another device", body = ErrorBody)))]
pub async fn directory(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> ApiResult<Json<Vec<DirectoryEntry>>> {
    require_loopback(&addr, "The account list")?;
    let entries = accounts::list(&state.pool)
        .await?
        .into_iter()
        .filter(|a| a.role != Role::Service)
        .map(|a| DirectoryEntry {
            id: a.id,
            name: a.name,
            role: a.role,
            avatar_color: a.avatar_color,
        })
        .collect();
    Ok(Json(entries))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SignInRequest {
    pub account_id: String,
    pub secret: String,
    /// Shown in the account's session list. Defaults to "Outpost desktop".
    pub label: Option<String>,
}

/// Sign in on this PC's desktop app with an account PIN or password.
/// Phones will use pairing instead (phase 3).
#[utoipa::path(post, path = "/api/auth/sign-in", tag = "auth",
    request_body = SignInRequest,
    responses(
        (status = 200, description = "Signed in", body = SignInResponse),
        (status = 401, description = "Wrong PIN or password; see `attempts_left`", body = ErrorBody),
        (status = 403, description = "Not allowed (service account, or not on this PC)", body = ErrorBody),
        (status = 404, description = "No such account", body = ErrorBody),
        (status = 429, description = "Locked after too many attempts; see `retry_after`", body = ErrorBody),
    ))]
pub async fn sign_in(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<SignInRequest>,
) -> ApiResult<Json<SignInResponse>> {
    require_loopback(&addr, "Desktop sign-in")?;
    let label = req.label.as_deref().unwrap_or("Outpost desktop");
    let signed_in =
        outpost_core::auth::sign_in(&state.pool, &req.account_id, &req.secret, SessionKind::Desktop, label).await?;
    Ok(Json(signed_in.into()))
}

/// End the current session. The token stops working immediately.
#[utoipa::path(post, path = "/api/auth/sign-out", tag = "auth", security(("bearer" = [])),
    responses((status = 204, description = "Signed out"), (status = 401, description = "Not signed in or wrong PIN", body = ErrorBody)))]
pub async fn sign_out(State(state): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    sessions::revoke(&state.pool, &auth.session.id).await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "sign_out",
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Me {
    pub account: Account,
    pub session: Session,
}

/// Who am I signed in as?
#[utoipa::path(get, path = "/api/me", tag = "auth", security(("bearer" = [])),
    responses((status = 200, description = "OK", body = Me), (status = 401, description = "Not signed in or wrong PIN", body = ErrorBody)))]
pub async fn me(auth: Auth) -> Json<Me> {
    Json(Me {
        account: auth.account,
        session: auth.session,
    })
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChangeSecretRequest {
    pub current_secret: String,
    pub new_secret: String,
}

/// Change my own PIN or password. Signs out all of my sessions, including this one.
#[utoipa::path(put, path = "/api/me/secret", tag = "auth", security(("bearer" = [])),
    request_body = ChangeSecretRequest,
    responses(
        (status = 204, description = "Changed; sign in again"),
        (status = 400, description = "Not accepted", body = ErrorBody), (status = 401, description = "Not signed in or wrong PIN", body = ErrorBody), (status = 429, description = "Locked, try later", body = ErrorBody),
    ))]
pub async fn change_my_secret(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<ChangeSecretRequest>,
) -> ApiResult<StatusCode> {
    outpost_core::auth::check_secret(&state.pool, &auth.account.id, &req.current_secret, "change_secret").await?;
    accounts::set_secret(&state.pool, &auth.account.id, req.new_secret).await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "secret_changed",
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
