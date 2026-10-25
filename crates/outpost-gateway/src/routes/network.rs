//! Network access: whether devices other than this PC may reach the API.
//! Only app-key requests work from the network; PIN sign-in, setup and the account
//! list stay on this PC. Ollama and ComfyUI are never exposed.

use std::net::{IpAddr, SocketAddr, UdpSocket};

use axum::extract::{ConnectInfo, State};
use axum::Json;
use outpost_core::settings;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::require_loopback;
use crate::error::{ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NetworkStatus {
    /// The saved setting.
    pub lan_access: bool,
    /// Where the gateway is listening right now.
    pub listening_on: Option<String>,
    /// True when the setting differs from what is running: restart Outpost to apply.
    pub restart_required: bool,
    /// This PC's address on the local network, for other devices to use.
    pub lan_address: Option<String>,
    /// Base URL for OpenAI-compatible apps on other devices (when access is on).
    pub base_url: Option<String>,
    /// The encrypted port phones pair with, when it is running.
    pub secure_port: Option<u16>,
    /// SHA-256 of Outpost's certificate, and its three check words.
    pub fingerprint: Option<String>,
    pub words: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NetworkRequest {
    pub lan_access: bool,
}

/// This PC's LAN address. Opens no connection: a UDP "connect" only picks the
/// outgoing interface.
pub fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("10.255.255.255:1").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

async fn status(state: &AppState) -> ApiResult<NetworkStatus> {
    let lan_access = settings::get_bool(&state.pool, settings::NETWORK_ACCESS).await?;
    let listening = state.listening.get().copied();
    let open_now = listening.is_some_and(|a: SocketAddr| !a.ip().is_loopback());
    let lan = lan_ip();
    let port = listening.map(|a| a.port()).unwrap_or(state.config.bind.port());
    Ok(NetworkStatus {
        lan_access,
        listening_on: listening.map(|a| a.to_string()),
        restart_required: listening.is_some() && open_now != lan_access,
        lan_address: lan.map(|ip| ip.to_string()),
        base_url: match (open_now, lan) {
            (true, Some(ip)) => Some(format!("http://{ip}:{port}/v1")),
            _ => None,
        },
        secure_port: state.tls.get().map(|t| t.port),
        fingerprint: state.tls.get().map(|t| t.fingerprint.clone()),
        words: state
            .tls
            .get()
            .map(|t| outpost_core::pairing::words(&t.fingerprint).iter().map(|w| w.to_string()).collect())
            .unwrap_or_default(),
    })
}

/// Network access setting and what is running now.
#[utoipa::path(get, path = "/api/settings/network", tag = "system", security(("bearer" = [])),
    responses((status = 200, description = "Network access", body = NetworkStatus), (status = 401, description = "Not signed in", body = ErrorBody)))]
pub async fn get_network(State(state): State<AppState>, _auth: Auth) -> ApiResult<Json<NetworkStatus>> {
    Ok(Json(status(&state).await?))
}

/// Turn network access on or off (owner, on this PC). Takes effect when Outpost restarts.
#[utoipa::path(put, path = "/api/settings/network", tag = "system", security(("bearer" = [])),
    request_body = NetworkRequest,
    responses((status = 200, description = "Saved", body = NetworkStatus), (status = 403, description = "Not allowed", body = ErrorBody)))]
pub async fn set_network(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    auth: Auth,
    Json(req): Json<NetworkRequest>,
) -> ApiResult<Json<NetworkStatus>> {
    require_loopback(&addr, "Changing network access")?;
    auth.require_owner()?;
    settings::set_bool(&state.pool, settings::NETWORK_ACCESS, req.lan_access).await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "network_access",
        Some(if req.lan_access { "on" } else { "off" }),
    )
    .await;
    Ok(Json(status(&state).await?))
}
