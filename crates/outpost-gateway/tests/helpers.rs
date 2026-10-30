//! Helper PCs: an owner's PC takes Create jobs this server can't run, sends the results
//! back into the gallery, and can be paused or cancelled.

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
        "outpost-helpers-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    config.ollama_url = "http://127.0.0.1:9".into();
    let state = AppState::with_pool(pool, config);
    (router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000)))), state)
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, kind: &str, body: Vec<u8>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, kind)
        .body(Body::from(body))
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

async fn call(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let data = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
    send(app, method, uri, token, "application/json", data).await
}

async fn owner(app: &Router) -> String {
    let req = Request::builder()
        .method("POST")
        .uri("/api/setup/owner")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"name": "Dan", "secret": "4821"}).to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    body["token"].as_str().unwrap().to_string()
}

const PACK: &str = "z_image_turbo";

fn poll(paused: bool) -> Value {
    json!({"vram_mb": 24576, "ram_mb": 65536, "ready_packs": [PACK], "paused": paused})
}

#[tokio::test]
async fn a_helper_pc_runs_what_the_server_cannot() {
    let (app, _state) = app().await;
    let dan = owner(&app).await;

    // Nothing downloaded here and no helper: refused, as before.
    let job = json!({"pack": PACK, "inputs": {"prompt": "a lighthouse"}});
    let (status, body) = call(&app, "POST", "/api/jobs", &dan, Some(job.clone())).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // A helper with a big card and the pack downloaded checks in: nothing to do yet.
    let (status, body) = call(&app, "POST", "/api/helpers/poll", &dan, Some(poll(false))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["job"].is_null());
    let (_, helpers) = call(&app, "GET", "/api/helpers", &dan, None).await;
    let name = helpers[0]["name"].as_str().unwrap().to_string();
    assert_eq!(helpers[0]["online"], true);

    // Create now lists the pack, running on the helper, and takes the job.
    let (_, packs) = call(&app, "GET", "/api/packs", &dan, None).await;
    let z = packs.as_array().unwrap().iter().find(|p| p["id"] == PACK).expect("listed");
    assert_eq!(z["runs_on"], name.as_str());
    let (status, made) = call(&app, "POST", "/api/jobs", &dan, Some(job.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();

    // Paused (gaming): no job, and the queue says what it waits for.
    let (_, body) = call(&app, "POST", "/api/helpers/poll", &dan, Some(poll(true))).await;
    assert!(body["job"].is_null());
    let (_, line) = call(&app, "GET", "/api/queue", &dan, None).await;
    assert_eq!(line[0]["waiting_for"], name.as_str(), "{line}");
    let (_, view) = call(&app, "GET", &format!("/api/jobs/{id}"), &dan, None).await;
    assert_eq!(view["waiting_for"], name.as_str());

    // Back on: it gets the job.
    let (_, body) = call(&app, "POST", "/api/helpers/poll", &dan, Some(poll(false))).await;
    assert_eq!(body["job"]["job_id"], id.as_str(), "{body}");
    assert_eq!(body["job"]["inputs"]["prompt"], "a lighthouse");
    assert!(body["job"]["lora"].is_null());
    // Nothing else to hand out.
    let (_, again) = call(&app, "POST", "/api/helpers/poll", &dan, Some(poll(false))).await;
    assert!(again["job"].is_null());

    // Progress shows wherever the job is looked at.
    let progress = json!({"phase": "Generating", "step": 3, "steps": 8, "percent": null});
    let (status, reply) = call(&app, "POST", &format!("/api/helpers/jobs/{id}/progress"), &dan, Some(progress)).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["cancel"], false);
    let (_, view) = call(&app, "GET", &format!("/api/jobs/{id}"), &dan, None).await;
    assert_eq!(view["status"], "running");
    assert_eq!(view["progress"]["step"], 3);
    let (_, line) = call(&app, "GET", "/api/queue", &dan, None).await;
    assert_eq!(line[0]["on"], name.as_str());
    assert_eq!(view["on"], name.as_str());
    assert_eq!(line[0]["progress"]["phase"], "Generating");

    // The result lands in the owner's gallery.
    let png = b"\x89PNG\r\n\x1a\nnot really".to_vec();
    let (status, body) = send(
        &app,
        "POST",
        &format!("/api/helpers/jobs/{id}/result?name=out_00001_.png&seed=42"),
        &dan,
        "application/octet-stream",
        png.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, _) = call(&app, "POST", &format!("/api/helpers/jobs/{id}/finish"), &dan, Some(json!({"ok": true}))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, view) = call(&app, "GET", &format!("/api/jobs/{id}"), &dan, None).await;
    assert_eq!(view["status"], "done", "{view}");
    assert_eq!(view["seed"], 42);
    assert_eq!(view["media"].as_array().unwrap().len(), 1);
    let (_, gallery) = call(&app, "GET", "/api/gallery", &dan, None).await;
    assert_eq!(gallery[0]["prompt"], "a lighthouse");
    assert_eq!(gallery[0]["bytes"], png.len());

    // Once it's finished, the helper can't report on it any more.
    let (status, _) =
        call(&app, "POST", &format!("/api/helpers/jobs/{id}/progress"), &dan, Some(json!({"phase": "x", "step": 0, "steps": 0}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancelling_a_job_on_a_helper_tells_it_to_stop() {
    let (app, _state) = app().await;
    let dan = owner(&app).await;
    call(&app, "POST", "/api/helpers/poll", &dan, Some(poll(false))).await;
    let (_, made) = call(&app, "POST", "/api/jobs", &dan, Some(json!({"pack": PACK, "inputs": {"prompt": "fog"}}))).await;
    let id = made["id"].as_str().unwrap().to_string();
    let (_, body) = call(&app, "POST", "/api/helpers/poll", &dan, Some(poll(false))).await;
    assert_eq!(body["job"]["job_id"], id.as_str());

    let (status, _) = call(&app, "DELETE", &format!("/api/jobs/{id}"), &dan, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let progress = json!({"phase": "Generating", "step": 1, "steps": 8});
    let (_, reply) = call(&app, "POST", &format!("/api/helpers/jobs/{id}/progress"), &dan, Some(progress)).await;
    assert_eq!(reply["cancel"], true);
    call(&app, "POST", &format!("/api/helpers/jobs/{id}/finish"), &dan, Some(json!({"ok": false, "cancelled": true}))).await;
    let (_, view) = call(&app, "GET", &format!("/api/jobs/{id}"), &dan, None).await;
    assert_eq!(view["status"], "cancelled", "{view}");
}

#[tokio::test]
async fn a_helper_that_fails_says_where() {
    let (app, _state) = app().await;
    let dan = owner(&app).await;
    call(&app, "POST", "/api/helpers/poll", &dan, Some(poll(false))).await;
    let (_, made) = call(&app, "POST", "/api/jobs", &dan, Some(json!({"pack": PACK, "inputs": {"prompt": "x"}}))).await;
    let id = made["id"].as_str().unwrap().to_string();
    call(&app, "POST", "/api/helpers/poll", &dan, Some(poll(false))).await;
    let failed = json!({"ok": false, "error": "ComfyUI ran out of memory."});
    call(&app, "POST", &format!("/api/helpers/jobs/{id}/finish"), &dan, Some(failed)).await;
    let (_, view) = call(&app, "GET", &format!("/api/jobs/{id}"), &dan, None).await;
    assert_eq!(view["status"], "failed");
    let error = view["error"].as_str().unwrap();
    assert!(error.starts_with("On ") && error.contains("ran out of memory"), "{error}");
}

/// The PC side against a real server: it signs in as the owner (skipping a sign-in that
/// isn't allowed), checks in, says what's missing here, and pauses.
#[tokio::test]
async fn this_pc_checks_in_as_a_helper() {
    use outpost_gateway::helper_pc::{HelperPc, Server};

    let (app, state) = app().await;
    let dan = owner(&app).await;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, outpost_gateway::router(state).into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });

    struct To(String, Vec<String>);
    impl Server for To {
        fn url(&self) -> String {
            self.0.clone()
        }
        fn tokens(&self) -> Vec<String> {
            self.1.clone()
        }
    }
    let dir = std::env::temp_dir().join(format!("outpost-helper-pc-{}", std::process::id()));
    let pc = HelperPc::new(&dir, true, false).await.unwrap();
    tokio::spawn(pc.clone().run(To(format!("http://{addr}"), vec!["op_not_a_real_token".into(), dan.clone()])));

    let wait_for = |want: &'static str| {
        let pc = pc.clone();
        async move {
            for _ in 0..100 {
                if pc.status().state == want {
                    return pc.status();
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            panic!("never {want}: {:?}", pc.status());
        }
    };
    // (Whether ComfyUI answers here depends on the machine running the test.)
    let s = wait_for("ready").await;
    if !s.comfyui {
        assert!(s.message.unwrap().contains("ComfyUI"));
    }
    let (_, helpers) = call(&app, "GET", "/api/helpers", &dan, None).await;
    assert_eq!(helpers.as_array().unwrap().len(), 1, "{helpers}");
    assert_eq!(helpers[0]["paused"], false);

    pc.set(true, true);
    wait_for("paused").await;
    let (_, helpers) = call(&app, "GET", "/api/helpers", &dan, None).await;
    assert_eq!(helpers[0]["paused"], true);
    pc.set(false, false);
    wait_for("off").await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// The picture tools take a gallery picture by id; the helper gets that same picture.
#[tokio::test]
async fn a_gallery_picture_goes_to_a_picture_tool() {
    let (app, _state) = app().await;
    let dan = owner(&app).await;
    let png = b"\x89PNG\r\n\x1a\nmy picture".to_vec();
    let (status, imported) =
        send(&app, "POST", "/api/media/import?name=mine.png", &dan, "application/octet-stream", png.clone()).await;
    assert_eq!(status, StatusCode::OK, "{imported}");
    let media_id = imported["media"]["id"].as_str().unwrap().to_string();

    let tool = json!({"vram_mb": 8192, "ram_mb": 16384, "ready_packs": ["remove_background", "upscale_esrgan"]});
    call(&app, "POST", "/api/helpers/poll", &dan, Some(tool.clone())).await;
    // Someone else's (or no such) picture is refused.
    let (status, _) =
        call(&app, "POST", "/api/jobs", &dan, Some(json!({"pack": "remove_background", "inputs": {"image": {"$media": "nope"}}}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, made) =
        call(&app, "POST", "/api/jobs", &dan, Some(json!({"pack": "remove_background", "inputs": {"image": {"$media": media_id}}}))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();

    let (_, body) = call(&app, "POST", "/api/helpers/poll", &dan, Some(tool)).await;
    assert_eq!(body["job"]["job_id"], id.as_str(), "{body}");
    assert_eq!(body["job"]["images"], json!(["image"]));
    let req = Request::builder()
        .uri(format!("/api/helpers/jobs/{id}/input/image"))
        .header(header::AUTHORIZATION, format!("Bearer {dan}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.into_body().collect().await.unwrap().to_bytes().to_vec(), png);
}
