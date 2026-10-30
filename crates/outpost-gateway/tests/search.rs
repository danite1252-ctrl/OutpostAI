//! Gallery search: pictures found by their prompts and by what's in them.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use outpost_gateway::{router, AppState, Config};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn app() -> (Router, AppState) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = std::env::temp_dir().join(format!(
        "outpost-search-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    // No Ollama: no embedding model, so passages are found by their words.
    config.ollama_url = "http://127.0.0.1:9".into();
    let state = AppState::with_pool(pool, config);
    (router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000)))), state)
}

async fn send(app: &Router, method: &str, uri: &str, token: Option<&str>, kind: &str, body: Vec<u8>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header(header::CONTENT_TYPE, kind);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let resp = app.clone().oneshot(req.body(Body::from(body)).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, value)
}

async fn call(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let data = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
    send(app, method, uri, Some(token), "application/json", data).await
}

async fn owner(app: &Router) -> String {
    let (status, body) =
        send(app, "POST", "/api/setup/owner", None, "application/json", json!({"name": "Dan", "secret": "4821"}).to_string().into_bytes()).await;
    assert_eq!(status, StatusCode::CREATED);
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn the_gallery_is_searched_by_prompt_and_description() {
    let (app, state) = app().await;
    let dan = owner(&app).await;
    let png = |tag: &str| [b"\x89PNG\r\n\x1a\n".to_vec(), tag.as_bytes().to_vec()].concat();
    let import = |name: &str, prompt: &str, tag: &str| {
        let uri = format!("/api/media/import?name={name}&prompt={}", prompt.replace(' ', "%20"));
        let app = app.clone();
        let dan = dan.clone();
        let body = png(tag);
        async move {
            let (status, m) = send(&app, "POST", &uri, Some(&dan), "application/octet-stream", body).await;
            assert_eq!(status, StatusCode::OK, "{m}");
            m["media"]["id"].as_str().unwrap().to_string()
        }
    };
    let dog = import("a.png", "golden retriever, shoreline, sunset", "1").await;
    let city = import("b.png", "neon city street at night", "2").await;
    let bike = import("c.png", "", "3").await;
    // What the vision model saw (normally written in the background).
    outpost_core::media_index::set_description(&state.pool, &dog, "llava", "A dog running along a sandy beach at sunset.").await.unwrap();
    outpost_core::media_index::set_description(&state.pool, &bike, "llava", "A red bicycle leaning on a brick wall.").await.unwrap();

    let ids = |v: &Value| v.as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let (status, found) = call(&app, "GET", "/api/gallery/search?q=dog%20on%20the%20beach", &dan, None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(ids(&found), vec![dog.clone()]);
    assert!(found[0]["description"].as_str().unwrap().contains("sandy beach"));
    // Found by its description alone (it had no prompt).
    let (_, found) = call(&app, "GET", "/api/gallery/search?q=bicycle", &dan, None).await;
    assert_eq!(ids(&found), vec![bike]);
    // And by prompt alone.
    let (_, found) = call(&app, "GET", "/api/gallery/search?q=neon", &dan, None).await;
    assert_eq!(ids(&found), vec![city]);
    let (_, found) = call(&app, "GET", "/api/gallery/search?q=spaceship", &dan, None).await;
    assert!(found.as_array().unwrap().is_empty());

    // Progress: 2 of 3 described; the owner can turn describing off.
    let (_, st) = call(&app, "GET", "/api/gallery/search/status", &dan, None).await;
    assert_eq!(st["described"], 2, "{st}");
    assert_eq!(st["total"], 3);
    assert_eq!(st["enabled"], true);
    let (status, st) = call(&app, "PUT", "/api/gallery/search/settings", &dan, Some(json!({"enabled": false}))).await;
    assert_eq!(status, StatusCode::OK, "{st}");
    assert_eq!(st["enabled"], false);

    // Someone else's search never sees my pictures.
    let other = outpost_core::accounts::create(
        &state.pool,
        outpost_core::accounts::NewAccount { name: "Sam".into(), role: outpost_core::accounts::Role::Member, secret: Some("4821".into()) },
    )
    .await
    .unwrap();
    let items = outpost_core::media_index::all(&state.pool, &other.id, None).await.unwrap();
    assert!(outpost_core::media_index::search(&items, "dog", None, 10).is_empty());
    let _ = std::fs::remove_dir_all(&state.config.data_dir);
}
