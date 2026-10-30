//! Chatting with documents: adding them, reading them in the background, attaching them to
//! chats and personas, and the passages a message gets.

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
        "outpost-docs-test-{}-{}",
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

async fn add(app: &Router, token: &str, name: &str, text: &str) -> String {
    let (status, body) = send(app, "POST", &format!("/api/documents?name={name}"), Some(token), "application/octet-stream", text.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["document"]["id"].as_str().unwrap().to_string();
    // Read in the background: wait until it's done.
    for _ in 0..100 {
        let (_, list) = call(app, "GET", "/api/documents", token, None).await;
        let d = list.as_array().unwrap().iter().find(|d| d["id"] == id.as_str()).cloned().unwrap();
        if d["status"] != "reading" {
            assert_eq!(d["status"], "ready", "{d}");
            return id;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("never read");
}

#[tokio::test]
async fn documents_are_read_attached_and_searched() {
    let (app, state) = app().await;
    let dan = owner(&app).await;

    // Not a kind Outpost reads.
    let (status, _) = send(&app, "POST", "/api/documents?name=x.exe", Some(&dan), "application/octet-stream", b"MZ".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let manual = add(&app, &dan, "manual.md", "# Kettle\n\nThe warranty covers parts for two years.\n\nDescale it monthly.").await;
    // The same file again is the same document.
    let (status, again) =
        send(&app, "POST", "/api/documents?name=copy.md", Some(&dan), "application/octet-stream", b"# Kettle\n\nThe warranty covers parts for two years.\n\nDescale it monthly.".to_vec()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["added"], false);
    assert_eq!(again["document"]["id"], manual.as_str());

    // A big one: many pages of filler, with the answer in one place.
    let mut big = String::new();
    for i in 0..400 {
        big.push_str(&format!("Section {i}. The committee reviewed the minutes and agreed to continue as before.\n\n"));
        if i == 271 {
            big.push_str("The pool gate code is 4711 and it changes every spring.\n\n");
        }
    }
    let handbook = add(&app, &dan, "handbook.txt", &big).await;

    let (_, chat) = call(&app, "POST", "/api/chats", &dan, Some(json!({}))).await;
    let chat_id = chat["id"].as_str().unwrap().to_string();
    // Nothing attached: nothing added.
    assert!(outpost_gateway::documents::context(&state, &chat_id, None, "what is the gate code", None).await.is_none());

    let (status, attached) =
        call(&app, "PUT", &format!("/api/chats/{chat_id}/documents"), &dan, Some(json!({"document_ids": [handbook]}))).await;
    assert_eq!(status, StatusCode::OK, "{attached}");
    assert_eq!(attached["documents"][0]["name"], "handbook.txt");

    // The big one is searched: the passage with the answer is among those given.
    let ctx = outpost_gateway::documents::context(&state, &chat_id, None, "What's the pool gate code?", None).await.unwrap();
    assert!(ctx.section.contains("4711"), "{}", ctx.section);
    assert!(ctx.sources.len() <= 6 && !ctx.sources.is_empty());
    assert!(ctx.section.contains("[1] handbook.txt"));
    // A short follow-up is searched with the message before it.
    let ctx = outpost_gateway::documents::context(&state, &chat_id, None, "when does it change?", Some("the pool gate code")).await.unwrap();
    assert!(ctx.section.contains("4711"));

    // A small one goes in whole.
    call(&app, "PUT", &format!("/api/chats/{chat_id}/documents"), &dan, Some(json!({"document_ids": [manual]}))).await;
    let ctx = outpost_gateway::documents::context(&state, &chat_id, None, "anything", None).await.unwrap();
    assert!(ctx.section.contains("two years") && ctx.section.contains("monthly"));

    // A persona's documents go with it.
    let (_, persona) = call(&app, "POST", "/api/personas", &dan, Some(json!({"name": "Librarian"}))).await;
    let pid = persona["id"].as_str().unwrap().to_string();
    let (status, pd) =
        call(&app, "PUT", &format!("/api/personas/{pid}/documents"), &dan, Some(json!({"document_ids": [handbook]}))).await;
    assert_eq!(status, StatusCode::OK, "{pd}");
    let p = outpost_core::personas::get(&state.pool, &chat["account_id"].as_str().unwrap(), &pid).await.unwrap();
    let ctx = outpost_gateway::documents::context(&state, &chat_id, Some(&p), "gate code", None).await.unwrap();
    assert!(ctx.section.contains("4711"), "persona's and chat's together");

    // Someone else's document can't be attached; deleting one detaches it everywhere.
    let (status, _) =
        call(&app, "PUT", &format!("/api/chats/{chat_id}/documents"), &dan, Some(json!({"document_ids": ["nope"]}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "DELETE", &format!("/api/documents/{manual}"), &dan, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, attached) = call(&app, "GET", &format!("/api/chats/{chat_id}/documents"), &dan, None).await;
    assert!(attached["documents"].as_array().unwrap().is_empty(), "{attached}");

    // The file can be downloaded again.
    let req = Request::builder()
        .uri(format!("/api/documents/{handbook}/file"))
        .header(header::AUTHORIZATION, format!("Bearer {dan}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers()[header::CONTENT_DISPOSITION].to_str().unwrap().contains("handbook.txt"));
    let _ = std::fs::remove_dir_all(&state.config.data_dir);
}

#[tokio::test]
async fn unreadable_documents_say_why() {
    let (app, _state) = app().await;
    let dan = owner(&app).await;
    let (status, body) = send(&app, "POST", "/api/documents?name=broken.pdf", Some(&dan), "application/octet-stream", b"not a pdf at all".to_vec()).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["document"]["id"].as_str().unwrap().to_string();
    for _ in 0..100 {
        let (_, list) = call(&app, "GET", "/api/documents", &dan, None).await;
        let d = &list[0];
        if d["status"] == "failed" {
            assert!(d["error"].as_str().unwrap().contains("PDF"), "{d}");
            return;
        }
        assert_eq!(d["id"], id.as_str());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("never failed");
}

/// Real files: a two-page PDF (page numbers kept) and a Word file.
#[test]
fn real_pdfs_and_word_files_are_read() {
    let pdf = include_bytes!("fixtures/two-pages.pdf");
    let doc = outpost_gateway::documents::extract("application/pdf", pdf).unwrap();
    assert_eq!(doc.pages, Some(2));
    let chunks = outpost_gateway::documents::chunk(&doc);
    let warranty = chunks.iter().find(|c| c.text.contains("two years")).expect("page 2's text");
    assert_eq!(warranty.page, Some(2));
    assert!(chunks.iter().any(|c| c.page == Some(1) && c.text.contains("fresh water")));

    let docx = include_bytes!("fixtures/rules.docx");
    let doc = outpost_gateway::documents::extract(
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        docx,
    )
    .unwrap();
    let text = &doc.parts[0].1;
    assert!(text.contains("The gate code is 4711."), "{text}");
}
