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
    assert!(body["data"].as_array().unwrap().is_empty());

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
