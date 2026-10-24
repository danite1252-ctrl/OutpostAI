//! End-to-end tests of the HTTP API against an in-memory database.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use outpost_gateway::{router, AppState, Config};
use serde_json::{json, Value};
use tower::ServiceExt;

fn temp_data_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "outpost-gateway-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ))
}

async fn app_from(addr: [u8; 4]) -> Router {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    // Keep test engine folders away from the real Outpost data folder.
    config.data_dir = temp_data_dir();
    // Nothing listens here, so Ollama looks "not running" and fails fast.
    config.ollama_url = "http://127.0.0.1:9".into();
    router(AppState::with_pool(pool, config)).layer(MockConnectInfo(SocketAddr::from((addr, 50000))))
}

async fn app() -> Router {
    app_from([127, 0, 0, 1]).await
}

/// One Outpost seen two ways: from this PC and from another machine on the network.
async fn local_and_remote() -> (Router, Router) {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = temp_data_dir();
    config.ollama_url = "http://127.0.0.1:9".into();
    let state = AppState::with_pool(pool, config);
    (
        router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000)))),
        router(state).layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 77], 50000)))),
    )
}

async fn call(app: &Router, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        // Every API error is JSON; anything else is kept as text so a failing test shows it.
        serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, value)
}

async fn setup_owner(app: &Router) -> (String, String) {
    let (status, body) = call(
        app,
        "POST",
        "/api/setup/owner",
        None,
        Some(json!({"name": "Dan", "secret": "4821"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["token"].as_str().unwrap().to_string(),
        body["account"]["id"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn first_run_creates_owner_once() {
    let app = app().await;
    let (_, health) = call(&app, "GET", "/api/health", None, None).await;
    assert_eq!(health["setup_required"], true);

    let (token, _) = setup_owner(&app).await;
    assert!(token.starts_with("op_"));

    let (_, health) = call(&app, "GET", "/api/health", None, None).await;
    assert_eq!(health["setup_required"], false);

    let (status, body) = call(
        &app,
        "POST",
        "/api/setup/owner",
        None,
        Some(json!({"name": "Eve", "secret": "1111"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "conflict");
}

#[tokio::test]
async fn setup_and_sign_in_only_from_this_pc() {
    let app = app_from([192, 168, 1, 50]).await;
    let (status, _) = call(
        &app,
        "POST",
        "/api/setup/owner",
        None,
        Some(json!({"name": "Dan", "secret": "4821"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn sign_in_me_and_sign_out() {
    let app = app().await;
    let (setup_token, dan_id) = setup_owner(&app).await;

    let (_, dir) = call(&app, "GET", "/api/accounts/directory", None, None).await;
    assert_eq!(dir.as_array().unwrap().len(), 1);
    assert_eq!(dir[0]["name"], "Dan");

    let (status, body) = call(
        &app,
        "POST",
        "/api/auth/sign-in",
        None,
        Some(json!({"account_id": dan_id, "secret": "4821"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["token"].as_str().unwrap().to_string();
    assert_ne!(token, setup_token);

    let (status, me) = call(&app, "GET", "/api/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["account"]["role"], "owner");
    assert_eq!(me["session"]["kind"], "desktop");

    let (status, _) = call(&app, "POST", "/api/auth/sign-out", Some(&token), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(&app, "GET", "/api/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn wrong_pins_count_down_then_lock() {
    let app = app().await;
    let (_, dan_id) = setup_owner(&app).await;
    let attempt = json!({"account_id": dan_id, "secret": "0000"});
    for left in [4, 3, 2, 1] {
        let (status, body) = call(&app, "POST", "/api/auth/sign-in", None, Some(attempt.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "bad_secret");
        assert_eq!(body["error"]["attempts_left"], left);
    }
    let (status, body) = call(&app, "POST", "/api/auth/sign-in", None, Some(attempt)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "locked");
    assert!(body["error"]["retry_after"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn only_owners_manage_accounts() {
    let app = app().await;
    let (owner_token, owner_id) = setup_owner(&app).await;

    let (status, sarah) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(&owner_token),
        Some(json!({"name": "Sarah", "role": "member", "secret": "hunter22"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sarah}");
    let sarah_id = sarah["id"].as_str().unwrap().to_string();

    let (_, signed_in) = call(
        &app,
        "POST",
        "/api/auth/sign-in",
        None,
        Some(json!({"account_id": sarah_id, "secret": "hunter22"})),
    )
    .await;
    let sarah_token = signed_in["token"].as_str().unwrap().to_string();

    let (status, _) = call(&app, "GET", "/api/accounts", Some(&sarah_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(&sarah_token),
        Some(json!({"name": "Mallory", "role": "owner", "secret": "9999"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Owner cannot delete themselves; can delete Sarah.
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/accounts/{owner_id}"),
        Some(&owner_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/accounts/{sarah_id}"),
        Some(&owner_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Sarah's session died with her account.
    let (status, _) = call(&app, "GET", "/api/me", Some(&sarah_token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn changing_my_pin_needs_the_current_one() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let (status, _) = call(
        &app,
        "PUT",
        "/api/me/secret",
        Some(&token),
        Some(json!({"current_secret": "0000", "new_secret": "135790"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(
        &app,
        "PUT",
        "/api/me/secret",
        Some(&token),
        Some(json!({"current_secret": "4821", "new_secret": "135790"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "GET", "/api/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "old session should be signed out");
}

#[tokio::test]
async fn models_need_auth_and_answer_without_ollama() {
    let app = app().await;
    let (status, _) = call(&app, "GET", "/v1/models", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (token, _) = setup_owner(&app).await;
    let (status, body) = call(&app, "GET", "/v1/models", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["object"], "list");
    // With Ollama unreachable, only the account's personas are listed.
    let data = body["data"].as_array().unwrap();
    assert!(data.iter().all(|m| m["owned_by"] == "outpost-persona"), "{body}");
    assert!(data.iter().any(|m| m["id"] == "persona:assistant"));

    let (status, body) = call(&app, "GET", "/api/engines", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let engines = body["engines"].as_array().unwrap();
    assert_eq!(engines.len(), 2);
    assert_eq!(engines[0]["id"], "ollama");
    assert_eq!(engines[1]["id"], "comfyui");
    for e in engines {
        // Nothing is installed in a fresh data folder; a copy already running on
        // this machine shows up as `external`.
        let state = e["state"].as_str().unwrap();
        assert!(
            state == "not_installed" || state == "external",
            "unexpected state {state}"
        );
    }
}

#[tokio::test]
async fn openapi_lists_every_route() {
    let app = app().await;
    let (status, spec) = call(&app, "GET", "/openapi.json", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let paths = spec["paths"].as_object().unwrap();
    for p in [
        "/api/health",
        "/api/setup/owner",
        "/api/accounts/directory",
        "/api/auth/sign-in",
        "/api/auth/sign-out",
        "/api/me",
        "/api/me/secret",
        "/api/accounts",
        "/api/accounts/{id}",
        "/api/accounts/{id}/secret",
        "/api/engines",
        "/api/engines/{id}/install",
        "/api/engines/{id}/start",
        "/api/engines/{id}/stop",
        "/v1/models",
        "/api/hardware",
        "/api/models",
        "/api/models/pull",
        "/api/models/remove",
        "/v1/chat/completions",
        "/api/personas",
        "/api/personas/{id}",
        "/api/chats",
        "/api/chats/{id}",
        "/api/chats/{id}/messages",
        "/api/memory",
        "/api/memory/{id}",
        "/api/keys",
        "/api/keys/{id}",
        "/api/settings/network",
        "/api/files/{id}",
        "/api/memory/settings",
        "/api/memory/auto",
        "/api/packs",
        "/api/packs/{id}/install",
        "/api/loras",
        "/api/jobs",
        "/api/jobs/{id}",
        "/api/gallery",
        "/api/media/{id}",
        "/api/downloads",
        "/api/gallery/shared",
        "/api/media/{id}/share",
        "/api/queue",
    ] {
        assert!(paths.contains_key(p), "missing {p} in OpenAPI spec");
    }
    assert!(spec["components"]["securitySchemes"]["bearer"].is_object());
}

#[tokio::test]
async fn only_owners_control_engines() {
    let app = app().await;
    let (owner_token, _) = setup_owner(&app).await;
    let (_, sarah) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(&owner_token),
        Some(json!({"name": "Sarah", "role": "member", "secret": "hunter22"})),
    )
    .await;
    let (_, signed_in) = call(
        &app,
        "POST",
        "/api/auth/sign-in",
        None,
        Some(json!({"account_id": sarah["id"], "secret": "hunter22"})),
    )
    .await;
    let sarah_token = signed_in["token"].as_str().unwrap().to_string();

    let (status, _) = call(&app, "POST", "/api/engines/ollama/install", Some(&sarah_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Unknown engine names are rejected before anything runs.
    let (status, body) = call(
        &app,
        "POST",
        "/api/engines/stable-diffusion/start",
        Some(&owner_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid", "{body}");

    // Starting something that isn't installed is refused (or 409 if a copy already
    // runs outside Outpost on this machine).
    let (status, _) = call(&app, "POST", "/api/engines/comfyui/start", Some(&owner_token), None).await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::CONFLICT,
        "{status}"
    );
}

#[tokio::test]
async fn models_overview_and_permissions() {
    let app = app().await;
    let (owner_token, _) = setup_owner(&app).await;

    let (status, hw) = call(&app, "GET", "/api/hardware", Some(&owner_token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(hw["ram_mb"].as_u64().unwrap() > 0);

    let (status, overview) = call(&app, "GET", "/api/models", Some(&owner_token), None).await;
    assert_eq!(status, StatusCode::OK);
    let catalog = overview["catalog"].as_array().unwrap();
    assert!(catalog.len() >= 10);
    // Unfiltered Qwen models are listed first.
    assert_eq!(catalog[0]["category"], "uncensored");
    assert!(catalog[0]["model"].as_str().unwrap().contains("qwen"));
    assert!(overview["installed"].is_array());

    // Bad names are refused before Ollama is asked.
    let (status, _) = call(
        &app,
        "POST",
        "/api/models/pull",
        Some(&owner_token),
        Some(json!({"model": "../../etc"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, sarah) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(&owner_token),
        Some(json!({"name": "Sarah", "role": "member", "secret": "hunter22"})),
    )
    .await;
    let (_, signed_in) = call(
        &app,
        "POST",
        "/api/auth/sign-in",
        None,
        Some(json!({"account_id": sarah["id"], "secret": "hunter22"})),
    )
    .await;
    let sarah_token = signed_in["token"].as_str().unwrap().to_string();
    let (status, _) = call(
        &app,
        "POST",
        "/api/models/pull",
        Some(&sarah_token),
        Some(json!({"model": "qwen3.5:4b"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn personas_chats_and_memory() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;

    // Every account starts with the Assistant persona.
    let (status, personas) = call(&app, "GET", "/api/personas", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(personas[0]["slug"], "assistant");

    let (status, coder) = call(
        &app,
        "POST",
        "/api/personas",
        Some(&token),
        Some(json!({"name": "Coder", "system_prompt": "Terse senior engineer.", "think": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{coder}");
    let coder_id = coder["id"].as_str().unwrap().to_string();

    let (status, renamed) = call(
        &app,
        "PATCH",
        &format!("/api/personas/{coder_id}"),
        Some(&token),
        Some(json!({"name": "Senior Coder"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["slug"], "coder");

    let (status, chat) = call(
        &app,
        "POST",
        "/api/chats",
        Some(&token),
        Some(json!({"persona_id": coder_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let chat_id = chat["id"].as_str().unwrap().to_string();

    // Empty messages are refused before any model is involved.
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/chats/{chat_id}/messages"),
        Some(&token),
        Some(json!({"content": "   "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, full) = call(&app, "GET", &format!("/api/chats/{chat_id}"), Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(full["messages"].as_array().unwrap().is_empty());

    let (status, m) = call(
        &app,
        "POST",
        "/api/memory",
        Some(&token),
        Some(json!({"text": "Lives in Sydney"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, list) = call(&app, "GET", "/api/memory", Some(&token), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/memory/{}", m["id"].as_str().unwrap()),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Personas show up as models for OpenAI-style apps.
    let (_, models) = call(&app, "GET", "/v1/models", Some(&token), None).await;
    let ids: Vec<_> = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&"persona:assistant".to_string()));
    assert!(ids.contains(&"persona:coder".to_string()));

    let (status, _) = call(&app, "DELETE", &format!("/api/chats/{chat_id}"), Some(&token), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn openai_endpoint_checks_before_calling_ollama() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let (status, body) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some(&token),
        Some(json!({"messages": [{"role": "user", "content": "hi"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some(&token),
        Some(json!({"model": "persona:nobody", "messages": [{"role": "user", "content": "hi"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, _) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        None,
        Some(json!({"model": "persona:assistant", "messages": []})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn app_keys_act_as_their_account() {
    let app = app().await;
    let (owner_token, _) = setup_owner(&app).await;

    // A service account for Hermes, with a key made by the owner.
    let (status, hermes) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(&owner_token),
        Some(json!({"name": "Hermes", "role": "service"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hermes}");
    let hermes_id = hermes["id"].as_str().unwrap().to_string();
    let (status, key) = call(
        &app,
        "POST",
        "/api/keys",
        Some(&owner_token),
        Some(json!({"label": "Hermes on the NAS", "account_id": hermes_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{key}");
    let key_token = key["token"].as_str().unwrap().to_string();
    let key_id = key["session"]["id"].as_str().unwrap().to_string();

    let (status, me) = call(&app, "GET", "/api/me", Some(&key_token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["account"]["name"], "Hermes");
    assert_eq!(me["session"]["kind"], "app");

    // The service account can't manage others' keys.
    let (status, _) = call(
        &app,
        "GET",
        &format!("/api/keys?account_id={}", "someone-else"),
        Some(&key_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (_, keys) = call(
        &app,
        "GET",
        &format!("/api/keys?account_id={hermes_id}"),
        Some(&owner_token),
        None,
    )
    .await;
    assert_eq!(keys.as_array().unwrap().len(), 1);

    let (status, _) = call(&app, "DELETE", &format!("/api/keys/{key_id}"), Some(&owner_token), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "GET", "/api/me", Some(&key_token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn network_setting_is_for_the_owner_on_this_pc() {
    let (local, remote) = local_and_remote().await;
    let (owner_token, _) = setup_owner(&local).await;

    let (status, net) = call(&local, "GET", "/api/settings/network", Some(&owner_token), None).await;
    assert_eq!(status, StatusCode::OK, "{net}");
    assert_eq!(net["lan_access"], false);
    assert!(net["base_url"].is_null());

    let (status, net) = call(
        &local,
        "PUT",
        "/api/settings/network",
        Some(&owner_token),
        Some(json!({"lan_access": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{net}");
    assert_eq!(net["lan_access"], true);
    let (_, net) = call(&local, "GET", "/api/settings/network", Some(&owner_token), None).await;
    assert_eq!(net["lan_access"], true);

    // A member can't change it.
    let (status, sam) = call(
        &local,
        "POST",
        "/api/accounts",
        Some(&owner_token),
        Some(json!({"name": "Sam", "secret": "2468", "role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sam}");
    let (_, signed_in) = call(
        &local,
        "POST",
        "/api/auth/sign-in",
        None,
        Some(json!({"account_id": sam["id"], "secret": "2468"})),
    )
    .await;
    let sam = signed_in["token"].as_str().unwrap().to_string();
    let (status, _) = call(
        &local,
        "PUT",
        "/api/settings/network",
        Some(&sam),
        Some(json!({"lan_access": false})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Nor can anyone from another machine, even with an owner app key.
    let (_, key) = call(
        &local,
        "POST",
        "/api/keys",
        Some(&owner_token),
        Some(json!({"label": "laptop"})),
    )
    .await;
    let owner_key = key["token"].as_str().unwrap().to_string();
    let (status, _) = call(
        &remote,
        "PUT",
        "/api/settings/network",
        Some(&owner_key),
        Some(json!({"lan_access": false})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn from_the_network_only_app_keys_work() {
    let (local, remote) = local_and_remote().await;
    let (owner_token, owner_id) = setup_owner(&local).await;

    // The account list, PIN sign-in and desktop sessions stay on this PC.
    let (status, _) = call(&remote, "GET", "/api/accounts/directory", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &remote,
        "POST",
        "/api/auth/sign-in",
        None,
        Some(json!({"account_id": owner_id, "secret": "4821"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&remote, "GET", "/api/me", Some(&owner_token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&local, "GET", "/api/me", Some(&owner_token), None).await;
    assert_eq!(status, StatusCode::OK);

    // Health answers so apps can find Outpost.
    let (status, _) = call(&remote, "GET", "/api/health", None, None).await;
    assert_eq!(status, StatusCode::OK);

    // An app key for a service account works from the other machine.
    let (_, hermes) = call(
        &local,
        "POST",
        "/api/accounts",
        Some(&owner_token),
        Some(json!({"name": "Hermes", "role": "service"})),
    )
    .await;
    let (status, key) = call(
        &local,
        "POST",
        "/api/keys",
        Some(&owner_token),
        Some(json!({"label": "Hermes box", "account_id": hermes["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{key}");
    let key_token = key["token"].as_str().unwrap();
    let (status, me) = call(&remote, "GET", "/api/me", Some(key_token), None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["account"]["name"], "Hermes");
    let (status, models) = call(&remote, "GET", "/v1/models", Some(key_token), None).await;
    assert_eq!(status, StatusCode::OK, "{models}");
}

#[tokio::test]
async fn images_are_checked_before_anything_is_saved() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let (status, chat) = call(&app, "POST", "/api/chats", Some(&token), Some(json!({}))).await;
    assert_eq!(status, StatusCode::CREATED, "{chat}");
    let url = format!("/api/chats/{}/messages", chat["id"].as_str().unwrap());

    // Nothing to send.
    let (status, body) = call(&app, "POST", &url, Some(&token), Some(json!({"content": "  "}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Not an image, whatever it claims to be.
    let (status, body) = call(
        &app,
        "POST",
        &url,
        Some(&token),
        Some(json!({"content": "what is this?", "images": ["data:image/png;base64,aGVsbG8gd29ybGQ="]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "invalid");

    // Too many.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let (status, _) = call(
        &app,
        "POST",
        &url,
        Some(&token),
        Some(json!({"content": "", "images": [png, png, png, png, png]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A good image still needs a model that can see. Without one (Ollama off in the
    // test, or no vision model) the request is refused and nothing is saved.
    let (status, _) = call(&app, "POST", &url, Some(&token), Some(json!({"images": [png]}))).await;
    if status != StatusCode::OK {
        let (_, full) = call(
            &app,
            "GET",
            &format!("/api/chats/{}", chat["id"].as_str().unwrap()),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(full["messages"].as_array().unwrap().len(), 0);
    }

    // Files are private and must exist.
    let (status, _) = call(&app, "GET", "/api/files/nope", Some(&token), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/api/files/nope", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn memory_settings_and_clearing_learned_memories() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let (status, s) = call(&app, "GET", "/api/memory/settings", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["auto"], true);

    let (status, s) = call(
        &app,
        "PUT",
        "/api/memory/settings",
        Some(&token),
        Some(json!({"auto": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["auto"], false);
    let (_, s) = call(&app, "GET", "/api/memory/settings", Some(&token), None).await;
    assert_eq!(s["auto"], false);

    // Manual memories are marked as such and survive clearing the learned ones.
    let (status, m) = call(
        &app,
        "POST",
        "/api/memory",
        Some(&token),
        Some(json!({"text": "Call me Dan"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{m}");
    assert_eq!(m["source"], "manual");
    let (status, cleared) = call(&app, "DELETE", "/api/memory/auto", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["removed"], 0);
    let (_, list) = call(&app, "GET", "/api/memory", Some(&token), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn create_lists_packs_and_checks_jobs() {
    let app = app().await;
    let (owner, _) = setup_owner(&app).await;

    let (status, packs) = call(&app, "GET", "/api/packs", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK, "{packs}");
    let z = packs
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "z_image_turbo")
        .expect("Z-Image Turbo pack");
    assert_eq!(z["models"]["ready"], false);
    assert!(z.get("graph").is_none(), "the graph stays on the server");
    assert!(z["inputs"].as_array().unwrap().iter().any(|i| i["key"] == "prompt"));

    // Models not downloaded yet: refused with a clear code.
    let (status, body) = call(
        &app,
        "POST",
        "/api/jobs",
        Some(&owner),
        Some(json!({"pack": "z_image_turbo", "inputs": {"prompt": "a lighthouse"}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "models_missing");

    let (status, _) = call(
        &app,
        "POST",
        "/api/jobs",
        Some(&owner),
        Some(json!({"pack": "nope", "inputs": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Lists start empty and are private.
    let (status, jobs) = call(&app, "GET", "/api/jobs", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(jobs.as_array().unwrap().is_empty());
    let (status, gallery) = call(&app, "GET", "/api/gallery", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(gallery.as_array().unwrap().is_empty());
    let (status, _) = call(&app, "GET", "/api/media/nope", Some(&owner), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/api/gallery", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, loras) = call(&app, "GET", "/api/loras", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(loras["folder"].as_str().unwrap().contains("loras"));

    // Every built-in pack is listed, edit and video lanes included.
    let ids: Vec<_> = packs
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect();
    for id in [
        "z_image_turbo",
        "qwen_image",
        "qwen_image_edit",
        "wan22_t2v",
        "wan22_i2v",
        "qwen_image_21_edit",
        "qwen_image_21_turbo",
        "character_z_image",
    ] {
        assert!(ids.contains(&id.to_string()), "{id} missing");
    }
    // Each pack says its task once, and each task has at most one pick.
    for p in packs.as_array().unwrap() {
        assert!(p["task"].is_string(), "{}", p["id"]);
        assert!(p["recommended"].is_boolean());
    }
    for task in ["picture", "character", "edit", "video", "animate"] {
        let picks = packs
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["task"] == task && p["recommended"] == true)
            .count();
        assert!(picks <= 1, "{task}: {picks} picks");
    }

    // Nothing downloading yet.
    let (status, downloads) = call(&app, "GET", "/api/downloads", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK, "{downloads}");
    assert_eq!(downloads["running"], 0);
    assert!(downloads["items"].as_array().unwrap().is_empty());
    let (status, _) = call(&app, "GET", "/api/downloads", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Only the owner downloads models.
    let (_, sam) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(&owner),
        Some(json!({"name": "Sam", "secret": "2468", "role": "member"})),
    )
    .await;
    let (_, signed_in) = call(
        &app,
        "POST",
        "/api/auth/sign-in",
        None,
        Some(json!({"account_id": sam["id"], "secret": "2468"})),
    )
    .await;
    let sam_token = signed_in["token"].as_str().unwrap();
    let (status, _) = call(&app, "POST", "/api/packs/z_image_turbo/install", Some(sam_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The owner can open anyone's gallery; others only their own.
    let sam_id = sam["id"].as_str().unwrap();
    let (status, list) = call(&app, "GET", &format!("/api/gallery?account_id={sam_id}"), Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let (_, me) = call(&app, "GET", "/api/me", Some(&owner), None).await;
    let owner_id = me["account"]["id"].as_str().unwrap().to_string();
    let (status, _) = call(&app, "GET", &format!("/api/gallery?account_id={owner_id}"), Some(sam_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&app, "GET", &format!("/api/gallery?account_id={sam_id}"), Some(sam_token), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, packs) = call(&app, "GET", "/api/packs", Some(sam_token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(packs.as_array().unwrap().iter().all(|p| p["missing_nodes"].is_array()));

    // The line for the GPU is visible to everyone (empty here).
    let (status, line) = call(&app, "GET", "/api/queue", Some(sam_token), None).await;
    assert_eq!(status, StatusCode::OK, "{line}");
    assert!(line.as_array().unwrap().is_empty());

    // The shared gallery: everyone can read it; sharing needs an item of yours.
    let (status, shared) = call(&app, "GET", "/api/gallery/shared", Some(sam_token), None).await;
    assert_eq!(status, StatusCode::OK, "{shared}");
    assert!(shared.as_array().unwrap().is_empty());
    let (status, _) = call(&app, "POST", "/api/media/nope/share", Some(sam_token), Some(json!({"shared": true}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/api/gallery/shared", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
