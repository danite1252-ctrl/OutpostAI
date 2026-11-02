//! Signing in to MCP servers that use OAuth (the MCP authorization spec): the desktop app
//! finds where the service signs people in, registers itself there when the service allows
//! it (Dynamic Client Registration), or uses an OAuth client the user made (Google), opens
//! the browser, catches the answer on this PC (`http://127.0.0.1:<port>/callback`), and keeps
//! the tokens, refreshing them when they run out.
//!
//! - Discovery: the server's 401 names its protected-resource metadata (RFC 9728), else the
//!   usual `/.well-known/oauth-protected-resource` places are tried; that names the
//!   authorization server, whose metadata (RFC 8414, or OpenID discovery) gives the endpoints.
//!   Older servers without any of it are their own authorization server (`/authorize`,
//!   `/token`, `/register`).
//! - PKCE (S256), `state`, and the `resource` parameter (RFC 8707) are always sent.

use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Longest the browser sign-in may take.
pub const SIGN_IN_WITHIN: Duration = Duration::from_secs(300);

/// The sign-in address for OAuth clients you make yourself (Google wants it registered
/// exactly): `http://127.0.0.1:33418/callback`. Registered clients use any free port.
pub const FIXED_PORT: u16 = 33418;

/// Google's sign-in, for its MCP servers (they don't publish it).
const GOOGLE_AUTHORIZE: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN: &str = "https://oauth2.googleapis.com/token";

fn is_google(endpoint: &str) -> bool {
    endpoint.contains("accounts.google.com") || endpoint.contains("oauth2.googleapis.com")
}

/// A server's sign-in, as kept on this PC (tokens included).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OAuthState {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    /// The client came from Dynamic Client Registration (for `redirect_port` only).
    #[serde(default)]
    pub registered: bool,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    /// The loopback port the client was registered with.
    #[serde(default)]
    pub redirect_port: u16,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl OAuthState {
    pub fn signed_in(&self) -> bool {
        self.access_token.is_some()
    }

    /// The access token runs out within a minute (or has).
    pub fn needs_refresh(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|t| t - 60 <= now)
    }
}

/// Where a server signs people in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Discovered {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub resource: Option<String>,
    pub scope: Option<String>,
}

/// `resource_metadata` and `scope` from a `WWW-Authenticate: Bearer ...` header.
pub fn parse_www_authenticate(header: &str) -> (Option<String>, Option<String>) {
    let mut resource = None;
    let mut scope = None;
    let rest = header.trim();
    let rest = rest.strip_prefix("Bearer").or_else(|| rest.strip_prefix("bearer")).unwrap_or(rest);
    // key="value" or key=value, comma-separated.
    let mut chars = rest.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ',' || c.is_whitespace()) {
            chars.next();
        }
        let key: String = std::iter::from_fn(|| chars.next_if(|c| *c != '=' && *c != ',')).collect();
        if key.is_empty() {
            break;
        }
        if chars.next_if_eq(&'=').is_none() {
            continue;
        }
        let value: String = if chars.next_if_eq(&'"').is_some() {
            let v: String = std::iter::from_fn(|| chars.next_if(|c| *c != '"')).collect();
            chars.next();
            v
        } else {
            std::iter::from_fn(|| chars.next_if(|c| *c != ',')).collect::<String>().trim().to_string()
        };
        match key.trim() {
            "resource_metadata" => resource = Some(value),
            "scope" => scope = Some(value),
            _ => {}
        }
    }
    (resource, scope)
}

fn origin_and_path(url: &str) -> Result<(String, String), String> {
    let u = reqwest::Url::parse(url).map_err(|_| format!("{url} isn't an address."))?;
    let origin = u.origin().ascii_serialization();
    let path = u.path().trim_end_matches('/').to_string();
    Ok((origin, path))
}

/// The MCP server's address as a resource: no fragment, no trailing slash.
pub fn canonical_resource(url: &str) -> String {
    let mut s = url.split('#').next().unwrap_or(url).to_string();
    while s.ends_with('/') {
        s.pop();
    }
    s
}

async fn get_json(client: &reqwest::Client, url: &str) -> Option<Value> {
    let r = client.get(url).header("Accept", "application/json").send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json::<Value>().await.ok().filter(Value::is_object)
}

/// Find out how a server signs people in.
pub async fn discover(client: &reqwest::Client, mcp_url: &str) -> Result<Discovered, String> {
    let (origin, path) = origin_and_path(mcp_url)?;
    // Ask without a token: the 401 says where its metadata is.
    let probe = client
        .post(mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Outpost","version":"1"}}}"#)
        .send()
        .await
        .map_err(|e| format!("Can't reach {mcp_url}: {e}"))?;
    let (header_meta, header_scope) = probe
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .map(parse_www_authenticate)
        .unwrap_or((None, None));
    let mut candidates: Vec<String> = Vec::new();
    candidates.extend(header_meta);
    if !path.is_empty() {
        candidates.push(format!("{origin}/.well-known/oauth-protected-resource{path}"));
    }
    candidates.push(format!("{origin}/.well-known/oauth-protected-resource"));
    let mut prm = None;
    for c in &candidates {
        if let Some(v) = get_json(client, c).await {
            prm = Some(v);
            break;
        }
    }
    let (issuer, resource, scopes) = match &prm {
        Some(p) => {
            let issuer = p["authorization_servers"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .unwrap_or(&origin)
                .to_string();
            let resource = p["resource"].as_str().map(str::to_string).unwrap_or_else(|| canonical_resource(mcp_url));
            let scopes = p["scopes_supported"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
                .filter(|s| !s.is_empty());
            (issuer, Some(resource), scopes)
        }
        None => (origin.clone(), Some(canonical_resource(mcp_url)), None),
    };
    let (iss_origin, iss_path) = origin_and_path(&issuer)?;
    let mut as_candidates = vec![
        format!("{iss_origin}/.well-known/oauth-authorization-server{iss_path}"),
        format!("{iss_origin}/.well-known/openid-configuration{iss_path}"),
    ];
    if !iss_path.is_empty() {
        as_candidates.push(format!("{iss_origin}{iss_path}/.well-known/openid-configuration"));
    }
    let mut meta = None;
    for c in &as_candidates {
        if let Some(v) = get_json(client, c).await {
            meta = Some(v);
            break;
        }
    }
    let scope = header_scope.or(scopes);
    match meta {
        Some(m) => {
            let s = |k: &str| m[k].as_str().map(str::to_string);
            Ok(Discovered {
                authorization_endpoint: s("authorization_endpoint").ok_or("The sign-in service didn't say where to sign in.")?,
                token_endpoint: s("token_endpoint").ok_or("The sign-in service didn't say where its tokens come from.")?,
                registration_endpoint: s("registration_endpoint"),
                resource,
                scope,
            })
        }
        // Google's servers sign in with Google's own accounts.
        None if prm.is_none() && origin.ends_with(".googleapis.com") => Ok(Discovered {
            authorization_endpoint: GOOGLE_AUTHORIZE.into(),
            token_endpoint: GOOGLE_TOKEN.into(),
            registration_endpoint: None,
            resource: None,
            scope,
        }),
        // An older server, signing people in itself at the usual places.
        None if prm.is_none() => Ok(Discovered {
            authorization_endpoint: format!("{origin}/authorize"),
            token_endpoint: format!("{origin}/token"),
            registration_endpoint: Some(format!("{origin}/register")),
            resource,
            scope,
        }),
        None => Err("Couldn't find how this service signs people in.".into()),
    }
}

/// Register Outpost with the service (Dynamic Client Registration).
pub async fn register(client: &reqwest::Client, endpoint: &str, redirect_uri: &str, scope: Option<&str>) -> Result<(String, Option<String>), String> {
    let mut body = serde_json::json!({
        "client_name": "Outpost",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native",
    });
    if let Some(s) = scope {
        body["scope"] = Value::String(s.to_string());
    }
    let r = client
        .post(endpoint)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Couldn't register with the sign-in service: {e}"))?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or_default();
    if !status.is_success() {
        let why = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("refused");
        return Err(format!("The sign-in service wouldn't register Outpost ({status}: {why})."));
    }
    let id = v["client_id"].as_str().ok_or("The sign-in service gave no client ID.")?.to_string();
    Ok((id, v["client_secret"].as_str().map(str::to_string)))
}

/// A PKCE verifier and its S256 challenge.
pub fn pkce() -> (String, String) {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let verifier = b64.encode(&bytes);
    let challenge = b64.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The browser address that starts the sign-in.
pub fn authorize_url(st: &OAuthState, redirect_uri: &str, challenge: &str, state: &str) -> Result<String, String> {
    let mut params: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", &st.client_id),
        ("redirect_uri", redirect_uri),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];
    if let Some(s) = st.scope.as_deref() {
        params.push(("scope", s));
    }
    // (Google doesn't take `resource`.)
    if let Some(r) = st.resource.as_deref().filter(|_| !is_google(&st.authorization_endpoint)) {
        params.push(("resource", r));
    }
    // Ask for a refresh token where the service needs asking (Google).
    if is_google(&st.authorization_endpoint) {
        params.push(("access_type", "offline"));
        params.push(("prompt", "consent"));
    }
    reqwest::Url::parse_with_params(&st.authorization_endpoint, &params)
        .map(|u| u.to_string())
        .map_err(|_| "The sign-in address isn't usable.".to_string())
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn token_request(client: &reqwest::Client, st: &OAuthState, mut form: Vec<(&str, String)>) -> Result<TokenAnswer, String> {
    form.push(("client_id", st.client_id.clone()));
    if let Some(s) = &st.client_secret {
        form.push(("client_secret", s.clone()));
    }
    if let Some(r) = st.resource.as_ref().filter(|_| !is_google(&st.token_endpoint)) {
        form.push(("resource", r.clone()));
    }
    let r = client
        .post(&st.token_endpoint)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the sign-in service: {e}"))?;
    let status = r.status();
    let text = r.text().await.unwrap_or_default();
    let answer: TokenAnswer = serde_json::from_str(&text)
        .or_else(|_| serde_urlencoded_lite(&text))
        .map_err(|_| format!("The sign-in service's answer was garbled ({status})."))?;
    if let Some(e) = &answer.error {
        return Err(format!("The sign-in service said: {}", answer.error_description.as_deref().unwrap_or(e)));
    }
    if answer.access_token.is_none() {
        return Err(format!("The sign-in service gave no token ({status})."));
    }
    Ok(answer)
}

/// Some services answer as a form (`access_token=...&...`).
fn serde_urlencoded_lite(text: &str) -> Result<TokenAnswer, ()> {
    let pairs: Vec<(String, String)> = reqwest::Url::parse(&format!("http://x/?{text}"))
        .map_err(|_| ())?
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let get = |k: &str| pairs.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    if get("access_token").is_none() && get("error").is_none() {
        return Err(());
    }
    Ok(TokenAnswer {
        access_token: get("access_token"),
        refresh_token: get("refresh_token"),
        expires_in: get("expires_in").and_then(|v| v.parse().ok()),
        error: get("error"),
        error_description: get("error_description"),
    })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn keep_tokens(st: &mut OAuthState, a: TokenAnswer) {
    st.access_token = a.access_token;
    if a.refresh_token.is_some() {
        st.refresh_token = a.refresh_token;
    }
    st.expires_at = a.expires_in.map(|s| now() + s);
}

/// Swap the refresh token for a new access token.
pub async fn refresh(client: &reqwest::Client, st: &OAuthState) -> Result<OAuthState, String> {
    let rt = st.refresh_token.clone().ok_or("needs sign-in: sign in again.")?;
    let answer = token_request(client, st, vec![("grant_type", "refresh_token".into()), ("refresh_token", rt)])
        .await
        .map_err(|e| format!("needs sign-in: {e}"))?;
    let mut next = st.clone();
    keep_tokens(&mut next, answer);
    Ok(next)
}

/// Wait for the browser to come back to `listener` with the code (for `state`).
async fn catch_code(listener: TcpListener, state: &str) -> Result<String, String> {
    loop {
        let (mut sock, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        let target = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("").to_string();
        let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}")).ok();
        let get = |k: &str| url.as_ref().and_then(|u| u.query_pairs().find(|(x, _)| x == k).map(|(_, v)| v.to_string()));
        if !target.starts_with("/callback") {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let (ok, result) = match (get("code"), get("state"), get("error")) {
            (_, _, Some(e)) => (false, Err(format!("The sign-in was refused: {}", get("error_description").unwrap_or(e)))),
            (Some(code), Some(s), None) if s == state => (true, Ok(code)),
            _ => (false, Err("The sign-in came back without what Outpost asked for. Try again.".to_string())),
        };
        let page = if ok {
            "<!doctype html><meta charset=utf-8><title>Signed in</title><body style=\"font-family:system-ui;background:#16110f;color:#f3eee8;display:grid;place-items:center;height:100vh;margin:0\"><div><h1>Signed in to Outpost</h1><p>You can close this tab and go back to Outpost.</p></div>"
        } else {
            "<!doctype html><meta charset=utf-8><title>Not signed in</title><body style=\"font-family:system-ui;background:#16110f;color:#f3eee8;display:grid;place-items:center;height:100vh;margin:0\"><div><h1>Not signed in</h1><p>Go back to Outpost to see why.</p></div>"
        };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
        return result;
    }
}

/// Sign in to the MCP server at `mcp_url`: `manual` is an OAuth client the user made (id,
/// secret), for services that don't let apps register (it signs in at [`FIXED_PORT`]);
/// `scope` asks for these scopes over what the service says; `previous` is the last sign-in
/// (its registration is reused). `open` shows the address in the browser.
pub async fn sign_in(
    client: &reqwest::Client,
    mcp_url: &str,
    manual: Option<(String, Option<String>)>,
    scope: Option<String>,
    previous: Option<&OAuthState>,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<OAuthState, String> {
    let mut found = discover(client, mcp_url).await?;
    if let Some(s) = scope.filter(|s| !s.trim().is_empty()) {
        found.scope = Some(s);
    }
    // The same port as last time keeps a registered client's redirect address valid.
    let reuse = previous.filter(|p| {
        p.registered && manual.is_none() && p.redirect_port != 0 && p.registration_endpoint == found.registration_endpoint
    });
    let listener = match reuse {
        Some(p) => match TcpListener::bind(("127.0.0.1", p.redirect_port)).await {
            Ok(l) => Some(l),
            Err(_) => None,
        },
        None => None,
    };
    let (listener, reuse) = match listener {
        Some(l) => (l, reuse),
        // Your own client: the address you registered with it.
        None if manual.is_some() => (
            TcpListener::bind(("127.0.0.1", FIXED_PORT))
                .await
                .map_err(|e| format!("Couldn't listen for the sign-in at port {FIXED_PORT} (is another sign-in open?): {e}"))?,
            None,
        ),
        None => (TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("Couldn't listen for the sign-in: {e}"))?, None),
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let mut st = OAuthState {
        authorization_endpoint: found.authorization_endpoint.clone(),
        token_endpoint: found.token_endpoint.clone(),
        registration_endpoint: found.registration_endpoint.clone(),
        resource: found.resource.clone(),
        scope: found.scope.clone(),
        redirect_port: port,
        ..Default::default()
    };
    match (manual, reuse) {
        (Some((id, secret)), _) => {
            st.client_id = id;
            st.client_secret = secret.filter(|s| !s.is_empty());
        }
        (None, Some(p)) => {
            st.client_id = p.client_id.clone();
            st.client_secret = p.client_secret.clone();
            st.registered = true;
        }
        (None, None) => {
            let endpoint = found.registration_endpoint.clone().ok_or(
                "needs client: this service doesn't let apps register themselves. Make an OAuth client for it (its instructions say how) and enter its client ID.",
            )?;
            let (id, secret) = register(client, &endpoint, &redirect, found.scope.as_deref()).await?;
            st.client_id = id;
            st.client_secret = secret;
            st.registered = true;
        }
    }
    finish(client, st, listener, open).await
}

/// Google's sign-in, for an OAuth client of the "Desktop app" kind (any loopback port works).
pub fn google_state(client_id: &str, client_secret: Option<String>, scope: &str) -> OAuthState {
    OAuthState {
        client_id: client_id.trim().to_string(),
        client_secret: client_secret.filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()),
        authorization_endpoint: GOOGLE_AUTHORIZE.into(),
        token_endpoint: GOOGLE_TOKEN.into(),
        scope: Some(scope.to_string()),
        ..Default::default()
    }
}

/// Sign in where the endpoints and client are known already (`st`): any free loopback port.
pub async fn sign_in_known(client: &reqwest::Client, st: OAuthState, open: impl FnOnce(&str) -> Result<(), String>) -> Result<OAuthState, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("Couldn't listen for the sign-in: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    finish(client, OAuthState { redirect_port: port, ..st }, listener, open).await
}

/// The browser part: open the sign-in, catch the code, swap it for tokens.
async fn finish(client: &reqwest::Client, mut st: OAuthState, listener: TcpListener, open: impl FnOnce(&str) -> Result<(), String>) -> Result<OAuthState, String> {
    let redirect = format!("http://127.0.0.1:{}/callback", st.redirect_port);
    let (verifier, challenge) = pkce();
    let state = uuid::Uuid::new_v4().simple().to_string();
    open(&authorize_url(&st, &redirect, &challenge, &state)?)?;
    let code = tokio::time::timeout(SIGN_IN_WITHIN, catch_code(listener, &state))
        .await
        .map_err(|_| "The sign-in took too long (5 minutes). Try again.".to_string())??;
    let answer = token_request(
        client,
        &st,
        vec![
            ("grant_type", "authorization_code".into()),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    keep_tokens(&mut st, answer);
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn www_authenticate_names_the_metadata_and_scope() {
        let (m, s) = parse_www_authenticate(r#"Bearer error="invalid_token", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource", scope="files.read files.write""#);
        assert_eq!(m.as_deref(), Some("https://mcp.example.com/.well-known/oauth-protected-resource"));
        assert_eq!(s.as_deref(), Some("files.read files.write"));
        assert_eq!(parse_www_authenticate("Bearer realm=x"), (None, None));
    }

    #[test]
    fn pkce_and_the_sign_in_address() {
        let (v, c) = pkce();
        assert!(v.len() >= 43);
        assert_eq!(c, base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())));
        let st = OAuthState {
            client_id: "abc".into(),
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            resource: Some("https://mcp.example.com/mcp".into()),
            scope: Some("read write".into()),
            ..Default::default()
        };
        let url = authorize_url(&st, "http://127.0.0.1:4711/callback", &c, "s1").unwrap();
        let u = reqwest::Url::parse(&url).unwrap();
        let q: std::collections::HashMap<String, String> = u.query_pairs().map(|(k, v)| (k.into(), v.into())).collect();
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:4711/callback");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["resource"], "https://mcp.example.com/mcp");
        assert_eq!(q["scope"], "read write");
        assert_eq!(canonical_resource("https://mcp.example.com/mcp/#x"), "https://mcp.example.com/mcp");
    }

    #[test]
    fn tokens_run_out() {
        let st = OAuthState { expires_at: Some(1000), ..Default::default() };
        assert!(!st.needs_refresh(900));
        assert!(st.needs_refresh(950));
        assert!(!OAuthState::default().needs_refresh(i64::MAX));
    }
}
