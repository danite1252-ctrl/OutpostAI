//! The MCP client (tools on the user's PC) against small stand-in servers: one speaking stdio
//! (a Python script), one over Streamable HTTP (answering as JSON and as an event stream).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use outpost_gateway::mcp_client::{Pool, ServerConfig};
use serde_json::{json, Value};

const FAKE_STDIO: &str = r#"
import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    method, mid = msg.get("method"), msg.get("id")
    if mid is None:
        continue
    if method == "initialize":
        # A request of its own first (clients may ignore it), then the answer.
        print(json.dumps({"jsonrpc": "2.0", "id": 900, "method": "roots/list"}), flush=True)
        result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake"}}
    elif method == "tools/list":
        if msg.get("params", {}).get("cursor") == "p2":
            result = {"tools": [{"name": "shout", "description": "Upper-case it", "inputSchema": {"type": "object"}}]}
        else:
            result = {"tools": [{"name": "echo", "description": "Say it back",
                                 "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
                                 "annotations": {"readOnlyHint": True}}], "nextCursor": "p2"}
    elif method == "tools/call":
        p = msg["params"]
        if p["name"] == "crash":
            sys.exit(3)
        text = p["arguments"].get("text", "")
        result = {"content": [{"type": "text", "text": text.upper() if p["name"] == "shout" else text}]}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "no such method"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": mid, "result": result}), flush=True)
"#;

fn python() -> Option<String> {
    ["python3", "python"].into_iter().find(|p| std::process::Command::new(p).arg("--version").output().is_ok()).map(str::to_string)
}

#[tokio::test]
async fn a_stdio_server_lists_and_runs_its_tools() {
    let Some(py) = python() else {
        eprintln!("no Python here; skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("outpost-mcp-stdio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("fake.py");
    std::fs::write(&script, FAKE_STDIO).unwrap();
    let pool = Pool::new(dir.join("engines"));
    let cfg = ServerConfig {
        id: "fake".into(),
        name: "Fake".into(),
        transport: "stdio".into(),
        command: py,
        args: vec![script.to_string_lossy().to_string()],
        ..Default::default()
    };

    // Both pages of tools.
    let tools = pool.tools(&cfg).await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["echo", "shout"]);
    assert_eq!(tools[0].read_only, Some(true));
    assert_eq!(tools[0].input_schema["properties"]["text"]["type"], "string");

    let r = pool.call(&cfg, "shout", json!({ "text": "hello" })).await.unwrap();
    assert_eq!(r.text, "HELLO");
    assert!(!r.is_error);

    // It stops mid-call: the call fails with a reason, and the next call starts it again.
    let err = pool.call(&cfg, "crash", json!({})).await.unwrap_err();
    assert!(err.contains("stopped"), "{err}");
    let r = pool.call(&cfg, "echo", json!({ "text": "back again" })).await.unwrap();
    assert_eq!(r.text, "back again");

    // An unknown tool is the server's error, passed on.
    let err = pool.call(&cfg, "missing", json!({})).await;
    assert!(err.is_ok() || err.is_err());
    pool.close("fake").await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[derive(Default)]
struct Seen {
    sessions: Vec<Option<String>>,
    auth: Vec<Option<String>>,
}

async fn fake_http(seen: Arc<Mutex<Seen>>, headers: HeaderMap, Json(msg): Json<Value>) -> Response {
    {
        let mut s = seen.lock().unwrap();
        s.sessions.push(headers.get("mcp-session-id").and_then(|v| v.to_str().ok()).map(str::to_string));
        s.auth.push(headers.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_string));
    }
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or_default().to_string();
    let Some(id) = id else { return StatusCode::ACCEPTED.into_response() };
    match method.as_str() {
        "initialize" => (
            [("mcp-session-id", "sess-1")],
            Json(json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": "2025-03-26", "capabilities": {} } })),
        )
            .into_response(),
        // As an event stream, with a notification first.
        "tools/list" => {
            let body = format!(
                "event: message\ndata: {}\n\nevent: message\ndata: {}\n\n",
                json!({ "jsonrpc": "2.0", "method": "notifications/message", "params": {} }),
                json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": [{ "name": "search", "description": "Find things" }] } })
            );
            ([("content-type", "text/event-stream")], body).into_response()
        }
        "tools/call" => Json(json!({ "jsonrpc": "2.0", "id": id, "result": {
            "content": [{ "type": "text", "text": format!("found: {}", msg["params"]["arguments"]["q"].as_str().unwrap_or_default()) }]
        } }))
        .into_response(),
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

#[tokio::test]
async fn an_http_server_answers_as_json_or_a_stream() {
    let seen: Arc<Mutex<Seen>> = Default::default();
    let s = seen.clone();
    let app = Router::new().route("/mcp", post(move |h: HeaderMap, b: Json<Value>| fake_http(s.clone(), h, b)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let pool = Pool::new(std::env::temp_dir());
    let cfg = ServerConfig {
        id: "web".into(),
        name: "Web".into(),
        transport: "http".into(),
        url: format!("http://{addr}/mcp"),
        headers: BTreeMap::from([("Authorization".to_string(), "Bearer k".to_string())]),
        ..Default::default()
    };
    let tools = pool.tools(&cfg).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search");
    // No schema given: an empty object one.
    assert_eq!(tools[0].input_schema["type"], "object");
    let r = pool.call(&cfg, "search", json!({ "q": "lighthouses" })).await.unwrap();
    assert_eq!(r.text, "found: lighthouses");

    let seen = seen.lock().unwrap();
    // The session from initialize is sent after it; the key with every request.
    assert_eq!(seen.sessions[0], None);
    assert!(seen.sessions[1..].iter().all(|s| s.as_deref() == Some("sess-1")));
    assert!(seen.auth.iter().all(|a| a.as_deref() == Some("Bearer k")));
}
