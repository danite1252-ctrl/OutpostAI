//! Tools on the Outpost server, per account: run here for chats from any device (phone, any
//! PC), with each person's own keys and sign-ins (sealed by [`crate::vault`]). See
//! `outpost_core::tools` for who may use which.
//!
//! - **An address (http)**: the MCP client here talks to it, with its headers, or the
//!   person's sign-in (OAuth) as `Authorization`.
//! - **A program (stdio)**: started here (`npx`/`uvx` with Node or uv downloaded into
//!   `<data>/engines`). Owners only: it runs with this server's access.
//! - **Built in (Google)**: [`crate::google_tools`] with the person's Google sign-in.
//!
//! Signing in happens in the person's browser, which has to come back to their own device:
//! the desktop app signs in itself and hands the result here (`PUT /api/tools/{id}/sign-in`);
//! the phone has this server do the OAuth work (`sign-in/start`, then `sign-in/finish` with
//! what its browser brought back to it). This server renews sign-ins either way.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts::Role;
use outpost_core::tools::{self, Tool, ToolFields, ToolView};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::google_tools;
use crate::mcp_client::{self, CallResult, ServerConfig, ToolInfo};
use crate::mcp_oauth::{self, OAuthState};
use crate::pc_tools::{PcTool, PcToolServer};
use crate::AppState;

/// Longest to wait for one tool server's list of tools when a message is sent.
const LIST_WITHIN: Duration = Duration::from_secs(40);

/// Settings kept sealed: environment variables (programs) and headers (addresses).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Secrets {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().connect_timeout(Duration::from_secs(20)).build().expect("HTTP client")
}

fn restricted(auth: &Auth) -> bool {
    auth.account.role == Role::Restricted
}

pub fn secrets(state: &AppState, t: &Tool) -> Secrets {
    t.secrets.as_deref().and_then(|s| state.vault.unseal(s)).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn oauth(state: &AppState, t: &Tool) -> Option<OAuthState> {
    t.oauth.as_deref().and_then(|s| state.vault.unseal(s)).and_then(|s| serde_json::from_str(&s).ok())
}

fn seal_secrets(state: &AppState, t: &mut Tool, s: &Secrets) {
    let names: Vec<String> = s.env.keys().chain(s.headers.keys()).cloned().collect();
    t.secret_names = serde_json::to_string(&names).unwrap_or_else(|_| "[]".into());
    t.secrets = if names.is_empty() { None } else { Some(state.vault.seal(&serde_json::to_string(s).unwrap_or_default())) };
}

/// Keep what's given; drop names given empty; keep the rest.
fn merge(old: &mut BTreeMap<String, String>, new: BTreeMap<String, String>) -> Result<(), ApiError> {
    for (k, v) in new {
        let k = k.trim().to_string();
        if k.is_empty() || k.len() > 100 || k.chars().any(|c| c.is_control() || c == '=') {
            return Err(ApiError::bad_request(format!("\"{k}\" isn't a usable setting name.")));
        }
        if v.is_empty() {
            old.remove(&k);
        } else {
            old.insert(k, v);
        }
    }
    Ok(())
}

fn is_google(t: &Tool) -> bool {
    t.transport == "builtin" && t.command == "google"
}

/// How the MCP client reaches it (its sign-in's token over any typed-in key).
fn config(state: &AppState, t: &Tool, st: Option<&OAuthState>) -> ServerConfig {
    let s = secrets(state, t);
    let mut headers = s.headers;
    if let Some(token) = st.and_then(|o| o.access_token.as_ref()) {
        headers.retain(|k, _| !k.eq_ignore_ascii_case("authorization"));
        headers.insert("Authorization".into(), format!("Bearer {token}"));
    }
    // A program per tool (personal tools are per person already); the tool id keeps them apart.
    ServerConfig { id: t.id.clone(), name: t.name.clone(), transport: t.transport.clone(), command: t.command.clone(), args: t.args(), env: s.env, url: t.url.clone(), headers }
}

/// The sign-in, renewed if it has run out (or `force`), and kept. One renewal at a time:
/// some services (Notion) replace the refresh token each time and cut off one used twice.
async fn renewed(state: &AppState, t: &Tool, force: bool) -> Result<Option<OAuthState>, String> {
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Some(st) = oauth(state, t) else { return Ok(None) };
    if st.refresh_token.is_none() || !(force || st.needs_refresh(now())) {
        return Ok(Some(st));
    }
    let _turn = ONE_AT_A_TIME.lock().await;
    // Another call may have renewed it while this one waited.
    let fresh = tools::get_any(&state.pool, &t.id).await.map_err(|e| e.to_string())?;
    let Some(cur) = oauth(state, &fresh) else { return Ok(None) };
    if cur.access_token != st.access_token && !cur.needs_refresh(now()) {
        return Ok(Some(cur));
    }
    let next = mcp_oauth::refresh(&http(), &cur).await?;
    let sealed = state.vault.seal(&serde_json::to_string(&next).unwrap_or_default());
    tools::save_oauth(&state.pool, &t.id, Some(&sealed), fresh.signed_in_as.as_deref()).await.map_err(|e| e.to_string())?;
    Ok(Some(next))
}

fn can_renew(st: &Option<OAuthState>, e: &str) -> bool {
    mcp_client::needs_sign_in(e) && st.as_ref().is_some_and(|o| o.refresh_token.is_some())
}

/// A tool's tools (starting it if need be).
pub async fn tool_list(state: &AppState, t: &Tool) -> Result<Vec<ToolInfo>, String> {
    let st = renewed(state, t, false).await?;
    if is_google(t) {
        st.as_ref().and_then(|o| o.access_token.as_ref()).ok_or_else(|| format!("needs sign-in: sign in to {} first.", t.name))?;
        return Ok(google_tools::tools(&t.args()));
    }
    match state.tool_pool.tools(&config(state, t, st.as_ref())).await {
        Err(e) if can_renew(&st, &e) => {
            let st = renewed(state, t, true).await?;
            state.tool_pool.tools(&config(state, t, st.as_ref())).await
        }
        other => other,
    }
}

/// Run one of a tool's tools.
pub async fn call(state: &AppState, tool_id: &str, name: &str, args: Value) -> Result<CallResult, String> {
    let t = tools::get_any(&state.pool, tool_id).await.map_err(|_| "That tool isn't on the server any more.".to_string())?;
    if !t.enabled || t.disabled_tools().iter().any(|d| d == name) {
        return Ok(CallResult { text: format!("{name} is switched off."), is_error: true });
    }
    let st = renewed(state, &t, false).await?;
    if is_google(&t) {
        let token = |st: &Option<OAuthState>| st.as_ref().and_then(|o| o.access_token.clone()).ok_or_else(|| "needs sign-in: sign in to Google first.".to_string());
        return match google_tools::call(&http(), &token(&st)?, &t.args(), name, &args).await {
            Err(e) if can_renew(&st, &e) => {
                let st = renewed(state, &t, true).await?;
                google_tools::call(&http(), &token(&st)?, &t.args(), name, &args).await
            }
            other => other,
        };
    }
    match state.tool_pool.call(&config(state, &t, st.as_ref()), name, args.clone()).await {
        Err(e) if can_renew(&st, &e) => {
            let st = renewed(state, &t, true).await?;
            state.tool_pool.call(&config(state, &t, st.as_ref()), name, args).await
        }
        other => other,
    }
}

/// The tools a chat uses on this server, ready for the model (any that won't start are left
/// out, with a note in the log).
pub async fn for_chat(state: &AppState, account_id: &str, restricted: bool, chat_id: &str) -> Vec<PcToolServer> {
    let ids = tools::chat_tools(&state.pool, account_id, restricted, chat_id).await.unwrap_or_default();
    let mut rows = Vec::new();
    for id in ids {
        if let Ok(t) = tools::get_usable(&state.pool, account_id, restricted, &id).await {
            if t.enabled {
                rows.push(t);
            }
        }
    }
    let jobs = rows.into_iter().map(|t| async move {
        let off = t.disabled_tools();
        match tokio::time::timeout(LIST_WITHIN, tool_list(state, &t)).await {
            Ok(Ok(list)) => Some(PcToolServer {
                id: t.id.clone(),
                name: t.name.clone(),
                tools: list
                    .into_iter()
                    .filter(|x| !off.contains(&x.name))
                    .map(|x| PcTool { name: x.name, description: x.description, input_schema: x.input_schema, read_only: x.read_only })
                    .collect(),
            }),
            Ok(Err(e)) => {
                tracing::warn!(tool = %t.name, error = %e, "a tool on the server didn't start");
                None
            }
            Err(_) => {
                tracing::warn!(tool = %t.name, "a tool on the server took too long to start");
                None
            }
        }
    });
    futures_util::future::join_all(jobs).await.into_iter().flatten().collect()
}

fn view(state: &AppState, t: &Tool, me: &str, owner_name: Option<String>) -> ToolView {
    let st = oauth(state, t);
    ToolView {
        id: t.id.clone(),
        name: t.name.clone(),
        catalog_id: t.catalog_id.clone(),
        transport: t.transport.clone(),
        command: t.command.clone(),
        args: t.args(),
        url: t.url.clone(),
        secret_names: t.secret_names(),
        signed_in: st.as_ref().is_some_and(OAuthState::signed_in),
        signed_in_as: t.signed_in_as.clone(),
        enabled: t.enabled,
        shared: t.shared,
        restricted_ok: t.restricted_ok,
        disabled_tools: t.disabled_tools(),
        mine: t.account_id == me,
        owner_name,
    }
}

async fn owner_names(state: &AppState) -> BTreeMap<String, String> {
    outpost_core::accounts::list(&state.pool).await.unwrap_or_default().into_iter().map(|a| (a.id, a.name)).collect()
}

/// The tools you can use: yours, and the household's shared ones.
#[utoipa::path(get, path = "/api/tools", tag = "tools", security(("bearer" = [])),
    responses((status = 200, description = "Tools", body = [ToolView])))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<ToolView>>> {
    let names = owner_names(&state).await;
    let me = &auth.account.id;
    let rows = tools::usable(&state.pool, me, restricted(&auth)).await?;
    Ok(Json(
        rows.iter()
            .map(|t| view(&state, t, me, (t.account_id != *me).then(|| names.get(&t.account_id).cloned()).flatten()))
            .collect(),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ToolInput {
    #[serde(flatten)]
    pub fields: ToolFields,
    /// Settings to keep sealed: a name given empty is removed, one left out is kept.
    #[serde(default)]
    pub secrets: Option<Secrets>,
}

fn check_may(auth: &Auth, transport: &str) -> Result<(), ApiError> {
    if auth.account.role == Role::Restricted {
        return Err(ApiError::forbidden("Ask an owner to add tools for you."));
    }
    if transport == "stdio" && auth.account.role != Role::Owner {
        return Err(ApiError::forbidden("Only an owner can add tools that run as programs on the Outpost."));
    }
    Ok(())
}

/// Add a tool (yours; share it to make it everyone's).
#[utoipa::path(post, path = "/api/tools", tag = "tools", security(("bearer" = [])),
    request_body = ToolInput, responses((status = 201, description = "Added", body = ToolView), (status = 403, description = "Not allowed", body = ErrorBody)))]
pub async fn create(State(state): State<AppState>, auth: Auth, Json(input): Json<ToolInput>) -> ApiResult<(StatusCode, Json<ToolView>)> {
    auth.require_person("Adding tools")?;
    let mut t = tools::new_tool(&auth.account.id, &input.fields)?;
    check_may(&auth, &t.transport)?;
    if t.shared && auth.account.role != Role::Owner {
        t.shared = false;
    }
    let mut s = Secrets::default();
    if let Some(given) = input.secrets {
        merge(&mut s.env, given.env)?;
        merge(&mut s.headers, given.headers)?;
    }
    seal_secrets(&state, &mut t, &s);
    tools::insert(&state.pool, &t).await?;
    Ok((StatusCode::CREATED, Json(view(&state, &t, &auth.account.id, None))))
}

/// Change one of your tools (its connection restarts when how it's reached changes).
#[utoipa::path(patch, path = "/api/tools/{id}", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The tool")), request_body = ToolInput,
    responses((status = 200, description = "Changed", body = ToolView)))]
pub async fn update(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(input): Json<ToolInput>) -> ApiResult<Json<ToolView>> {
    let mut t = tools::get_own(&state.pool, &auth.account.id, &id).await?;
    let old_url = t.url.clone();
    tools::apply(&mut t, &input.fields)?;
    check_may(&auth, &t.transport)?;
    if t.shared && auth.account.role != Role::Owner {
        return Err(ApiError::forbidden("Only an owner can share a tool with everyone."));
    }
    if let Some(given) = input.secrets {
        let mut s = secrets(&state, &t);
        merge(&mut s.env, given.env)?;
        merge(&mut s.headers, given.headers)?;
        seal_secrets(&state, &mut t, &s);
    }
    // A new address: the old sign-in was for somewhere else.
    if t.url != old_url && t.transport == "http" {
        t.oauth = None;
        t.signed_in_as = None;
    }
    tools::save(&state.pool, &t).await?;
    state.tool_pool.close(&t.id).await;
    Ok(Json(view(&state, &t, &auth.account.id, None)))
}

#[utoipa::path(delete, path = "/api/tools/{id}", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The tool")), responses((status = 204, description = "Removed")))]
pub async fn remove(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    tools::delete(&state.pool, &auth.account.id, &id).await?;
    state.tool_pool.close(&id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Start a tool and list its tools (Test). An error that starts `needs sign-in:`, `needs
/// node:` or `needs uv:` says what to do first.
#[utoipa::path(get, path = "/api/tools/{id}/tools", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The tool")),
    responses((status = 200, description = "Its tools"), (status = 409, description = "It didn't start", body = ErrorBody)))]
pub async fn test(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<ToolInfo>>> {
    let t = tools::get_usable(&state.pool, &auth.account.id, restricted(&auth), &id).await?;
    tool_list(&state, &t).await.map(Json).map_err(|e| ApiError::new(StatusCode::CONFLICT, "tool_failed", e))
}

/// What a device needs to sign in to a tool for the server.
#[derive(Debug, Serialize, ToSchema)]
pub struct SignInInfo {
    /// `google` (Outpost's Google tools) or `oauth` (an MCP server's own sign-in).
    pub kind: String,
    pub url: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
    /// The last sign-in without its tokens (a registration to reuse).
    #[schema(value_type = Object)]
    pub previous: Option<OAuthState>,
}

const GOOGLE_CLIENT: &str = "tools.google_client";

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct GoogleClient {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
}

async fn google_client(state: &AppState) -> Option<GoogleClient> {
    let sealed = outpost_core::settings::get(&state.pool, GOOGLE_CLIENT).await.ok().flatten()?;
    serde_json::from_str(&state.vault.unseal(&sealed)?).ok()
}

/// How to sign in to one of your tools.
#[utoipa::path(get, path = "/api/tools/{id}/sign-in", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The tool")), responses((status = 200, description = "How", body = SignInInfo)))]
pub async fn sign_in_info(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<SignInInfo>> {
    let t = tools::get_own(&state.pool, &auth.account.id, &id).await?;
    let previous = oauth(&state, &t).map(|o| OAuthState { access_token: None, refresh_token: None, expires_at: None, ..o });
    if is_google(&t) {
        let client = google_client(&state).await;
        return Ok(Json(SignInInfo {
            kind: "google".into(),
            url: String::new(),
            client_id: client.as_ref().map(|c| c.client_id.clone()),
            client_secret: client.and_then(|c| c.client_secret),
            scope: Some(google_tools::scopes(&t.args())),
            previous,
        }));
    }
    if t.transport != "http" {
        return Err(ApiError::bad_request("Only tools at an address sign in this way."));
    }
    Ok(Json(SignInInfo { kind: "oauth".into(), url: t.url.clone(), client_id: None, client_secret: None, scope: None, previous }))
}

/// Keep a sign-in made on your device for one of your tools.
#[utoipa::path(put, path = "/api/tools/{id}/sign-in", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The tool")), responses((status = 200, description = "Signed in", body = ToolView)))]
pub async fn sign_in_save(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(st): Json<OAuthState>) -> ApiResult<Json<ToolView>> {
    let mut t = tools::get_own(&state.pool, &auth.account.id, &id).await?;
    keep_sign_in(&state, &mut t, st).await?;
    Ok(Json(view(&state, &t, &auth.account.id, None)))
}

async fn keep_sign_in(state: &AppState, t: &mut Tool, st: OAuthState) -> ApiResult<()> {
    if st.access_token.is_none() {
        return Err(ApiError::bad_request("That sign-in has no token."));
    }
    t.signed_in_as = if is_google(t) { google_tools::account_email(&http(), st.access_token.as_deref().unwrap_or("")).await } else { None };
    t.oauth = Some(state.vault.seal(&serde_json::to_string(&st).unwrap_or_default()));
    tools::save_oauth(&state.pool, &t.id, t.oauth.as_deref(), t.signed_in_as.as_deref()).await?;
    state.tool_pool.close(&t.id).await;
    Ok(())
}

/// Sign-ins started for a browser on the phone, waiting for it to come back (by their state).
static STARTED: std::sync::Mutex<Option<std::collections::HashMap<String, (String, String, mcp_oauth::Started, std::time::Instant)>>> =
    std::sync::Mutex::new(None);

fn started() -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<String, (String, String, mcp_oauth::Started, std::time::Instant)>>> {
    STARTED.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SignInStart {
    /// The port the phone listens on: the browser comes back to http://127.0.0.1:<port>/callback.
    pub port: u16,
    /// An OAuth client of your own, for a service that doesn't let apps register (or Google's,
    /// when the household hasn't one yet).
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SignInFinish {
    /// The browser's callback: its path and query (`/callback?code=…&state=…`).
    pub callback: String,
}

fn sign_in_error(e: String) -> ApiError {
    if e.starts_with("needs client:") {
        ApiError::new(StatusCode::CONFLICT, "needs_client", e)
    } else {
        ApiError::new(StatusCode::BAD_GATEWAY, "sign_in_failed", e)
    }
}

/// Start signing in to one of your tools from a device whose browser comes back to itself
/// (the phone): the server does the OAuth work and hands back the address to open.
#[utoipa::path(post, path = "/api/tools/{id}/sign-in/start", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The tool")), request_body = SignInStart,
    responses((status = 200, description = "The address to open"), (status = 409, description = "Needs an OAuth client", body = ErrorBody)))]
pub async fn sign_in_start(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(req): Json<SignInStart>) -> ApiResult<Json<Value>> {
    let t = tools::get_own(&state.pool, &auth.account.id, &id).await?;
    let previous = oauth(&state, &t);
    let given = req.client_id.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(|c| (c.to_string(), req.client_secret.clone().filter(|s| !s.trim().is_empty())));
    let begun = (if is_google(&t) {
        let (client_id, secret) = match google_client(&state).await {
            Some(c) => (c.client_id, c.client_secret),
            None => {
                let Some((id, secret)) = given else {
                    return Err(sign_in_error("needs client: an owner needs to set up the household's Google client first.".into()));
                };
                // An owner's client is kept for the household.
                if auth.account.role == Role::Owner {
                    let c = GoogleClient { client_id: id.clone(), client_secret: secret.clone() };
                    let sealed = state.vault.seal(&serde_json::to_string(&c).unwrap_or_default());
                    outpost_core::settings::set(&state.pool, GOOGLE_CLIENT, &sealed).await?;
                }
                (id, secret)
            }
        };
        mcp_oauth::start_known(mcp_oauth::google_state(&client_id, secret, &google_tools::scopes(&t.args())), req.port)
    } else if t.transport == "http" {
        mcp_oauth::start(&http(), &t.url, given, None, previous.as_ref(), req.port).await
    } else {
        return Err(ApiError::bad_request("Only tools at an address sign in this way."));
    })
    .map_err(sign_in_error)?;
    let url = begun.url.clone();
    let mut g = started();
    let map = g.get_or_insert_with(Default::default);
    map.retain(|_, (_, _, _, at)| at.elapsed() < mcp_oauth::SIGN_IN_WITHIN * 2);
    map.insert(begun.state.clone(), (auth.account.id.clone(), t.id.clone(), begun, std::time::Instant::now()));
    Ok(Json(json!({ "url": url })))
}

/// Finish a sign-in started with `sign-in/start`, with what the browser brought back.
#[utoipa::path(post, path = "/api/tools/{id}/sign-in/finish", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The tool")), request_body = SignInFinish,
    responses((status = 200, description = "Signed in", body = ToolView), (status = 400, description = "Not one we started", body = ErrorBody)))]
pub async fn sign_in_finish(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(req): Json<SignInFinish>) -> ApiResult<Json<ToolView>> {
    let mut t = tools::get_own(&state.pool, &auth.account.id, &id).await?;
    let wanted = reqwest::Url::parse(&format!("http://127.0.0.1/{}", req.callback.trim_start_matches('/')))
        .ok()
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.to_string()))
        .unwrap_or_default();
    let begun = {
        let mut g = started();
        let map = g.get_or_insert_with(Default::default);
        match map.get(&wanted) {
            Some((account, tool, _, at)) if account == &auth.account.id && tool == &t.id && at.elapsed() < mcp_oauth::SIGN_IN_WITHIN * 2 => {
                map.remove(&wanted).map(|(_, _, b, _)| b)
            }
            _ => None,
        }
    };
    let Some(begun) = begun else {
        return Err(ApiError::bad_request("That sign-in wasn't started here, or took too long. Try again."));
    };
    let st = mcp_oauth::exchange(&http(), begun, &req.callback).await.map_err(sign_in_error)?;
    keep_sign_in(&state, &mut t, st).await?;
    Ok(Json(view(&state, &t, &auth.account.id, None)))
}

/// Sign out (the tokens are forgotten; a client it used is kept).
#[utoipa::path(delete, path = "/api/tools/{id}/sign-in", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The tool")), responses((status = 204, description = "Signed out")))]
pub async fn sign_out(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let t = tools::get_own(&state.pool, &auth.account.id, &id).await?;
    let kept = oauth(&state, &t).map(|o| OAuthState { access_token: None, refresh_token: None, expires_at: None, ..o });
    let sealed = kept.map(|k| state.vault.seal(&serde_json::to_string(&k).unwrap_or_default()));
    tools::save_oauth(&state.pool, &t.id, sealed.as_deref(), None).await?;
    state.tool_pool.close(&t.id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The household's Google login client (owners set it; anyone signing in to Google uses it).
#[utoipa::path(get, path = "/api/tools/google-client", tag = "tools", security(("bearer" = [])),
    responses((status = 200, description = "Its client ID, if set")))]
pub async fn google_client_get(State(state): State<AppState>, _auth: Auth) -> Json<Value> {
    let c = google_client(&state).await;
    Json(json!({ "client_id": c.as_ref().map(|c| c.client_id.clone()), "has_secret": c.is_some_and(|c| c.client_secret.is_some()) }))
}

#[utoipa::path(put, path = "/api/tools/google-client", tag = "tools", security(("bearer" = [])),
    request_body = GoogleClient, responses((status = 204, description = "Saved")))]
pub async fn google_client_set(State(state): State<AppState>, auth: Auth, Json(c): Json<GoogleClient>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    if c.client_id.trim().is_empty() {
        return Err(ApiError::bad_request("Give the client ID."));
    }
    let c = GoogleClient { client_id: c.client_id.trim().into(), client_secret: c.client_secret.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) };
    outpost_core::settings::set(&state.pool, GOOGLE_CLIENT, &state.vault.seal(&serde_json::to_string(&c).unwrap_or_default())).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ToolIds {
    pub ids: Vec<String>,
}

#[utoipa::path(get, path = "/api/personas/{id}/tools", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The persona")), responses((status = 200, description = "Tool ids")))]
pub async fn persona_get(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<String>>> {
    Ok(Json(tools::persona_tools(&state.pool, &auth.account.id, restricted(&auth), &id).await?))
}

#[utoipa::path(put, path = "/api/personas/{id}/tools", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The persona")), request_body = ToolIds, responses((status = 200, description = "Tool ids")))]
pub async fn persona_set(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(req): Json<ToolIds>) -> ApiResult<Json<Vec<String>>> {
    Ok(Json(tools::set_persona_tools(&state.pool, &auth.account.id, restricted(&auth), &id, &req.ids).await?))
}

#[utoipa::path(get, path = "/api/chats/{id}/tools", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The chat")), responses((status = 200, description = "Tool ids")))]
pub async fn chat_get(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<String>>> {
    Ok(Json(tools::chat_tools(&state.pool, &auth.account.id, restricted(&auth), &id).await?))
}

#[utoipa::path(put, path = "/api/chats/{id}/tools", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The chat")), request_body = ToolIds, responses((status = 200, description = "Tool ids")))]
pub async fn chat_set(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(req): Json<ToolIds>) -> ApiResult<Json<Vec<String>>> {
    Ok(Json(tools::set_chat_tools(&state.pool, &auth.account.id, restricted(&auth), &id, &req.ids).await?))
}

/// Back to the persona's tools.
#[utoipa::path(delete, path = "/api/chats/{id}/tools", tag = "tools", security(("bearer" = [])),
    params(("id" = String, Path, description = "The chat")), responses((status = 204, description = "Reset")))]
pub async fn chat_reset(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    tools::reset_chat_tools(&state.pool, &auth.account.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Node and uv on this server (for `npx`/`uvx` tools), and a download in progress.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct Runtimes {
    pub node: bool,
    pub uv: bool,
    pub installing: Option<String>,
    pub percent: Option<f32>,
    pub error: Option<String>,
}

static RUNTIMES: std::sync::Mutex<Option<Runtimes>> = std::sync::Mutex::new(None);

fn edit_runtimes(f: impl FnOnce(&mut Runtimes)) {
    let mut g = RUNTIMES.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(Default::default));
}

/// The tool library both apps show (the desktop builds it in too): MCP servers that work with
/// Outpost's chats, with what each needs to be added.
const CATALOG: &str = include_str!("../tool-catalog.json");

/// The tool library, for the phone (and anything else adding tools here).
#[utoipa::path(get, path = "/api/tools/catalog", tag = "tools", security(("bearer" = [])),
    responses((status = 200, description = "Library entries", body = [Object])))]
pub async fn catalog(_auth: Auth) -> axum::response::Response {
    use axum::response::IntoResponse;
    ([(axum::http::header::CONTENT_TYPE, "application/json")], CATALOG).into_response()
}

#[utoipa::path(get, path = "/api/tools/runtimes", tag = "tools", security(("bearer" = [])),
    responses((status = 200, description = "Node and uv here", body = Runtimes)))]
pub async fn runtimes(State(state): State<AppState>, _auth: Auth) -> Json<Runtimes> {
    use outpost_engines::runtimes as rt;
    let root = state.config.data_dir.join("engines");
    let mut r = RUNTIMES.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_default();
    r.node = rt::node_bin(&root).is_some() || rt::on_path("npx").is_some();
    r.uv = rt::uv_bin(&root).is_some() || rt::on_path("uvx").is_some();
    Json(r)
}

/// Download Node or uv here, for program tools (owners).
#[utoipa::path(post, path = "/api/tools/runtimes/{which}", tag = "tools", security(("bearer" = [])),
    params(("which" = String, Path, description = "node or uv")), responses((status = 202, description = "Downloading")))]
pub async fn install_runtime(State(state): State<AppState>, auth: Auth, Path(which): Path<String>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    if which != "node" && which != "uv" {
        return Err(ApiError::bad_request("Outpost can install node or uv."));
    }
    if RUNTIMES.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|r| r.installing.is_some()) {
        return Err(ApiError::conflict("A download is running already."));
    }
    edit_runtimes(|r| {
        r.installing = Some(which.clone());
        r.percent = None;
        r.error = None;
    });
    let root = state.config.data_dir.join("engines");
    tokio::spawn(async move {
        let client = http();
        let progress = |done: u64, total: Option<u64>| edit_runtimes(|r| r.percent = total.filter(|t| *t > 0).map(|t| done as f32 * 100.0 / t as f32));
        let result = if which == "node" {
            outpost_engines::runtimes::ensure_node(&client, &root, progress).await
        } else {
            outpost_engines::runtimes::ensure_uv(&client, &root, progress).await
        };
        edit_runtimes(|r| {
            r.installing = None;
            r.percent = None;
            r.error = result.err().map(|e| format!("{e:#}"));
        });
    });
    Ok(StatusCode::ACCEPTED)
}
