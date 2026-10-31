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
    assert_eq!(engines.len(), 3);
    assert_eq!(engines[0]["id"], "ollama");
    assert_eq!(engines[1]["id"], "comfyui");
    assert_eq!(engines[2]["id"], "voice");
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
        "/api/chats/{id}/messages/{message_id}",
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
        "/api/pairing",
        "/api/pairing/{id}",
        "/api/pair/hello",
        "/api/pair/complete",
        "/api/devices",
        "/api/devices/{id}",
        "/api/connection",
        "/mcp",
        "/api/voice",
        "/api/voice/listening",
        "/api/voice/transcribe",
        "/api/voice/speak",
        "/v1/audio/speech",
        "/v1/audio/transcriptions",
        "/api/integrations",
        "/api/webhooks",
        "/api/webhooks/{id}",
        "/api/webhooks/{id}/test",
        "/api/training",
        "/api/training/trigger-check",
        "/api/training/install",
        "/api/datasets",
        "/api/datasets/{id}",
        "/api/datasets/{id}/images",
        "/api/datasets/{id}/images/{image_id}",
        "/api/datasets/{id}/import",
        "/api/datasets/{id}/caption",
        "/api/datasets/{id}/train",
        "/api/trainings/{id}",
        "/api/trainings/{id}/samples/{file}",
        "/api/trainings/{id}/use",
        "/api/characters",
        "/api/characters/{id}",
        "/api/characters/lora-files",
        "/api/diagnostics",
        "/api/diagnostics/crashes/{id}",
        "/api/updates",
        "/api/updates/check",
        "/api/updates/settings",
        "/api/media/import",
        "/api/gallery/search",
        "/api/gallery/search/status",
        "/api/gallery/search/settings",
        "/api/documents",
        "/api/documents/{id}",
        "/api/documents/{id}/file",
        "/api/chats/{id}/documents",
        "/api/personas/{id}/documents",
        "/api/helpers",
        "/api/helpers/poll",
        "/api/helpers/jobs/{id}/input/{key}",
        "/api/helpers/jobs/{id}/lora",
        "/api/helpers/jobs/{id}/progress",
        "/api/helpers/jobs/{id}/result",
        "/api/helpers/jobs/{id}/finish",
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

    // Deleting a message that isn't there (the core tests cover the rest).
    let (status, _) = call(&app, "DELETE", &format!("/api/chats/{chat_id}/messages/nope"), Some(&token), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

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
    assert_eq!(net["tailscale_access"], false);
    // Tailscale is its own switch; leaving it out keeps it as it was.
    let (_, net) = call(
        &local,
        "PUT",
        "/api/settings/network",
        Some(&owner_token),
        Some(json!({"lan_access": true, "tailscale_access": true})),
    )
    .await;
    assert_eq!(net["tailscale_access"], true);
    let (_, net) = call(
        &local,
        "PUT",
        "/api/settings/network",
        Some(&owner_token),
        Some(json!({"lan_access": true})),
    )
    .await;
    assert_eq!(net["tailscale_access"], true);
    assert!(net["phone_hosts"].is_array());

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

    // By default only what suits this machine's card is listed; `all` lists everything.
    let (status, suited) = call(&app, "GET", "/api/packs", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK, "{suited}");
    let (status, packs) = call(&app, "GET", "/api/packs?all=true", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK, "{packs}");
    let every = packs.as_array().unwrap();
    assert!(suited.as_array().unwrap().len() <= every.len());
    // Nothing downloaded here, so what's listed is exactly what fits the card.
    for p in every {
        let listed = suited.as_array().unwrap().iter().any(|s| s["id"] == p["id"]);
        assert_eq!(listed, p["fit"] == "fits", "{}", p["id"]);
    }
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

async fn call_with(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
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
        serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, value)
}

#[tokio::test]
async fn a_phone_pairs_over_the_secure_port_only() {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = temp_data_dir();
    config.ollama_url = "http://127.0.0.1:9".into();
    let state = AppState::with_pool(pool, config);
    let local = router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));

    // Network access off: no encrypted port, so no pairing yet.
    let (owner, _) = setup_owner(&local).await;
    let (status, body) = call(&local, "POST", "/api/pairing", Some(&owner), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let _ = state.tls.set(outpost_gateway::tls::TlsInfo {
        port: 7421,
        fingerprint: "ab".repeat(32),
    });
    let phone_plain = router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 77], 50000))));
    let phone = router(state.clone())
        .layer(axum::Extension(outpost_gateway::tls::Secure))
        .layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 77], 50000))));

    let (status, pairing) = call(&local, "POST", "/api/pairing", Some(&owner), None).await;
    assert_eq!(status, StatusCode::CREATED, "{pairing}");
    let code = pairing["code"].as_str().unwrap().to_string();
    assert_eq!(pairing["words"].as_array().unwrap().len(), 3);
    assert!(pairing["qr_svg"].as_str().unwrap().contains("<svg"));
    let link = pairing["link"].as_str().unwrap();
    assert!(link.starts_with("https://danite1252-ctrl.github.io/OutpostAI/pair/#v=1&"), "{link}");
    assert!(!link.split('#').next().unwrap().contains("code="), "the code stays after the #");
    assert!(pairing["link"].as_str().unwrap().contains(&format!("code={code}")));

    // Plain HTTP from the network is refused; the secure port takes it.
    let (status, _) = call(&phone_plain, "POST", "/api/pair/hello", None, Some(json!({"code": code}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&phone, "POST", "/api/pair/hello", None, Some(json!({"code": "000001"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, hello) = call(&phone, "POST", "/api/pair/hello", None, Some(json!({"code": code}))).await;
    assert_eq!(status, StatusCode::OK, "{hello}");
    let dan = hello["accounts"][0]["id"].as_str().unwrap().to_string();

    let complete = |secret: &str| json!({"code": code, "account_id": dan, "secret": secret, "device_name": "Dan's Pixel"});
    let (status, _) = call(&phone, "POST", "/api/pair/complete", None, Some(complete("0000"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, paired) = call(&phone, "POST", "/api/pair/complete", None, Some(complete("4821"))).await;
    assert_eq!(status, StatusCode::OK, "{paired}");
    let device = paired["token"].as_str().unwrap().to_string();
    assert_eq!(paired["session"]["kind"], "device");

    // The code is spent; the PC sees which phone used it.
    let (status, _) = call(&phone, "POST", "/api/pair/complete", None, Some(complete("4821"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let id = pairing["id"].as_str().unwrap();
    let (_, st) = call(&local, "GET", &format!("/api/pairing/{id}"), Some(&owner), None).await;
    assert_eq!(st["used"], true);
    assert_eq!(st["device"], "Dan's Pixel");

    // The phone token works on the secure port only.
    let (status, _) = call(&phone, "GET", "/api/me", Some(&device), None).await;
    assert_eq!(status, StatusCode::OK);
    // A paired phone can ask where else Outpost answers (e.g. Tailscale).
    let (status, conn) = call(&phone, "GET", "/api/connection", Some(&device), None).await;
    assert_eq!(status, StatusCode::OK, "{conn}");
    assert_eq!(conn["port"], 7421);
    assert!(conn["hosts"].is_array());
    let (status, _) = call(&phone_plain, "GET", "/api/me", Some(&device), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // A paired device can pair another one (a server with no screen has no other way),
    // but only with the PIN again.
    let (status, body) = call(&phone, "POST", "/api/pairing", Some(&device), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "pin_required");
    let (status, second) = call_with(&phone, "POST", "/api/pairing", Some(&device), &[("x-outpost-pin", "4821")], None).await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second_id = second["id"].as_str().unwrap();
    let (status, _) = call(&phone, "GET", &format!("/api/pairing/{second_id}"), Some(&device), None).await;
    assert_eq!(status, StatusCode::OK);

    // An owner's paired device may change network access with the PIN, but not turn it
    // off (that would cut it off); app keys can't at all.
    let net = |on: bool| json!({"lan_access": on});
    let (status, _) = call(&phone, "PUT", "/api/settings/network", Some(&device), Some(net(true))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let pin = [("x-outpost-pin", "4821")];
    let (status, body) = call_with(&phone, "PUT", "/api/settings/network", Some(&device), &pin, Some(net(true))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["lan_access"], true);
    let (status, _) = call_with(&phone, "PUT", "/api/settings/network", Some(&device), &pin, Some(net(false))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Risky actions from a phone ask for the PIN again.
    let key = json!({"label": "From phone"});
    let (status, body) = call(&phone, "POST", "/api/keys", Some(&device), Some(key.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "pin_required");
    let (status, _) = call_with(&phone, "POST", "/api/keys", Some(&device), &[("x-outpost-pin", "0000")], Some(key.clone())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = call_with(&phone, "POST", "/api/keys", Some(&device), &[("x-outpost-pin", "4821")], Some(key)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    // An app key can't make more keys or manage accounts (it may travel over plain HTTP).
    let app_key = body["token"].as_str().unwrap().to_string();
    let (status, _) = call(&local, "POST", "/api/keys", Some(&app_key), Some(json!({"label": "x"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &local,
        "POST",
        "/api/accounts",
        Some(&app_key),
        Some(json!({"name": "Mallory", "secret": "2468", "role": "owner"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Paired phones are listed and can be unpaired.
    let (_, mine) = call(&phone, "GET", "/api/devices", Some(&device), None).await;
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["current"], true);
    assert_eq!(mine[0]["label"], "Dan's Pixel");
    let (status, all) = call(&local, "GET", "/api/devices?all=true", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all[0]["account_name"], "Dan");
    let sid = mine[0]["id"].as_str().unwrap();
    let (status, _) = call(&local, "DELETE", &format!("/api/devices/{sid}"), Some(&owner), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&phone, "GET", "/api/me", Some(&device), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn training_studio_datasets_captions_and_checks() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let t = Some(token.as_str());
    const PNG: &str = "iVBORw0KGgoAAAAA";

    // Real words and names are flagged, with a safer suggestion.
    let (status, c) = call(&app, "GET", "/api/training/trigger-check?word=kat", t, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(c["ok"], false);
    let suggestion = c["suggestion"].as_str().unwrap().to_string();
    let (_, c) = call(&app, "GET", &format!("/api/training/trigger-check?word={suggestion}"), t, None).await;
    assert_eq!(c["ok"], true);

    // Point at a folder with no ai-toolkit so nothing can really train here.
    let missing = temp_data_dir().join("no-toolkit");
    let (status, st) = call(&app, "PUT", "/api/training", t, Some(json!({"toolkit_dir": missing}))).await;
    assert_eq!(status, StatusCode::OK, "{st}");
    assert!(st["toolkit_problem"].is_string());
    // Outpost can install it itself (not done here: it's gigabytes).
    assert_eq!(st["install"]["installed"], false, "{st}");
    assert_eq!(st["install"]["installing"], false);
    assert!(st["install"]["dir"].as_str().unwrap().contains("ai-toolkit"));
    assert!(!st["install"]["pinned_version"].as_str().unwrap().is_empty());

    let (status, d) = call(&app, "POST", "/api/datasets", t, Some(json!({"name": "Kat"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{d}");
    assert_eq!(d["trigger_check"]["ok"], true, "a trigger made from the name is safe: {d}");
    let id = d["id"].as_str().unwrap().to_string();
    let trigger = d["trigger_word"].as_str().unwrap().to_string();

    let (status, _) = call(&app, "POST", &format!("/api/datasets/{id}/images"), t, Some(json!({"image": "aGVsbG8="}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "not a picture");
    let mut first = String::new();
    for i in 0..4 {
        let caption = if i == 0 { format!("{trigger}, a woman smiling") } else { String::new() };
        let (status, img) = call(
            &app,
            "POST",
            &format!("/api/datasets/{id}/images"),
            t,
            Some(json!({"image": format!("data:image/png;base64,{PNG}"), "caption": caption})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{img}");
        if i == 0 {
            assert_eq!(img["caption"], "a woman smiling", "the trigger is added at training time");
            first = img["id"].as_str().unwrap().to_string();
        }
    }
    // A folder on this PC: pictures plus their captions.
    let folder = temp_data_dir().join("kat");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("one.png"), [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]).unwrap();
    std::fs::write(folder.join("one.txt"), "on a bench").unwrap();
    let (status, r) = call(&app, "POST", &format!("/api/datasets/{id}/import"), t, Some(json!({"folder": folder}))).await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["added"], 1);

    let (status, img) = call(
        &app,
        "PATCH",
        &format!("/api/datasets/{id}/images/{first}"),
        t,
        Some(json!({"caption": "a woman in a red coat."})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(img["caption"], "a woman in a red coat");
    let (_, d) = call(&app, "GET", &format!("/api/datasets/{id}"), t, None).await;
    assert_eq!(d["image_count"], 5);
    assert_eq!(d["captioned_count"], 2);
    // Test prompts follow the captions ("a woman ...") and start with the trigger.
    let suggested = d["suggested_prompts"].as_array().unwrap();
    assert_eq!(suggested.len(), 3);
    assert!(suggested[0].as_str().unwrap().starts_with(&format!("{trigger}, a woman in a red coat")), "{d}");
    let (status, _) = call(&app, "GET", &format!("/api/datasets/{id}/images/{first}"), t, None).await;
    assert_eq!(status, StatusCode::OK);

    // No vision model (when no Ollama is running on this PC) is explained; with a real
    // Ollama about, captioning starts instead. Either way, no ai-toolkit is explained.
    let (status, e) = call(&app, "POST", &format!("/api/datasets/{id}/caption"), t, Some(json!({}))).await;
    assert!(
        (status == StatusCode::BAD_REQUEST && e["error"]["code"] == "no_vision_model") || status == StatusCode::ACCEPTED,
        "{status} {e}"
    );
    let (status, e) = call(&app, "POST", &format!("/api/datasets/{id}/train"), t, Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(e["error"]["code"], "toolkit_missing");
    let (status, _) = call(&app, "POST", &format!("/api/datasets/{id}/train"), t, Some(json!({"steps": 10}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // No characters yet; /draw explains what's missing instead of chatting.
    let (_, chars) = call(&app, "GET", "/api/characters", t, None).await;
    assert_eq!(chars, json!([]));
    let (_, loras) = call(&app, "GET", "/api/loras", t, None).await;
    assert_eq!(loras["characters"], json!([]));
    let (_, chat) = call(&app, "POST", "/api/chats", t, Some(json!({}))).await;
    let url = format!("/api/chats/{}/messages", chat["id"].as_str().unwrap());
    let (status, e) = call(&app, "POST", &url, t, Some(json!({"content": "/draw a lighthouse at dusk"}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{e}");
    let (status, _) = call(&app, "POST", &url, t, Some(json!({"content": "/draw"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(&app, "DELETE", &format!("/api/datasets/{id}"), t, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "GET", &format!("/api/datasets/{id}"), t, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mcp_speaks_json_rpc_with_an_app_key() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let t = Some(token.as_str());
    let rpc = |id: i64, method: &str, params: Value| json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});

    // No key: refused like the rest of the API.
    let (status, _) = call(&app, "POST", "/mcp", None, Some(rpc(1, "ping", json!({})))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, init) = call(&app, "POST", "/mcp", t, Some(rpc(1, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}})))).await;
    assert_eq!(status, StatusCode::OK, "{init}");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "outpost");

    // A notification gets no body.
    let (status, body) = call(&app, "POST", "/mcp", t, Some(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body, Value::Null);

    let (_, list) = call(&app, "POST", "/mcp", t, Some(rpc(2, "tools/list", json!({})))).await;
    let names: Vec<String> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"draw".to_string()) && names.contains(&"list_characters".to_string()));

    let (_, r) = call(&app, "POST", "/mcp", t, Some(rpc(3, "tools/call", json!({"name": "list_characters", "arguments": {}})))).await;
    assert_eq!(r["result"]["isError"], false);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("No characters"));

    // Nothing downloaded here: draw explains instead of hanging.
    let (_, r) = call(&app, "POST", "/mcp", t, Some(rpc(4, "tools/call", json!({"name": "draw", "arguments": {"prompt": "a lighthouse"}})))).await;
    assert_eq!(r["result"]["isError"], true, "{r}");

    let (_, r) = call(&app, "POST", "/mcp", t, Some(rpc(5, "tools/call", json!({"name": "remember", "arguments": {"text": "Dan keeps bees"}})))).await;
    assert_eq!(r["result"]["isError"], false, "{r}");
    let (_, mem) = call(&app, "GET", "/api/memory", t, None).await;
    assert_eq!(mem[0]["text"], "Dan keeps bees");

    let (_, r) = call(&app, "POST", "/mcp", t, Some(rpc(6, "no/such", json!({})))).await;
    assert_eq!(r["error"]["code"], -32601);
}

#[tokio::test]
async fn webhooks_are_set_up_by_people_and_signed() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let t = Some(token.as_str());
    let (status, list) = call(&app, "GET", "/api/webhooks", t, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list["events"].as_array().unwrap().iter().any(|e| e == "job.finished"));

    let (status, _) = call(&app, "POST", "/api/webhooks", t, Some(json!({"url": "not a url", "events": ["job.finished"]}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Nothing listens on port 9: made fine, and the test ping says it couldn't connect.
    let (status, made) = call(&app, "POST", "/api/webhooks", t, Some(json!({"url": "http://127.0.0.1:9/hook", "events": ["job.finished", "bogus"]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert!(made["secret"].as_str().unwrap().starts_with("whsec_"));
    assert_eq!(made["webhook"]["events"], json!(["job.finished"]));
    let id = made["webhook"]["id"].as_str().unwrap().to_string();
    let (_, r) = call(&app, "POST", &format!("/api/webhooks/{id}/test"), t, None).await;
    assert_eq!(r["status"], 0, "{r}");
    let (_, list) = call(&app, "GET", "/api/webhooks", t, None).await;
    assert!(list["webhooks"][0].get("secret").is_none());
    assert_eq!(list["webhooks"][0]["last_status"], 0);

    // App keys can't make webhooks.
    let (_, key) = call(&app, "POST", "/api/keys", t, Some(json!({"label": "script"}))).await;
    let k = key["token"].as_str().unwrap().to_string();
    let (status, _) = call(&app, "POST", "/api/webhooks", Some(&k), Some(json!({"url": "http://127.0.0.1:9/x", "events": ["job.finished"]}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = call(&app, "DELETE", &format!("/api/webhooks/{id}"), t, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn voice_explains_when_it_isnt_running() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let t = Some(token.as_str());
    let (status, v) = call(&app, "GET", "/api/voice", t, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["default_voice"], "af_heart");
    // Listening models: small until another is chosen; one that isn't downloaded needs Voice.
    assert_eq!(v["listening"], "small", "{v}");
    assert_eq!(v["listening_in_use"], "small");
    let ids: Vec<&str> = v["listening_models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["base", "small", "large-v3-turbo"]);
    let (status, _) = call(&app, "PUT", "/api/voice/listening", t, Some(json!({"model": "huge"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, e) = call(&app, "PUT", "/api/voice/listening", t, Some(json!({"model": "large-v3-turbo"}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("Install Voice"));
    let (_, v) = call(&app, "GET", "/api/voice", t, None).await;
    assert_eq!(v["listening"], "small", "not chosen when it couldn't be got");
    let (status, _) = call(&app, "POST", "/api/voice/transcribe", t, Some(json!({"audio": "not base64!"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&app, "POST", "/api/voice/speak", t, Some(json!({"text": "```\ncode\n```"}))).await;
    // "(code)" is something to say, so it reaches the (absent) voice service, unless
    // one happens to run on this machine.
    assert!(status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::OK, "{status}");
    let (status, _) = call(&app, "POST", "/api/voice/speak", t, Some(json!({"text": "  "}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Personas keep a voice and the read-aloud switch.
    let (_, personas) = call(&app, "GET", "/api/personas", t, None).await;
    let id = personas[0]["id"].as_str().unwrap().to_string();
    let (status, p) = call(&app, "PATCH", &format!("/api/personas/{id}"), t, Some(json!({"voice": "bm_george", "speak_replies": true}))).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!((p["voice"].as_str(), p["speak_replies"].as_bool()), (Some("bm_george"), Some(true)));
    let (status, _) = call(&app, "PATCH", &format!("/api/personas/{id}"), t, Some(json!({"voice": "../../x"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn diagnostics_and_update_settings() {
    let app = app().await;
    let (token, _) = setup_owner(&app).await;
    let t = Some(token.as_str());

    let (status, d) = call(&app, "GET", "/api/diagnostics", t, None).await;
    assert_eq!(status, StatusCode::OK, "{d}");
    assert!(d["crash_dir"].as_str().unwrap().ends_with("crash-reports"));
    assert_eq!(d["crashes"].as_array().unwrap().len(), 0);
    let (status, _) = call(&app, "GET", "/api/diagnostics/crashes/..%2F..%2Foutpost.db", t, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/api/diagnostics", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Update checks are on by default and can be switched off (no network call is made here).
    let (status, u) = call(&app, "GET", "/api/updates", t, None).await;
    assert_eq!(status, StatusCode::OK, "{u}");
    assert_eq!(u["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(u["enabled"], true);
    assert!(u["update"].is_null());
    let (status, u) = call(&app, "PUT", "/api/updates/settings", t, Some(json!({"enabled": false}))).await;
    assert_eq!(status, StatusCode::OK, "{u}");
    assert_eq!(u["enabled"], false);
    // A PC never installs by itself.
    assert_eq!(u["auto_install"], false);
}

#[tokio::test]
async fn a_server_installs_updates_by_itself_until_turned_off() {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = temp_data_dir();
    config.ollama_url = "http://127.0.0.1:9".into();
    config.server = true;
    let app = router(AppState::with_pool(pool, config)).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));
    let (token, _) = setup_owner(&app).await;
    let t = Some(token.as_str());
    let (_, u) = call(&app, "GET", "/api/updates", t, None).await;
    assert_eq!(u["auto_install"], true, "{u}");
    let (_, u) = call(&app, "PUT", "/api/updates/settings", t, Some(json!({"enabled": true, "auto_install": false}))).await;
    assert_eq!(u["auto_install"], false, "{u}");
    assert_eq!(u["enabled"], true);
    // Leaving it out changes nothing.
    let (_, u) = call(&app, "PUT", "/api/updates/settings", t, Some(json!({"enabled": true}))).await;
    assert_eq!(u["auto_install"], false);
}

#[tokio::test]
async fn a_server_is_set_up_from_another_device_with_its_setup_code() {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = temp_data_dir();
    std::fs::create_dir_all(&config.data_dir).unwrap();
    config.ollama_url = "http://127.0.0.1:9".into();
    let state = AppState::with_pool(pool, config);
    let remote = router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 77], 50000))));

    // No code, or a wrong one: refused.
    let body = |code: &str| json!({"name": "Dan", "secret": "4826", "setup_code": code, "device": "Dan's desktop"});
    let (status, _) = call(&remote, "POST", "/api/setup/owner", None, Some(json!({"name": "Dan", "secret": "4826"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    state.setup.refresh(&state).await;
    let text = std::fs::read_to_string(outpost_gateway::setup_code::file(&state.config.data_dir)).unwrap();
    let code = text.lines().next().unwrap().trim_start_matches("Outpost setup code: ").to_string();
    assert_eq!(code.len(), 9, "{text}");
    let (status, _) = call(&remote, "POST", "/api/setup/owner", None, Some(body("0000-0000"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The right code makes the owner, signed in as a paired device; the code is used up.
    let (status, v) = call(&remote, "POST", "/api/setup/owner", None, Some(body(&code))).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["session"]["kind"], "device");
    assert_eq!(v["session"]["label"], "Dan's desktop");
    assert!(!outpost_gateway::setup_code::file(&state.config.data_dir).exists());
    let (status, _) = call(&remote, "POST", "/api/setup/owner", None, Some(body(&code))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

async fn upload(app: &Router, uri: &str, token: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn loras_i_already_have_can_be_uploaded() {
    let app = app().await;
    let (owner, dan) = setup_owner(&app).await;
    // A tiny but well-formed safetensors file: header length, then a JSON header.
    let header = br#"{"__metadata__":{"ss_network_module":"lora"}}"#;
    let mut lora = (header.len() as u64).to_le_bytes().to_vec();
    lora.extend_from_slice(header);
    lora.extend_from_slice(&[0u8; 64]);

    let (status, body) = upload(
        &app,
        "/api/loras?name=Kat%20v2.safetensors&character=Kat&trigger=k4tt&subject=woman",
        &owner,
        lora.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    // In my own LoRA folder.
    let kat = format!("{dan}/Kat_v2.safetensors");
    assert_eq!(body["file"], kat.as_str());
    assert_eq!(body["character"]["trigger_word"], "k4tt");

    let (_, list) = call(&app, "GET", "/api/loras", Some(&owner), None).await;
    assert!(list["loras"].as_array().unwrap().iter().any(|l| l == kat.as_str()), "{list}");
    assert!(list["characters"].as_array().unwrap().iter().any(|c| c["name"] == "Kat"));
    // No half-finished files are left behind or listed.
    assert_eq!(list["loras"].as_array().unwrap().len(), 1);

    // Same name again: asks first; replace=true overwrites.
    let (status, body) = upload(&app, "/api/loras?name=Kat_v2.safetensors", &owner, lora.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "exists");
    let (status, _) = upload(&app, "/api/loras?name=Kat_v2.safetensors&replace=true", &owner, lora.clone()).await;
    assert_eq!(status, StatusCode::CREATED);

    // Not a safetensors file, or not named like one.
    let (status, _) = upload(&app, "/api/loras?name=fake.safetensors", &owner, b"PK\x03\x04 a zip file".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = upload(&app, "/api/loras?name=kat.ckpt", &owner, lora.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // A character needs its trigger word.
    let (status, _) = upload(&app, "/api/loras?name=b.safetensors&character=Bob", &owner, lora).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, list) = call(&app, "GET", "/api/loras", Some(&owner), None).await;
    assert_eq!(list["loras"].as_array().unwrap().len(), 1, "{list}");

    // Like personas, my LoRAs are mine: Sam neither sees nor uses them.
    let (status, sam) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(&owner),
        Some(json!({"name": "Sam", "role": "member", "secret": "2468"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sam}");
    let (_, signed) = call(
        &app,
        "POST",
        "/api/auth/sign-in",
        None,
        Some(json!({"account_id": sam["id"], "secret": "2468"})),
    )
    .await;
    let sam_token = signed["token"].as_str().unwrap().to_string();
    let (_, theirs) = call(&app, "GET", "/api/loras", Some(&sam_token), None).await;
    assert!(theirs["loras"].as_array().unwrap().is_empty(), "{theirs}");
    assert!(theirs["characters"].as_array().unwrap().is_empty());
    // (Jobs check LoRAs against this same list, so Sam can't name Dan's either.)
    let (_, files) = call(&app, "GET", "/api/characters/lora-files", Some(&sam_token), None).await;
    assert!(files.as_array().unwrap().is_empty(), "{files}");
    // Sam's upload of the same name is Sam's own, beside Dan's.
    let mut other = 2u64.to_le_bytes().to_vec();
    other.extend(b"{}");
    let (status, body) = upload(&app, "/api/loras?name=Kat_v2.safetensors", &sam_token, other).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (_, theirs) = call(&app, "GET", "/api/loras", Some(&sam_token), None).await;
    assert_eq!(theirs["loras"].as_array().unwrap().len(), 1);
    let (_, mine) = call(&app, "GET", "/api/loras", Some(&owner), None).await;
    assert_eq!(mine["loras"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn pictures_come_over_from_another_outpost_once() {
    let app = app().await;
    let (owner, _) = setup_owner(&app).await;
    // A 1x1 PNG.
    let png = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
    )
    .unwrap();
    let uri = "/api/media/import?name=old.png&prompt=Kat%20at%20the%20beach&pack_id=z_image_turbo&seed=42&created_at=1761470100";
    let (status, body) = upload(&app, uri, &owner, png.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["added"], true);
    assert_eq!(body["media"]["created_at"], 1761470100);
    assert_eq!(body["media"]["prompt"], "Kat at the beach");
    assert_eq!(body["media"]["seed"], 42);
    // The same item again isn't doubled.
    let (status, body) = upload(&app, uri, &owner, png.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["added"], false);
    let (_, gallery) = call(&app, "GET", "/api/gallery", Some(&owner), None).await;
    let items = gallery.as_array().or_else(|| gallery["items"].as_array()).cloned().unwrap_or_default();
    assert_eq!(items.len(), 1, "{gallery}");
    // Only pictures and videos.
    let (status, _) = upload(&app, "/api/media/import?name=notes.txt", &owner, b"hi".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invites_add_people_and_reset_pins_from_their_own_devices() {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = temp_data_dir();
    config.ollama_url = "http://127.0.0.1:9".into();
    let state = AppState::with_pool(pool, config);
    let local = router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));
    let phone = router(state.clone())
        .layer(axum::Extension(outpost_gateway::tls::Secure))
        .layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 77], 50000))));
    let (owner, _) = setup_owner(&local).await;
    let _ = state.tls.set(outpost_gateway::tls::TlsInfo { port: 7421, fingerprint: "ab".repeat(32) });

    // The owner invites Sam: a code, QR and words, valid a day.
    let (status, inv) = call(&local, "POST", "/api/invites", Some(&owner), Some(json!({"name": "Sam"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{inv}");
    assert_eq!(inv["invite"]["kind"], "join");
    assert!(inv["link"].as_str().unwrap().ends_with("&invite=join"));
    let code = inv["code"].as_str().unwrap().to_string();

    // On Sam's phone: the code says who it's for (no account list), and Sam picks a PIN.
    let (status, hello) = call(&phone, "POST", "/api/pair/hello", None, Some(json!({"code": code}))).await;
    assert_eq!(status, StatusCode::OK, "{hello}");
    assert_eq!(hello["invite"]["name"], "Sam");
    assert!(hello["accounts"].as_array().unwrap().is_empty());
    let accept = |pin: &str| json!({"code": code, "secret": pin, "device_name": "Sam's phone"});
    let (status, _) = call(&phone, "POST", "/api/pair/complete", None, Some(json!({"code": code, "account_id": "x", "secret": "1", "device_name": "x"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "an invite isn't a pairing code");
    let (status, sam) = call(&phone, "POST", "/api/pair/accept", None, Some(accept("2468"))).await;
    assert_eq!(status, StatusCode::OK, "{sam}");
    assert_eq!(sam["account"]["name"], "Sam");
    assert_eq!(sam["account"]["role"], "member");
    assert_eq!(sam["session"]["kind"], "device");
    let sam_token = sam["token"].as_str().unwrap().to_string();
    let (status, _) = call(&phone, "GET", "/api/me", Some(&sam_token), None).await;
    assert_eq!(status, StatusCode::OK);

    // Members can't invite.
    let (status, _) = call_with(&phone, "POST", "/api/invites", Some(&sam_token), &[("x-outpost-pin", "2468")], Some(json!({"name": "Eve"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Sam forgets the PIN: a reset invite; the new PIN signs the old devices out.
    let sam_id = sam["account"]["id"].as_str().unwrap();
    let (status, reset) = call(&local, "POST", "/api/invites", Some(&owner), Some(json!({"account_id": sam_id}))).await;
    assert_eq!(status, StatusCode::CREATED, "{reset}");
    assert_eq!(reset["invite"]["kind"], "reset");
    let code = reset["code"].as_str().unwrap();
    let (status, again) = call(&phone, "POST", "/api/pair/accept", None, Some(json!({"code": code, "secret": "97531", "device_name": "Sam's new phone"}))).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    let (status, _) = call(&phone, "GET", "/api/me", Some(&sam_token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the old phone is signed out");
}

#[tokio::test]
async fn a_shared_pc_checks_each_persons_pin() {
    let app = app().await;
    let (owner, _) = setup_owner(&app).await;
    let check = |pin: &'static str| {
        let app = app.clone();
        let owner = owner.clone();
        async move { call_with(&app, "POST", "/api/me/check-pin", Some(&owner), &[("x-outpost-pin", pin)], None).await.0 }
    };
    assert_eq!(check("4821").await, StatusCode::NO_CONTENT);
    assert_eq!(check("0000").await, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&app, "POST", "/api/me/check-pin", Some(&owner), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn personas_are_sent_as_copies_without_my_memories() {
    let app = app().await;
    let (owner, _) = setup_owner(&app).await;
    let (_, sam) = call(&app, "POST", "/api/accounts", Some(&owner), Some(json!({"name": "Sam", "role": "member", "secret": "2468"}))).await;
    let sam_id = sam["id"].as_str().unwrap().to_string();
    // Dan's list starts with the default Assistant.
    let (_, _) = call(&app, "GET", "/api/personas", Some(&owner), None).await;
    let (status, family) = call(
        &app,
        "POST",
        "/api/personas",
        Some(&owner),
        Some(json!({"name": "Family helper", "system_prompt": "I am talking with {name}. Our family: Dan, Sam.", "voice": "af_heart"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{family}");
    let fid = family["id"].as_str().unwrap();
    let (_, _) = call(&app, "POST", "/api/memory", Some(&owner), Some(json!({"text": "Dan's secret", "persona_id": fid}))).await;

    // Everyone's names, for choosing who gets it.
    let (status, people) = call(&app, "GET", "/api/people", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(people.as_array().unwrap().len(), 2);

    let (status, sent) = call(&app, "POST", &format!("/api/personas/{fid}/send"), Some(&owner), Some(json!({"account_ids": [sam_id]}))).await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert_eq!(sent[0]["persona_name"], "Family helper");
    // Sending the Assistant again: Sam already has one called that.
    let (_, mine) = call(&app, "GET", "/api/personas", Some(&owner), None).await;
    let assistant = mine.as_array().unwrap().iter().find(|p| p["name"] == "Assistant").unwrap()["id"].as_str().unwrap().to_string();
    let (_, sent) = call(&app, "POST", &format!("/api/personas/{assistant}/send"), Some(&owner), Some(json!({"account_ids": [sam_id]}))).await;
    assert_eq!(sent[0]["persona_name"], "Assistant (from Dan)");

    // Sam's copy has the prompt and voice, and none of Dan's memories.
    let (_, signed) = call(&app, "POST", "/api/auth/sign-in", None, Some(json!({"account_id": sam_id, "secret": "2468"}))).await;
    let sam_token = signed["token"].as_str().unwrap();
    let (_, theirs) = call(&app, "GET", "/api/personas", Some(sam_token), None).await;
    let copy = theirs.as_array().unwrap().iter().find(|p| p["name"] == "Family helper").unwrap();
    assert_eq!(copy["voice"], "af_heart");
    assert!(copy["system_prompt"].as_str().unwrap().contains("{name}"));
    let (_, memories) = call(&app, "GET", "/api/memory", Some(sam_token), None).await;
    assert!(memories.as_array().unwrap().iter().all(|m| m["text"] != "Dan's secret"));

    // Sharing: Dan shares a persona with everyone; Sam sees it (as Dan's), chats with it,
    // can't change it, and can make his own copy.
    let (_, tutor) = call(&app, "POST", "/api/personas", Some(&owner), Some(json!({"name": "Tutor", "shared": true}))).await;
    let tid = tutor["id"].as_str().unwrap().to_string();
    let (_, theirs) = call(&app, "GET", "/api/personas", Some(sam_token), None).await;
    let shared = theirs.as_array().unwrap().iter().find(|p| p["id"] == tid.as_str()).expect("Sam sees it");
    assert_eq!(shared["owner_name"], "Dan");
    let (status, chat) = call(&app, "POST", "/api/chats", Some(sam_token), Some(json!({"persona_id": tid}))).await;
    assert_eq!(status, StatusCode::CREATED, "{chat}");
    let (status, _) = call(&app, "PATCH", &format!("/api/personas/{tid}"), Some(sam_token), Some(json!({"name": "Mine now"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, mine) = call(&app, "POST", &format!("/api/personas/{tid}/copy"), Some(sam_token), None).await;
    assert_eq!(status, StatusCode::CREATED, "{mine}");
    assert_eq!(mine["account_id"], sam_id.as_str());
    assert_eq!(mine["shared"], false);
    // Unshared: it's gone from Sam's list.
    let (_, _) = call(&app, "PATCH", &format!("/api/personas/{tid}"), Some(&owner), Some(json!({"shared": false}))).await;
    let (_, theirs) = call(&app, "GET", "/api/personas", Some(sam_token), None).await;
    assert!(theirs.as_array().unwrap().iter().all(|p| p["id"] != tid.as_str()));
}
