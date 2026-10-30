//! Outpost as an MCP server: apps like Claude Desktop, Hermes or any MCP client can
//! use Outpost's personas, pictures and characters as tools.
//!
//! Transport: MCP "Streamable HTTP" in its simplest form. Each JSON-RPC message is
//! POSTed to `/mcp` with the account's bearer token (an app key), and answered with
//! one JSON body; notifications get `202 Accepted`. Outpost keeps no MCP session, so
//! nothing needs `Mcp-Session-Id`. For apps that only start local programs, the
//! gateway binary bridges stdio to this endpoint: `outpost-gateway --mcp-stdio`.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use outpost_core::jobs::{self, JobStatus};
use outpost_core::{media, memory, personas, training};
use serde_json::{json, Value};

use crate::extract::Auth;
use crate::routes::chat::{plan_pictures, queue_pictures};
use crate::{llm, AppState};

/// Protocol versions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The longest `draw` waits for its pictures before answering with job ids instead.
/// Some apps (Claude Desktop) give up on a tool call after about a minute.
const DRAW_WAIT: Duration = Duration::from_secs(50);
/// The longest `job_status` waits for an unfinished picture.
const STATUS_WAIT: Duration = Duration::from_secs(45);

const INSTRUCTIONS: &str = "Outpost runs AI models on the user's own PC. Use `chat` to ask one of their \
personas, `draw` to make pictures (name one of their characters, from `list_characters`, to draw them), \
and `remember` to save a fact to their memory. Pictures come back as images.";

/// Handle one MCP message.
#[utoipa::path(post, path = "/mcp", tag = "mcp", security(("bearer" = [])),
    request_body(content = Object, description = "A JSON-RPC 2.0 message (MCP Streamable HTTP)"),
    responses(
        (status = 200, description = "The JSON-RPC response", body = Object),
        (status = 202, description = "A notification was received"),
        (status = 401, description = "No or bad app key"),
    ))]
pub async fn handle(State(state): State<AppState>, auth: Auth, Json(body): Json<Value>) -> Response {
    // Batches were dropped from MCP in 2025-06-18; answer each anyway for older clients.
    if let Value::Array(items) = body {
        let mut out = Vec::new();
        for item in items {
            if let Some(r) = respond(&state, &auth, item).await {
                out.push(r);
            }
        }
        return if out.is_empty() { StatusCode::ACCEPTED.into_response() } else { Json(Value::Array(out)).into_response() };
    }
    match respond(&state, &auth, body).await {
        Some(r) => Json(r).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// MCP over HTTP needs no GET stream here.
pub async fn no_stream() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, "Outpost's MCP endpoint takes POST only.").into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The answer to one JSON-RPC message; `None` for notifications.
async fn respond(state: &AppState, auth: &Auth, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    // A notification (no id), or a response to something we never ask: nothing to say.
    let id = id?;
    if method.is_empty() {
        return Some(rpc_error(id, -32600, "Not a JSON-RPC request."));
    }
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method.as_str() {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call_tool(state, auth, &params).await),
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err(rpc_error(id.clone(), -32601, &format!("Outpost doesn't support {method}."))),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => e,
    })
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOL_VERSIONS[1]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "outpost", "title": "Outpost", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

/// The tools, with their input schemas.
pub fn tools() -> Value {
    json!([
        {
            "name": "list_personas",
            "title": "List personas",
            "description": "The user's personas (each with its own model, prompt and memory). Pass a slug to `chat`.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "chat",
            "title": "Ask a persona",
            "description": "Ask one of the user's personas something and get its reply, from a model running on their PC. One-off: nothing is saved to their chats.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "What to ask." },
                    "persona": { "type": "string", "description": "A persona slug from list_personas. Default: assistant." }
                },
                "required": ["message"]
            }
        },
        {
            "name": "list_characters",
            "title": "List characters",
            "description": "The user's trained characters. Name one in `draw` (\"Kat at the beach\") to draw them.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "draw",
            "title": "Draw pictures",
            "description": "Make pictures on the user's PC and return them. Name a character to draw them. Ask for several with \"4 pictures of ...\", or separate scenes with ';'. Up to 8. Waits up to 50 seconds; pictures still drawing come back as job ids: call job_status for each (it waits too).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "What to draw, e.g. \"Kat at the beach at sunset\"." },
                    "wait": { "type": "boolean", "description": "Wait for the pictures (default true). False returns job ids at once." }
                },
                "required": ["prompt"]
            }
        },
        {
            "name": "job_status",
            "title": "Picture status",
            "description": "How a picture from `draw` is getting on, and the picture once it's done. Waits up to 45 seconds for it to finish.",
            "inputSchema": {
                "type": "object",
                "properties": { "job_id": { "type": "string" } },
                "required": ["job_id"]
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "recent_pictures",
            "title": "Recent pictures",
            "description": "The newest pictures in the user's gallery, with their prompts and ids.",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 20, "description": "How many (default 5)." } }
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "get_picture",
            "title": "Get a picture",
            "description": "One picture from the user's gallery, by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "media_id": { "type": "string" } },
                "required": ["media_id"]
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "remember",
            "title": "Remember a fact",
            "description": "Save a fact about the user to Outpost's memory; their personas with memory on will know it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The fact, e.g. \"Dan keeps bees\"." },
                    "persona": { "type": "string", "description": "Only this persona (slug); leave out for all." }
                },
                "required": ["text"]
            }
        }
    ])
}

/// A tool result: content blocks, and whether it went wrong.
fn text(t: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }], "isError": false })
}

fn failed(t: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }], "isError": true })
}

fn arg<'a>(params: &'a Value, key: &str) -> Option<&'a Value> {
    params.get("arguments").and_then(|a| a.get(key))
}

fn arg_str(params: &Value, key: &str) -> Option<String> {
    arg(params, key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

async fn call_tool(state: &AppState, auth: &Auth, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let account = auth.account.id.as_str();
    match name {
        "list_personas" => match personas::list(&state.pool, account).await {
            Ok(list) => text(
                list.iter()
                    .map(|p| format!("{} (slug: {}){}", p.name, p.slug, p.model.as_ref().map(|m| format!(", model {m}")).unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(e) => failed(e.to_string()),
        },
        "chat" => {
            let Some(message) = arg_str(params, "message") else { return failed("Give a message.") };
            let slug = arg_str(params, "persona").unwrap_or_else(|| "assistant".into());
            match ask(state, account, &slug, &message).await {
                Ok(reply) => text(reply),
                Err(e) => failed(e),
            }
        }
        "list_characters" => match training::characters(&state.pool, Some(account)).await {
            Ok(list) if list.is_empty() => text("No characters yet. Train one in Outpost's Train screen."),
            Ok(mut list) => {
                for c in &mut list {
                    let _ = training::fill_subject(&state.pool, c).await;
                }
                text(
                    list.iter()
                        .map(|c| {
                            format!(
                                "{} (trigger word {}{})",
                                c.name,
                                c.trigger_word,
                                c.subject.as_ref().map(|s| format!(", a {s}")).unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
            Err(e) => failed(e.to_string()),
        },
        "draw" => {
            let Some(prompt) = arg_str(params, "prompt") else { return failed("Say what to draw.") };
            let wait = arg(params, "wait").and_then(Value::as_bool).unwrap_or(true);
            draw(state, auth, &prompt, wait).await
        }
        "job_status" => {
            let Some(job_id) = arg_str(params, "job_id") else { return failed("Give a job_id.") };
            // Wait a while for it to finish, so the caller needn't ask over and over.
            let started = std::time::Instant::now();
            loop {
                match jobs::get(&state.pool, account, &job_id).await {
                    Ok(job) if job.status.finished() || started.elapsed() > STATUS_WAIT => {
                        break job_result(state, account, &job).await
                    }
                    Ok(_) => tokio::time::sleep(Duration::from_secs(1)).await,
                    Err(_) => break failed("No such picture job on this account."),
                }
            }
        }
        "recent_pictures" => {
            let limit = arg(params, "limit").and_then(Value::as_i64).unwrap_or(5).clamp(1, 20);
            match media::list(&state.pool, account, limit, None).await {
                Ok(list) if list.is_empty() => text("The gallery is empty."),
                Ok(list) => text(
                    list.iter()
                        .map(|m| format!("{} · {} · {}", m.id, m.kind, m.prompt.as_deref().unwrap_or("(no prompt)")))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                Err(e) => failed(e.to_string()),
            }
        }
        "get_picture" => {
            let Some(id) = arg_str(params, "media_id") else { return failed("Give a media_id.") };
            match picture_block(state, account, &id).await {
                Some(block) => json!({ "content": [block], "isError": false }),
                None => failed("No such picture in this account's gallery."),
            }
        }
        "remember" => {
            let Some(t) = arg_str(params, "text") else { return failed("Give the fact to remember.") };
            let persona_id = match arg_str(params, "persona") {
                Some(slug) => match personas::get_by_slug(&state.pool, account, &slug).await {
                    Ok(p) => Some(p.id),
                    Err(_) => return failed(format!("No persona \"{slug}\".")),
                },
                None => None,
            };
            match memory::add(&state.pool, account, persona_id.as_deref(), &t).await {
                Ok(_) => text(format!("Remembered: {t}")),
                Err(e) => failed(e.to_string()),
            }
        }
        _ => failed(format!("Outpost has no tool called {name}.")),
    }
}

/// Ask a persona once, without saving a chat.
async fn ask(state: &AppState, account: &str, slug: &str, message: &str) -> Result<String, String> {
    let persona = personas::get_by_slug(&state.pool, account, slug)
        .await
        .map_err(|_| format!("No persona \"{slug}\". Use list_personas."))?;
    let resolved = llm::resolve_model(state, None, Some(&persona)).await.map_err(|e| e.message)?;
    let system = llm::system_prompt(state, account, Some(&persona), Some(message))
        .await
        .map_err(|e| e.message)?;
    let mut messages = Vec::new();
    if !system.is_empty() {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.push(json!({ "role": "user", "content": message }));
    let mut body = json!({ "model": resolved.model, "messages": messages, "stream": false });
    if resolved.can_think {
        body["think"] = json!(false);
    }
    if let Some(t) = persona.temperature {
        body["options"] = json!({ "temperature": t });
    }
    let url = format!("{}/api/chat", state.config.ollama_url.trim_end_matches('/'));
    let resp = tokio::time::timeout(Duration::from_secs(600), state.llm.post(&url).json(&body).send())
        .await
        .map_err(|_| "The model took too long to answer.".to_string())?
        .map_err(|_| "Ollama isn't running on the Outpost PC.".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Ollama refused: {}", resp.text().await.unwrap_or_default()));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    let reply = v["message"]["content"].as_str().unwrap_or_default();
    Ok(crate::routes::training::strip_thinking(reply).trim().to_string())
}

async fn draw(state: &AppState, auth: &Auth, prompt: &str, wait: bool) -> Value {
    let account = auth.account.id.as_str();
    let planned = match plan_pictures(state, account, prompt).await {
        Ok(p) => p,
        Err(e) => return failed(e.message),
    };
    let ids = match queue_pictures(state, account, jobs::priority_for(auth.session.kind), &planned).await {
        Ok(ids) => ids,
        Err(e) => return failed(e.message),
    };
    let summary = planned
        .iter()
        .zip(&ids)
        .map(|(p, id)| format!("job {id}: {} with {}, prompt \"{}\"", p.what, p.pack.name, p.prompt()))
        .collect::<Vec<_>>()
        .join("\n");
    if !wait {
        return text(format!("Queued:\n{summary}\nUse job_status to fetch each picture."));
    }
    let started = std::time::Instant::now();
    let mut content = vec![json!({ "type": "text", "text": format!("Drew:\n{summary}") })];
    let mut errors = 0;
    for id in &ids {
        loop {
            let job = match jobs::get(&state.pool, account, id).await {
                Ok(j) => j,
                Err(_) => break,
            };
            if job.status.finished() {
                if job.status == JobStatus::Done {
                    for m in media::for_job(&state.pool, account, id).await.unwrap_or_default() {
                        if let Some(block) = picture_block(state, account, &m.id).await {
                            content.push(block);
                        }
                    }
                } else {
                    errors += 1;
                    content.push(json!({ "type": "text", "text": format!("job {id} {}: {}", status_word(job.status), job.error.unwrap_or_default()) }));
                }
                break;
            }
            if started.elapsed() > DRAW_WAIT {
                content.push(json!({ "type": "text", "text": format!("job {id} is still {}; call job_status with this id to get it.", status_word(job.status)) }));
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    json!({ "content": content, "isError": errors == ids.len() })
}

fn status_word(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Queued => "waiting",
        JobStatus::Running => "drawing",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
    }
}

async fn job_result(state: &AppState, account: &str, job: &jobs::Job) -> Value {
    if job.status != JobStatus::Done {
        let mut line = format!("job {}: {}", job.id, status_word(job.status));
        if job.status == JobStatus::Queued {
            if let Ok(n) = jobs::position(&state.pool, job).await {
                line.push_str(&format!(", {n} ahead"));
            }
        }
        if let Some(r) = state.queue.progress_of(&job.id) {
            if r.progress.steps > 0 {
                line.push_str(&format!(", step {} of {}", r.progress.step, r.progress.steps));
            }
        }
        if let Some(e) = &job.error {
            line.push_str(&format!(": {e}"));
        }
        return if job.status == JobStatus::Failed { failed(line) } else { text(line) };
    }
    let mut content = vec![json!({ "type": "text", "text": format!("job {}: done", job.id) })];
    for m in media::for_job(&state.pool, account, &job.id).await.unwrap_or_default() {
        if let Some(block) = picture_block(state, account, &m.id).await {
            content.push(block);
        }
    }
    json!({ "content": content, "isError": false })
}

/// A gallery picture as an MCP image block (videos as a link to the API instead).
async fn picture_block(state: &AppState, account: &str, media_id: &str) -> Option<Value> {
    let (m, path) = media::get(&state.pool, &state.config.files_dir(), account, media_id).await.ok()?;
    if m.kind != "image" {
        return Some(json!({ "type": "text", "text": format!("Video {} (fetch it from GET /api/media/{}).", m.id, m.id) }));
    }
    let data = tokio::fs::read(&path).await.ok()?;
    Some(json!({
        "type": "image",
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": m.mime,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_have_names_and_object_schemas() {
        let list = tools();
        let names: Vec<&str> = list.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"draw") && names.contains(&"chat") && names.contains(&"remember"));
        for t in list.as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
            assert!(t["description"].as_str().unwrap().len() > 20);
        }
    }

    #[test]
    fn version_is_the_clients_when_known() {
        assert_eq!(initialize(&json!({"protocolVersion": "2025-03-26"}))["protocolVersion"], "2025-03-26");
        assert_eq!(initialize(&json!({"protocolVersion": "1999-01-01"}))["protocolVersion"], "2025-06-18");
    }
}

/// Bridge an MCP client that talks over stdin/stdout (one JSON message per line, as
/// Claude Desktop does with local servers) to a running Outpost's `/mcp`. Nothing but
/// MCP messages may go to stdout, so this logs nothing.
pub async fn stdio_bridge(url: String, key: String) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .build()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let id = serde_json::from_str::<Value>(line).ok().and_then(|v| v.get("id").cloned());
        let reply = match client
            .post(&url)
            .bearer_auth(&key)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(line.to_string())
            .send()
            .await
        {
            Ok(r) if r.status() == reqwest::StatusCode::ACCEPTED => None,
            Ok(r) if r.status().is_success() => Some(r.text().await.unwrap_or_default()),
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                let why = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                    .unwrap_or(body);
                id.clone().map(|id| rpc_error(id, -32000, &format!("Outpost answered {status}: {why}")).to_string())
            }
            Err(_) => id
                .clone()
                .map(|id| rpc_error(id, -32000, &format!("Outpost isn't answering at {url}. Is it running?")).to_string()),
        };
        if let Some(r) = reply {
            // One message per line: compact any pretty JSON.
            let compact = serde_json::from_str::<Value>(&r).map(|v| v.to_string()).unwrap_or(r);
            out.write_all(compact.as_bytes()).await?;
            out.write_all(b"\n").await?;
            out.flush().await?;
        }
    }
    Ok(())
}

/// Where things are, for the API page's setup snippets.
#[derive(Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Integrations {
    /// The API's port on this PC.
    pub port: u16,
    /// The program that bridges stdio MCP clients (Claude Desktop) to Outpost, when
    /// it can be found: `<path> --mcp-stdio` with `OUTPOST_KEY` set.
    pub bridge_path: Option<String>,
}

/// Find `outpost-gateway(.exe)`: this program, or one beside it (a release build
/// beside a debug one counts).
pub fn bridge_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "outpost-gateway.exe" } else { "outpost-gateway" };
    if exe.file_name().is_some_and(|n| n == name) {
        return Some(exe);
    }
    let dir = exe.parent()?;
    let mut candidates = vec![dir.join(name)];
    if let Some(target) = dir.parent() {
        candidates.push(target.join("release").join(name));
        candidates.push(target.join("debug").join(name));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Ports and paths for connecting apps.
#[utoipa::path(get, path = "/api/integrations", tag = "mcp", security(("bearer" = [])),
    responses((status = 200, description = "Where things are", body = Integrations)))]
pub async fn integrations(State(state): State<AppState>, _auth: Auth) -> Json<Integrations> {
    Json(Integrations {
        port: state.config.bind.port(),
        bridge_path: bridge_path().map(|p| p.to_string_lossy().to_string()),
    })
}
