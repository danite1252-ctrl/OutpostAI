//! Tools on the Outpost server, per person: added through the API with their keys sealed,
//! started and run here against a stand-in MCP server, chosen per persona and chat, shared
//! with the household or kept personal, and signed in to.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use http_body_util::BodyExt;
use outpost_gateway::{router, AppState, Config};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn app() -> (Router, AppState) {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = std::env::temp_dir().join(format!("outpost-server-tools-{}", uuid::Uuid::new_v4()));
    let state = AppState::with_pool(pool, config);
    (router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000)))), state)
}

async fn call(app: &Router, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let resp = app.clone().oneshot(req.body(Body::from(body.map(|b| b.to_string()).unwrap_or_default())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn people(app: &Router) -> (String, String, String) {
    let (_, owner) = call(app, "POST", "/api/setup/owner", None, Some(json!({"name": "Dan", "secret": "4821"}))).await;
    let dan = owner["token"].as_str().unwrap().to_string();
    let mut tokens = Vec::new();
    for (name, role) in [("SJ", "member"), ("Noah", "restricted")] {
        let (s, acct) = call(app, "POST", "/api/accounts", Some(&dan), Some(json!({"name": name, "role": role, "secret": "hunter22"}))).await;
        assert_eq!(s, StatusCode::CREATED, "{acct}");
        let (_, signed) = call(app, "POST", "/api/auth/sign-in", None, Some(json!({"account_id": acct["id"], "secret": "hunter22"}))).await;
        tokens.push(signed["token"].as_str().unwrap().to_string());
    }
    (dan, tokens.remove(0), tokens.remove(0))
}

/// A stand-in MCP server that wants `Authorization: Bearer key-1`.
async fn fake_mcp(seen: Arc<Mutex<Vec<String>>>) -> String {
    let app = Router::new().route(
        "/mcp",
        post(move |headers: HeaderMap, Json(msg): Json<Value>| {
            let seen = seen.clone();
            async move {
                let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                seen.lock().unwrap().push(auth.clone());
                if auth != "Bearer key-1" {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                let Some(id) = msg.get("id").cloned() else { return StatusCode::ACCEPTED.into_response() };
                let result = match msg["method"].as_str() {
                    Some("initialize") => json!({ "protocolVersion": "2025-06-18", "capabilities": {} }),
                    Some("tools/list") => json!({ "tools": [{ "name": "HassTurnOff", "description": "Turn a device off", "inputSchema": { "type": "object" } }] }),
                    Some("tools/call") => json!({ "content": [{ "type": "text", "text": format!("Turned off {}", msg["params"]["arguments"]["name"].as_str().unwrap_or("?")) }] }),
                    _ => json!({}),
                };
                Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[tokio::test]
async fn tools_on_the_server_are_per_person_and_shared_ones_reach_everyone() {
    let (app, state) = app().await;
    let (dan, sj, noah) = people(&app).await;
    let seen: Arc<Mutex<Vec<String>>> = Default::default();
    let url = fake_mcp(seen.clone()).await;

    // Dan adds Home Assistant with its key; the key is kept sealed and never shown.
    let (s, ha) = call(&app, "POST", "/api/tools", Some(&dan), Some(json!({
        "name": "Home Assistant", "transport": "http", "url": url,
        "secrets": { "headers": { "Authorization": "Bearer key-1" } }
    }))).await;
    assert_eq!(s, StatusCode::CREATED, "{ha}");
    let id = ha["id"].as_str().unwrap().to_string();
    assert_eq!(ha["secret_names"], json!(["Authorization"]));
    assert!(!ha.to_string().contains("key-1"));
    let stored: (Option<String>,) = sqlx::query_as("SELECT secrets FROM tools WHERE id = ?").bind(&id).fetch_one(&state.pool).await.unwrap();
    assert!(!stored.0.unwrap().contains("key-1"), "keys are sealed in the database");

    // Test starts it here and lists its tools, sending the key.
    let (s, list) = call(&app, "GET", &format!("/api/tools/{id}/tools"), Some(&dan), None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(list[0]["name"], "HassTurnOff");
    assert!(seen.lock().unwrap().iter().all(|a| a == "Bearer key-1"));

    // Personal: SJ and Noah don't see it. Shared (and allowed for restricted): everyone does.
    let (_, sj_list) = call(&app, "GET", "/api/tools", Some(&sj), None).await;
    assert_eq!(sj_list.as_array().unwrap().len(), 0);
    let (s, _) = call(&app, "PATCH", &format!("/api/tools/{id}"), Some(&dan), Some(json!({ "shared": true }))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, sj_list) = call(&app, "GET", "/api/tools", Some(&sj), None).await;
    assert_eq!(sj_list[0]["owner_name"], "Dan");
    assert_eq!(sj_list[0]["mine"], false);
    let (_, noah_list) = call(&app, "GET", "/api/tools", Some(&noah), None).await;
    assert_eq!(noah_list.as_array().unwrap().len(), 0);
    call(&app, "PATCH", &format!("/api/tools/{id}"), Some(&dan), Some(json!({ "restricted_ok": true }))).await;
    let (_, noah_list) = call(&app, "GET", "/api/tools", Some(&noah), None).await;
    assert_eq!(noah_list.as_array().unwrap().len(), 1);
    // Only Dan changes it; SJ (a member) can't add programs; Noah can't add tools.
    let (s, _) = call(&app, "PATCH", &format!("/api/tools/{id}"), Some(&sj), Some(json!({ "name": "Mine now" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "POST", "/api/tools", Some(&sj), Some(json!({ "name": "Fetch", "transport": "stdio", "command": "uvx", "args": ["mcp-server-fetch"] }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&app, "POST", "/api/tools", Some(&noah), Some(json!({ "name": "X", "transport": "http", "url": url }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // SJ's chat uses it, and the call runs here (as the chat's model would).
    let (_, chat) = call(&app, "POST", "/api/chats", Some(&sj), Some(json!({}))).await;
    let chat_id = chat["id"].as_str().expect("a chat").to_string();
    let sj_id = chat["account_id"].as_str().unwrap().to_string();
    let (s, ids) = call(&app, "PUT", &format!("/api/chats/{chat_id}/tools"), Some(&sj), Some(json!({ "ids": [id] }))).await;
    assert_eq!(s, StatusCode::OK, "{ids}");
    let servers = outpost_gateway::server_tools::for_chat(&state, &sj_id, false, &chat_id).await;
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].tools[0].name, "HassTurnOff");
    let r = outpost_gateway::server_tools::call(&state, &id, "HassTurnOff", json!({ "name": "kitchen lamp" })).await.unwrap();
    assert_eq!(r.text, "Turned off kitchen lamp");

    // A wrong key: Test says to sign in (or check the key).
    call(&app, "PATCH", &format!("/api/tools/{id}"), Some(&dan), Some(json!({ "secrets": { "headers": { "Authorization": "Bearer wrong" } } }))).await;
    let (s, err) = call(&app, "GET", &format!("/api/tools/{id}/tools"), Some(&dan), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(err["error"]["message"].as_str().unwrap().starts_with("needs sign-in:"), "{err}");

    // A sign-in made on a device is kept here, and its token used over the key.
    let (s, signed) = call(&app, "PUT", &format!("/api/tools/{id}/sign-in"), Some(&dan), Some(json!({
        "client_id": "c", "authorization_endpoint": "https://a/authorize", "token_endpoint": "https://a/token", "access_token": "key-1"
    }))).await;
    assert_eq!(s, StatusCode::OK, "{signed}");
    assert_eq!(signed["signed_in"], true);
    let (s, list) = call(&app, "GET", &format!("/api/tools/{id}/tools"), Some(&dan), None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    // Signing out forgets the token.
    call(&app, "DELETE", &format!("/api/tools/{id}/sign-in"), Some(&dan), None).await;
    let (_, l) = call(&app, "GET", "/api/tools", Some(&dan), None).await;
    assert_eq!(l[0]["signed_in"], false);

    // The household Google client: owners set it; the ID is shown, never the secret.
    let (s, _) = call(&app, "PUT", "/api/tools/google-client", Some(&sj), Some(json!({ "client_id": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    call(&app, "PUT", "/api/tools/google-client", Some(&dan), Some(json!({ "client_id": "abc.apps.googleusercontent.com", "client_secret": "s3" }))).await;
    let (_, gc) = call(&app, "GET", "/api/tools/google-client", Some(&sj), None).await;
    assert_eq!(gc, json!({ "client_id": "abc.apps.googleusercontent.com", "has_secret": true }));
    let (_, g) = call(&app, "POST", "/api/tools", Some(&sj), Some(json!({ "name": "Google", "transport": "builtin", "command": "google", "args": ["drive", "gmail"] }))).await;
    let (_, info) = call(&app, "GET", &format!("/api/tools/{}/sign-in", g["id"].as_str().unwrap()), Some(&sj), None).await;
    assert_eq!(info["kind"], "google");
    assert_eq!(info["client_id"], "abc.apps.googleusercontent.com");
    assert!(info["scope"].as_str().unwrap().contains("gmail.readonly"));
    // Not signed in yet: its tools say so.
    let (s, _) = call(&app, "GET", &format!("/api/tools/{}/tools", g["id"].as_str().unwrap()), Some(&sj), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
}
