//! Network access: whether devices other than this PC may reach the API.
//! Only app-key requests work from the network; PIN sign-in, setup and the account
//! list stay on this PC. Ollama and ComfyUI are never exposed.

use std::net::{IpAddr, SocketAddr, UdpSocket};

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::Json;
use outpost_core::sessions::SessionKind;
use outpost_core::settings;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::require_loopback;
use crate::error::{ApiError, ApiResult, ErrorBody};
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
    /// The saved "Allow my Tailscale devices" setting.
    pub tailscale_access: bool,
    /// This PC on Tailscale, when it's connected.
    pub tailscale: Option<crate::tailscale::TailscaleInfo>,
    /// Outpost is answering on Tailscale right now.
    pub tailscale_running: bool,
    /// Addresses a phone can use, best first (home network, then Tailscale).
    pub phone_hosts: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NetworkRequest {
    pub lan_access: bool,
    /// Left out: unchanged.
    #[serde(default)]
    pub tailscale_access: Option<bool>,
}

/// Where phones can reach the encrypted port, best first: the home network address,
/// then the Tailscale name and address.
pub async fn phone_hosts(state: &AppState, ts: Option<&crate::tailscale::TailscaleInfo>) -> Vec<String> {
    let mut hosts = Vec::new();
    let open_now = state.listening.get().is_some_and(|a| !a.ip().is_loopback());
    if open_now {
        if let Some(ip) = lan_ip() {
            hosts.push(ip.to_string());
        }
    }
    if state.tailnet.get().is_some() {
        if let Some(ts) = ts {
            if let Some(name) = &ts.dns_name {
                hosts.push(name.clone());
            }
            hosts.push(ts.ip.clone());
        }
    }
    hosts.dedup();
    hosts
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
    let lan_access = match settings::get(&state.pool, settings::NETWORK_ACCESS).await?.as_deref() {
        Some(v) => v == "on",
        // A server has it on until the owner turns it off.
        None => state.config.server,
    };
    let listening = state.listening.get().copied();
    let open_now = listening.is_some_and(|a: SocketAddr| !a.ip().is_loopback());
    let lan = lan_ip();
    let port = listening.map(|a| a.port()).unwrap_or(state.config.bind.port());
    let tailscale_access = match settings::get(&state.pool, settings::TAILSCALE_ACCESS).await?.as_deref() {
        Some(v) => v == "on",
        None => state.config.server,
    };
    let ts = crate::tailscale::detect().await;
    let tailscale_running = state.tailnet.get().is_some();
    let ts_differs = ts.is_some() && tailscale_access != tailscale_running;
    Ok(NetworkStatus {
        lan_access,
        listening_on: listening.map(|a| a.to_string()),
        restart_required: listening.is_some() && (open_now != lan_access || ts_differs),
        phone_hosts: phone_hosts(state, ts.as_ref()).await,
        tailscale_access,
        tailscale: ts,
        tailscale_running,
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

/// Turn network access on or off (owner). Takes effect when Outpost restarts. On the
/// Outpost PC, or from an owner's paired device with the PIN again (`X-Outpost-Pin`); a
/// paired device can't turn network access off, since that would cut it off.
#[utoipa::path(put, path = "/api/settings/network", tag = "system", security(("bearer" = [])),
    request_body = NetworkRequest,
    responses((status = 200, description = "Saved", body = NetworkStatus), (status = 403, description = "Not allowed", body = ErrorBody)))]
pub async fn set_network(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    auth: Auth,
    headers: HeaderMap,
    Json(req): Json<NetworkRequest>,
) -> ApiResult<Json<NetworkStatus>> {
    auth.require_owner()?;
    if require_loopback(&addr, "Changing network access").is_err() {
        if auth.session.kind != SessionKind::Device {
            return Err(ApiError::forbidden(
                "Network access is changed on the Outpost PC, or from an owner's paired device.",
            ));
        }
        auth.require_fresh_pin(&state, &headers).await?;
        if !req.lan_access {
            return Err(ApiError::forbidden(
                "Turning network access off from another device would cut that device off. \
                 Do it on the server itself.",
            ));
        }
    }
    settings::set_bool(&state.pool, settings::NETWORK_ACCESS, req.lan_access).await?;
    if let Some(on) = req.tailscale_access {
        settings::set_bool(&state.pool, settings::TAILSCALE_ACCESS, on).await?;
    }
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "network_access",
        Some(&format!(
            "lan {}, tailscale {}",
            if req.lan_access { "on" } else { "off" },
            match req.tailscale_access {
                Some(true) => "on",
                Some(false) => "off",
                None => "unchanged",
            }
        )),
    )
    .await;
    Ok(Json(status(&state).await?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ConnectionInfo {
    /// Addresses for the encrypted port, best first.
    pub hosts: Vec<String>,
    pub port: Option<u16>,
}

/// Where this Outpost can be reached now. Paired phones ask for it to learn new
/// addresses (e.g. Tailscale) without pairing again.
#[utoipa::path(get, path = "/api/connection", tag = "pairing", security(("bearer" = [])),
    responses((status = 200, description = "Addresses", body = ConnectionInfo)))]
pub async fn connection(State(state): State<AppState>, _auth: Auth) -> ApiResult<Json<ConnectionInfo>> {
    let ts = if state.tailnet.get().is_some() {
        crate::tailscale::detect().await
    } else {
        None
    };
    Ok(Json(ConnectionInfo {
        hosts: phone_hosts(&state, ts.as_ref()).await,
        port: state.tls.get().map(|t| t.port),
    }))
}
