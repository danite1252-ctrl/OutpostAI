//! Pairing phones, and the list of paired phones.
//!
//! 1. Someone at the Outpost PC presses "Pair a phone": `POST /api/pairing` returns a
//!    6-digit code, the certificate fingerprint, three check words and a QR link.
//! 2. The phone connects to the encrypted port, sends the code to `/api/pair/hello`
//!    and gets the account list; it shows the three words from the certificate it sees.
//! 3. The person picks their account and enters its PIN: `/api/pair/complete` returns
//!    a device token that stays signed in until revoked.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts::{self, Role};
use outpost_core::pairing;
use outpost_core::sessions::{self, Session, SessionKind};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::auth::DirectoryEntry;
use super::SignInResponse;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::{Auth, Transport};
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PairingView {
    /// Keep for checking whether it was used (`GET /api/pairing/{id}`).
    pub id: String,
    /// Six digits, shown on the PC.
    pub code: String,
    pub expires_at: i64,
    /// SHA-256 of Outpost's certificate (lowercase hex); phones pin it.
    pub fingerprint: String,
    /// The same three words appear on the phone when it reaches the real Outpost.
    pub words: Vec<String>,
    /// The encrypted port.
    pub port: u16,
    /// Addresses phones can try, best first.
    pub hosts: Vec<String>,
    /// Everything above in one link, for the QR code.
    pub link: String,
    /// The link as a QR code (SVG).
    pub qr_svg: String,
    pub name: String,
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Make a pairing code (desktop app only). Needs network access on, so the encrypted
/// port is running.
#[utoipa::path(post, path = "/api/pairing", tag = "pairing", security(("bearer" = [])),
    responses((status = 201, description = "A new code", body = PairingView),
        (status = 403, description = "Not from the desktop app", body = ErrorBody),
        (status = 409, description = "Network access is off", body = ErrorBody)))]
pub async fn create_code(State(state): State<AppState>, auth: Auth) -> ApiResult<(StatusCode, Json<PairingView>)> {
    auth.require_desktop("Pairing a phone")?;
    let tls = state.tls.get().cloned().ok_or_else(|| {
        ApiError::conflict(
            "Turn on \"Allow other devices on my network\" in Settings and restart Outpost, then pair the phone.",
        )
    })?;
    let (code, row) = pairing::create(&state.pool, &auth.account.id).await?;
    let hosts: Vec<String> = super::network::lan_ip().into_iter().map(|ip| ip.to_string()).collect();
    let name = super::health::machine_name();
    let link = format!(
        "outpost://pair?v=1&name={}&hosts={}&port={}&fp={}&code={}",
        encode(&name),
        encode(&hosts.join(",")),
        tls.port,
        tls.fingerprint,
        code
    );
    let qr_svg = qrcode::QrCode::new(link.as_bytes())
        .map(|q| {
            q.render::<qrcode::render::svg::Color>()
                .min_dimensions(264, 264)
                .quiet_zone(true)
                .dark_color(qrcode::render::svg::Color("#000000"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .build()
        })
        .unwrap_or_default();
    Ok((
        StatusCode::CREATED,
        Json(PairingView {
            qr_svg,
            id: row.id,
            code,
            expires_at: row.expires_at,
            words: pairing::words(&tls.fingerprint).iter().map(|w| w.to_string()).collect(),
            fingerprint: tls.fingerprint,
            port: tls.port,
            hosts,
            link,
            name,
        }),
    ))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PairingStatus {
    pub used: bool,
    pub expired: bool,
    /// The phone it paired, once used.
    pub device: Option<String>,
    pub account_name: Option<String>,
}

/// Has my pairing code been used yet? (desktop app only)
#[utoipa::path(get, path = "/api/pairing/{id}", tag = "pairing", security(("bearer" = [])),
    params(("id" = String, Path, description = "Pairing id")),
    responses((status = 200, description = "Status", body = PairingStatus), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn code_status(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<PairingStatus>> {
    auth.require_desktop("Pairing a phone")?;
    let row = pairing::get(&state.pool, &id).await?;
    if row.created_by != auth.account.id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "Not found."));
    }
    let (mut device, mut account_name) = (None, None);
    if let Some(sid) = &row.session_id {
        if let Ok(s) = sessions::get(&state.pool, sid).await {
            account_name = accounts::get(&state.pool, &s.account_id).await.ok().map(|a| a.name);
            device = Some(s.label);
        }
    }
    Ok(Json(PairingStatus {
        used: row.used_at.is_some(),
        expired: row.used_at.is_none() && row.expires_at <= outpost_core::now(),
        device,
        account_name,
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HelloRequest {
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HelloResponse {
    /// This Outpost's name.
    pub name: String,
    pub version: String,
    /// The accounts the phone can sign in to.
    pub accounts: Vec<DirectoryEntry>,
}

/// Step 2 on the phone: check the code and list the accounts. Encrypted port only.
#[utoipa::path(post, path = "/api/pair/hello", tag = "pairing",
    request_body = HelloRequest,
    responses((status = 200, description = "Code accepted", body = HelloResponse),
        (status = 400, description = "Wrong or expired code", body = ErrorBody),
        (status = 403, description = "Not over the encrypted port", body = ErrorBody)))]
pub async fn hello(
    State(state): State<AppState>,
    transport: Transport,
    Json(req): Json<HelloRequest>,
) -> ApiResult<Json<HelloResponse>> {
    transport.require_secure()?;
    pairing::check(&state.pool, &req.code).await?;
    let accounts = accounts::list(&state.pool)
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
    Ok(Json(HelloResponse {
        name: super::health::machine_name(),
        version: env!("CARGO_PKG_VERSION").into(),
        accounts,
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CompleteRequest {
    pub code: String,
    pub account_id: String,
    /// The account's PIN or password.
    pub secret: String,
    /// What to call this phone, e.g. "Dan's Pixel 9".
    pub device_name: String,
}

/// Step 3 on the phone: sign in with the code and the account's PIN. Returns a device
/// token. Encrypted port only.
#[utoipa::path(post, path = "/api/pair/complete", tag = "pairing",
    request_body = CompleteRequest,
    responses((status = 200, description = "Paired", body = SignInResponse),
        (status = 400, description = "Wrong or expired code", body = ErrorBody),
        (status = 401, description = "Wrong PIN; see `attempts_left`", body = ErrorBody),
        (status = 403, description = "Not over the encrypted port", body = ErrorBody),
        (status = 429, description = "Account locked; see `retry_after`", body = ErrorBody)))]
pub async fn complete(
    State(state): State<AppState>,
    transport: Transport,
    Json(req): Json<CompleteRequest>,
) -> ApiResult<Json<SignInResponse>> {
    transport.require_secure()?;
    let name: String = req.device_name.trim().chars().take(60).collect();
    let signed_in = pairing::complete(&state.pool, &req.code, &req.account_id, &req.secret, &name).await?;
    tracing::info!(device = %signed_in.session.label, account = %signed_in.account.name, "paired a phone");
    Ok(Json(signed_in.into()))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeviceView {
    #[serde(flatten)]
    pub session: Session,
    pub account_name: String,
    /// This is the session making the request.
    pub current: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DevicesQuery {
    /// Everyone's (owner only).
    pub all: Option<bool>,
}

/// Paired phones: mine, or everyone's for the owner (`?all=true`).
#[utoipa::path(get, path = "/api/devices", tag = "pairing", security(("bearer" = [])),
    params(DevicesQuery),
    responses((status = 200, description = "Paired phones", body = [DeviceView])))]
pub async fn list_devices(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<DevicesQuery>,
) -> ApiResult<Json<Vec<DeviceView>>> {
    let everyone = q.all.unwrap_or(false);
    if everyone {
        auth.require_owner()?;
    }
    let mut out = Vec::new();
    for account in accounts::list(&state.pool).await? {
        if !everyone && account.id != auth.account.id {
            continue;
        }
        for session in sessions::list_kind(&state.pool, &account.id, SessionKind::Device).await? {
            out.push(DeviceView {
                current: session.id == auth.session.id,
                session,
                account_name: account.name.clone(),
            });
        }
    }
    out.sort_by(|a, b| b.session.last_used_at.cmp(&a.session.last_used_at));
    Ok(Json(out))
}

/// Unpair a phone: mine, or anyone's for the owner. It is signed out at once.
#[utoipa::path(delete, path = "/api/devices/{id}", tag = "pairing", security(("bearer" = [])),
    params(("id" = String, Path, description = "Device session id")),
    responses((status = 204, description = "Unpaired"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn revoke_device(
    State(state): State<AppState>,
    auth: Auth,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let session = sessions::get(&state.pool, &id).await?;
    let mine = session.account_id == auth.account.id;
    if session.kind != SessionKind::Device || !(mine || auth.require_owner().is_ok()) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "Not found."));
    }
    // Unpairing someone else's phone is for a person, with the PIN from a phone.
    if !mine {
        auth.require_fresh_pin(&state, &headers).await?;
    }
    sessions::revoke(&state.pool, &id).await?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "device_revoke",
        Some(&id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
