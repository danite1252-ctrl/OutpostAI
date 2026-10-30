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
    /// For an invite: who it's for.
    #[serde(default)]
    pub invite: Option<InviteInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InviteInfo {
    /// `join` (a new account) or `reset` (a new PIN for an existing one).
    pub kind: String,
    /// The person's name.
    pub name: String,
    /// For a join: their role.
    pub role: Option<String>,
}

fn invite_info(row: &pairing::PairingCode) -> Option<InviteInfo> {
    Some(InviteInfo {
        kind: row.invite_kind.clone()?,
        name: row.invite_name.clone().unwrap_or_default(),
        role: row.invite_role.clone(),
    })
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Make a pairing code: in the desktop app on the Outpost PC, or on a paired device with
/// the PIN again (`X-Outpost-Pin`), e.g. a desktop app using a server. Needs network
/// access on, so the encrypted port is running.
#[utoipa::path(post, path = "/api/pairing", tag = "pairing", security(("bearer" = [])),
    responses((status = 201, description = "A new code", body = PairingView),
        (status = 403, description = "An app key, or no PIN from a paired device", body = ErrorBody),
        (status = 409, description = "Network access is off", body = ErrorBody)))]
pub async fn create_code(
    State(state): State<AppState>,
    auth: Auth,
    headers: axum::http::HeaderMap,
) -> ApiResult<(StatusCode, Json<PairingView>)> {
    auth.require_person("Pairing a device")?;
    auth.require_fresh_pin(&state, &headers).await?;
    let tls = secure_port(&state)?;
    let (code, row) = pairing::create(&state.pool, &auth.account.id).await?;
    Ok((StatusCode::CREATED, Json(view(&state, &tls, code, row).await)))
}

fn secure_port(state: &AppState) -> Result<crate::tls::TlsInfo, ApiError> {
    state.tls.get().cloned().ok_or_else(|| {
        ApiError::conflict(
            "Turn on \"Allow other devices on my network\" in Settings and restart Outpost, then pair the phone.",
        )
    })
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InviteRequest {
    /// Reset this account's PIN.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Or invite someone new with this name...
    #[serde(default)]
    pub name: Option<String>,
    /// ...and role (`member` unless said; `owner` or `restricted` too).
    #[serde(default)]
    pub role: Option<Role>,
}

/// Invite someone (owner): a code and QR they use on their own phone or PC to join as a
/// new account, or to set a new PIN for theirs (`account_id`). They pick the PIN; the
/// device is paired. Works once, for 24 hours.
#[utoipa::path(post, path = "/api/invites", tag = "pairing", security(("bearer" = [])),
    request_body = InviteRequest,
    responses((status = 201, description = "The invite", body = PairingView),
        (status = 403, description = "Not an owner, or no PIN from a paired device", body = ErrorBody),
        (status = 409, description = "Network access is off, or the name is taken", body = ErrorBody)))]
pub async fn create_invite(
    State(state): State<AppState>,
    auth: Auth,
    headers: axum::http::HeaderMap,
    Json(req): Json<InviteRequest>,
) -> ApiResult<(StatusCode, Json<PairingView>)> {
    auth.require_owner()?;
    auth.require_person("Inviting someone")?;
    auth.require_fresh_pin(&state, &headers).await?;
    let tls = secure_port(&state)?;
    let invite = match (req.account_id, req.name) {
        (Some(account_id), _) => pairing::Invite::Reset { account_id },
        (None, Some(name)) => pairing::Invite::Join { name, role: req.role.unwrap_or(Role::Member) },
        (None, None) => return Err(ApiError::bad_request("Say who the invite is for.")),
    };
    let (code, row) = pairing::create_invite(&state.pool, &auth.account.id, invite).await?;
    Ok((StatusCode::CREATED, Json(view(&state, &tls, code, row).await)))
}

/// The page a pairing QR opens on a phone without Outpost (`docs/pair/` on GitHub Pages).
pub const PAIR_PAGE: &str = "https://danite1252-ctrl.github.io/OutpostAI/pair/";

/// A code with everything a device needs to use it, and its QR.
async fn view(state: &AppState, tls: &crate::tls::TlsInfo, code: String, row: pairing::PairingCode) -> PairingView {
    let ts = if state.tailnet.get().is_some() {
        crate::tailscale::detect().await
    } else {
        None
    };
    let hosts = super::network::phone_hosts(state, ts.as_ref()).await;
    let name = super::health::machine_name();
    // A web page, so a phone camera without the app lands on "Get the Outpost app"; the
    // details go after the `#`, which browsers never send to the page's website.
    let mut link = format!(
        "{PAIR_PAGE}#v=1&name={}&hosts={}&port={}&fp={}&code={}",
        encode(&name),
        encode(&hosts.join(",")),
        tls.port,
        tls.fingerprint,
        code
    );
    if let Some(kind) = &row.invite_kind {
        link.push_str(&format!("&invite={kind}"));
    }
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
    PairingView {
        qr_svg,
        invite: invite_info(&row),
        id: row.id,
        code,
        expires_at: row.expires_at,
        words: pairing::words(&tls.fingerprint).iter().map(|w| w.to_string()).collect(),
        fingerprint: tls.fingerprint.clone(),
        port: tls.port,
        hosts,
        link,
        name,
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PairingStatus {
    pub used: bool,
    pub expired: bool,
    /// The phone it paired, once used.
    pub device: Option<String>,
    pub account_name: Option<String>,
}

/// Has my pairing code been used yet?
#[utoipa::path(get, path = "/api/pairing/{id}", tag = "pairing", security(("bearer" = [])),
    params(("id" = String, Path, description = "Pairing id")),
    responses((status = 200, description = "Status", body = PairingStatus), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn code_status(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<PairingStatus>> {
    auth.require_person("Pairing a device")?;
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
    /// The accounts the phone can sign in to (none for an invite).
    pub accounts: Vec<DirectoryEntry>,
    /// An invite: who it's for. The person then picks a PIN (`/api/pair/accept`).
    #[serde(default)]
    pub invite: Option<InviteInfo>,
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
    let code = pairing::check(&state.pool, &req.code).await?;
    if let Some(invite) = invite_info(&code) {
        return Ok(Json(HelloResponse {
            name: super::health::machine_name(),
            version: env!("CARGO_PKG_VERSION").into(),
            accounts: Vec::new(),
            invite: Some(invite),
        }));
    }
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
        invite: None,
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AcceptRequest {
    pub code: String,
    /// The PIN or password the person chose.
    pub secret: String,
    /// What to call this device, e.g. "Sam's phone".
    pub device_name: String,
}

/// Use an invite: the chosen PIN makes the account (or replaces its PIN), and this device
/// is paired to it. Encrypted port only.
#[utoipa::path(post, path = "/api/pair/accept", tag = "pairing",
    request_body = AcceptRequest,
    responses((status = 200, description = "Joined and paired", body = SignInResponse),
        (status = 400, description = "Wrong or expired code, or the PIN isn't accepted", body = ErrorBody),
        (status = 403, description = "Not over the encrypted port", body = ErrorBody)))]
pub async fn accept(
    State(state): State<AppState>,
    transport: Transport,
    Json(req): Json<AcceptRequest>,
) -> ApiResult<Json<SignInResponse>> {
    transport.require_secure()?;
    let name: String = req.device_name.trim().chars().take(60).collect();
    let signed_in = pairing::accept_invite(&state.pool, &req.code, &req.secret, &name).await?;
    tracing::info!(device = %signed_in.session.label, account = %signed_in.account.name, "used an invite");
    Ok(Json(signed_in.into()))
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
