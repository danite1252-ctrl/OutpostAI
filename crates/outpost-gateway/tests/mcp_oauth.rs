//! Signing in to an MCP server with OAuth, against a stand-in service that does it all:
//! a 401 naming its metadata, discovery, client registration, the browser's redirect back to
//! this PC, PKCE, tokens and refreshing them; then the signed-in token works on the server.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use outpost_gateway::mcp_client::{Pool, ServerConfig};
use outpost_gateway::mcp_oauth;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Fake {
    base: String,
    challenge: Option<String>,
    registered_redirect: Option<String>,
    resource_seen: Option<String>,
}

type S = Arc<Mutex<Fake>>;

async fn mcp(State(s): State<S>, headers: HeaderMap, Json(msg): Json<Value>) -> Response {
    let ok = headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer tok-2");
    if !ok {
        let base = s.lock().unwrap().base.clone();
        return (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", format!(r#"Bearer resource_metadata="{base}/.well-known/oauth-protected-resource", scope="tools""#))],
        )
            .into_response();
    }
    let Some(id) = msg.get("id").cloned() else { return StatusCode::ACCEPTED.into_response() };
    let result = match msg["method"].as_str() {
        Some("initialize") => json!({ "protocolVersion": "2025-06-18", "capabilities": {} }),
        Some("tools/list") => json!({ "tools": [{ "name": "whoami", "description": "Says who you are" }] }),
        _ => json!({}),
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

async fn prm(State(s): State<S>) -> Json<Value> {
    let base = s.lock().unwrap().base.clone();
    Json(json!({ "resource": format!("{base}/mcp"), "authorization_servers": [base] }))
}

async fn asm(State(s): State<S>) -> Json<Value> {
    let base = s.lock().unwrap().base.clone();
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"),
        "code_challenge_methods_supported": ["S256"],
    }))
}

async fn register(State(s): State<S>, Json(body): Json<Value>) -> Response {
    s.lock().unwrap().registered_redirect = body["redirect_uris"][0].as_str().map(str::to_string);
    (StatusCode::CREATED, Json(json!({ "client_id": "outpost-1" }))).into_response()
}

async fn authorize(State(s): State<S>, Query(q): Query<HashMap<String, String>>) -> Response {
    let mut f = s.lock().unwrap();
    assert_eq!(q["client_id"], "outpost-1");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(Some(&q["redirect_uri"]), f.registered_redirect.as_ref());
    assert_eq!(q["scope"], "tools");
    f.challenge = Some(q["code_challenge"].clone());
    f.resource_seen = q.get("resource").cloned();
    // The "browser" signs in and comes back to Outpost.
    Redirect::to(&format!("{}?code=c0de&state={}", q["redirect_uri"], q["state"])).into_response()
}

async fn token(State(s): State<S>, Form(f): Form<HashMap<String, String>>) -> Response {
    let fake = s.lock().unwrap();
    match f.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
            let expect = b64.encode(Sha256::digest(f["code_verifier"].as_bytes()));
            if f["code"] != "c0de" || Some(&expect) != fake.challenge.as_ref() {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_grant" }))).into_response();
            }
            Json(json!({ "access_token": "tok-1", "refresh_token": "r-1", "expires_in": 30, "token_type": "Bearer" })).into_response()
        }
        Some("refresh_token") if f["refresh_token"] == "r-1" => {
            Json(json!({ "access_token": "tok-2", "expires_in": 3600, "token_type": "Bearer" })).into_response()
        }
        _ => (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_grant", "error_description": "bad refresh" }))).into_response(),
    }
}

#[tokio::test]
async fn signing_in_with_oauth_and_using_the_token() {
    let state: S = Default::default();
    let app = Router::new()
        .route("/mcp", post(mcp))
        .route("/.well-known/oauth-protected-resource", get(prm))
        .route("/.well-known/oauth-authorization-server", get(asm))
        .route("/register", post(register))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    state.lock().unwrap().base = base.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let url = format!("{base}/mcp");
    let found = mcp_oauth::discover(&client, &url).await.unwrap();
    assert_eq!(found.authorization_endpoint, format!("{base}/authorize"));
    assert_eq!(found.scope.as_deref(), Some("tools"));

    // The "browser": opening the address follows the redirects back to Outpost.
    let st = mcp_oauth::sign_in(&client, &url, None, None, None, |address| {
        let address = address.to_string();
        tokio::spawn(async move {
            let _ = reqwest::get(address).await;
        });
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(st.access_token.as_deref(), Some("tok-1"));
    assert_eq!(st.refresh_token.as_deref(), Some("r-1"));
    assert!(st.registered);
    assert_eq!(state.lock().unwrap().resource_seen.as_deref(), Some(format!("{base}/mcp").as_str()));

    // tok-1 isn't accepted by the server (as if it ran out): refreshing gives tok-2, and the
    // refresh token is kept when the service doesn't send a new one.
    let st = mcp_oauth::refresh(&client, &st).await.unwrap();
    assert_eq!(st.access_token.as_deref(), Some("tok-2"));
    assert_eq!(st.refresh_token.as_deref(), Some("r-1"));

    let pool = Pool::new(std::env::temp_dir());
    let cfg = ServerConfig {
        id: "svc".into(),
        name: "Service".into(),
        transport: "http".into(),
        url: url.clone(),
        headers: BTreeMap::from([("Authorization".to_string(), "Bearer tok-2".to_string())]),
        ..Default::default()
    };
    let tools = pool.tools(&cfg).await.unwrap();
    assert_eq!(tools[0].name, "whoami");

    // Without a token the server asks for a sign-in, and says so.
    let anon = ServerConfig { id: "anon".into(), headers: BTreeMap::new(), ..cfg };
    let err = pool.tools(&anon).await.unwrap_err();
    assert!(err.starts_with("needs sign-in:"), "{err}");

    // A refresh that's refused means signing in again.
    let bad = mcp_oauth::OAuthState { refresh_token: Some("old".into()), ..st };
    let err = mcp_oauth::refresh(&client, &bad).await.unwrap_err();
    assert!(err.starts_with("needs sign-in:"), "{err}");
}
