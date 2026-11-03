//! Chat on a lent PC: a helper PC (see [`crate::helpers`]) lending its card for chat runs
//! Ollama there, and the server sends it the chat requests for the models it has.
//!
//! The helper reaches the server; the server never reaches the helper (it may be behind a
//! firewall). So the helper asks for work (`POST /api/helpers/chat/next`, waiting up to
//! [`WAIT`]), runs each request against its own Ollama, and sends Ollama's answer back as it
//! streams (`POST /api/helpers/chat/{id}/reply`, the body being Ollama's own reply, its status
//! in `x-ollama-status`). Whoever asked reads it like Ollama's reply here: [`post`] hides which
//! one answered. Stopping a reply here (the person pressed Stop) closes the helper's upload,
//! and the helper stops its Ollama.
//!
//! Which models run there: those the lent PC has and the server doesn't, and, when its card is
//! bigger than the server's, those both have. Embeddings always stay on the server.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use outpost_engines::models::InstalledModel;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Notify};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::helpers::Helper;
use crate::AppState;

/// Longest a helper's ask for work waits.
const WAIT: Duration = Duration::from_secs(25);
/// Longest a request waits for the helper to start answering (it loads the model first).
const ANSWER_WITHIN: Duration = Duration::from_secs(180);
/// Longest a request waits for the helper to pick it up at all.
const PICKED_WITHIN: Duration = Duration::from_secs(15);

/// A request for a helper's Ollama.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LentRequest {
    pub id: String,
    /// Ollama's path, e.g. `/api/chat`.
    pub path: String,
    #[schema(value_type = Object)]
    pub body: Value,
}

struct Waiter {
    session_id: String,
    picked: Option<oneshot::Sender<()>>,
    status: Option<oneshot::Sender<u16>>,
    chunks: mpsc::Sender<Result<Bytes, String>>,
}

/// Requests waiting for helpers, and the answers on their way back.
#[derive(Clone, Default)]
pub struct LentChat {
    queue: Arc<Mutex<HashMap<String, VecDeque<LentRequest>>>>,
    waiting: Arc<Mutex<HashMap<String, Waiter>>>,
    notify: Arc<Notify>,
}

impl LentChat {
    fn push(&self, session_id: &str, req: LentRequest) {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(session_id.to_string())
            .or_default()
            .push_back(req);
        self.notify.notify_waiters();
    }

    fn pop(&self, session_id: &str) -> Option<LentRequest> {
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).get_mut(session_id)?.pop_front()
    }

    fn forget(&self, id: &str) {
        for q in self.queue.lock().unwrap_or_else(|p| p.into_inner()).values_mut() {
            q.retain(|r| r.id != id);
        }
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(id);
    }

    /// A helper stopped lending: what it was asked fails straight away.
    pub fn drop_helper(&self, session_id: &str) {
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).remove(session_id);
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).retain(|_, w| w.session_id != session_id);
    }
}

/// Ollama's answer, from here or a lent PC.
pub struct Reply {
    pub status: u16,
    stream: BoxStream<'static, Result<Bytes, String>>,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn bytes_stream(self) -> BoxStream<'static, Result<Bytes, String>> {
        self.stream
    }

    pub async fn bytes(mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        while let Some(chunk) = self.stream.next().await {
            out.extend_from_slice(&chunk?);
        }
        Ok(out)
    }

    pub async fn text(self) -> String {
        self.bytes().await.map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default()
    }

    pub async fn json<T: DeserializeOwned>(self) -> Result<T, String> {
        let bytes = self.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    }
}

/// The lent PC a model would run on now, if any (see the module's notes).
pub async fn runs_on(state: &AppState, model: &str) -> Option<Helper> {
    let helper = state.helpers.for_model(model)?;
    let server = crate::routes::models::measure_cached(state).await;
    if helper.vram_mb > server.vram_mb {
        return Some(helper);
    }
    let here = state.models.installed().await.unwrap_or_default();
    (!here.iter().any(|m| m.model == model)).then_some(helper)
}

/// The chat models the server can use: its own, plus the lent PCs' (marked with where they
/// run). An error only when Ollama is off here and no lent PC has any.
pub async fn installed(state: &AppState) -> Result<Vec<InstalledModel>, outpost_engines::models::ModelError> {
    let here = state.models.installed().await;
    let helpers: Vec<Helper> = state.helpers.all().into_iter().filter(Helper::lends_chat).collect();
    if helpers.is_empty() {
        return here;
    }
    let server_vram = crate::routes::models::measure_cached(state).await.vram_mb;
    let out = match here {
        Ok(list) => list,
        Err(e) if helpers.iter().all(|h| h.chat_models.is_empty()) => return Err(e),
        Err(_) => Vec::new(),
    };
    Ok(merge(out, helpers, server_vram))
}

/// The server's models plus the lent PCs': a model only there runs there; one in both runs
/// on the lent PC when its card is bigger. Embedding models stay the server's.
pub fn merge(mut out: Vec<InstalledModel>, mut helpers: Vec<Helper>, server_vram_mb: u64) -> Vec<InstalledModel> {
    // Bigger cards first: a model on two lent PCs runs on the bigger.
    helpers.sort_by_key(|h| std::cmp::Reverse(h.vram_mb));
    for h in &helpers {
        for m in &h.chat_models {
            if m.badges.contains(&outpost_engines::catalog::Badge::Embedding) {
                continue;
            }
            match out.iter_mut().find(|x| x.model == m.model) {
                Some(x) if x.lent.is_none() && x.here && h.vram_mb > server_vram_mb => x.lent = Some(h.name.clone()),
                Some(_) => {}
                None => {
                    let mut m = m.clone();
                    m.here = false;
                    m.lent = Some(h.name.clone());
                    out.push(m);
                }
            }
        }
    }
    out.sort_by(|a, b| a.model.cmp(&b.model));
    out
}

/// Models whose chat template refused Outpost's extra system notes, and how far they need
/// them folded: 1 = one system message, at the start; 2 = none (into the first user message).
static STRICT_ROLES: Mutex<Option<HashMap<String, u8>>> = Mutex::new(None);

fn strictness(model: &str) -> u8 {
    STRICT_ROLES.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|m| m.get(model).copied()).unwrap_or(0)
}

fn remember_strictness(model: &str, level: u8) {
    STRICT_ROLES.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(HashMap::new).insert(model.to_string(), level);
}

/// Ollama's error when a model's template won't take a system message where it was (Mistral's
/// and Devstral's take one, first; some take none): "Only user, assistant and tool roles are
/// supported, got system."
fn refuses_system(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("system") && (e.contains("role") || e.contains("raise_exception") || e.contains("jinja"))
}

/// The body with its system messages folded, for a strict template. Level 1: all of them in
/// one at the start, except notes placed later in the chat (the clock), which go in front of
/// the user message they preceded. Level 2: no system message at all; the lot goes in front
/// of the first user message.
pub fn fold_system(body: &Value, level: u8) -> Value {
    let Some(messages) = body["messages"].as_array() else { return body.clone() };
    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    let mut leading: Vec<String> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for m in messages {
        let content = m["content"].as_str().unwrap_or_default().to_string();
        if m["role"] == "system" {
            if out.is_empty() {
                leading.push(content);
            } else {
                pending.push(content);
            }
            continue;
        }
        let mut m = m.clone();
        if m["role"] == "user" {
            let mut notes = std::mem::take(&mut pending);
            if level >= 2 && !leading.is_empty() && !out.iter().any(|x: &Value| x["role"] == "user") {
                notes.splice(0..0, std::mem::take(&mut leading));
            }
            if !notes.is_empty() {
                let text = m["content"].as_str().unwrap_or_default();
                m["content"] = Value::String(format!("{}\n\n{text}", notes.join("\n\n")));
            }
        }
        out.push(m);
    }
    // Notes after the last user message, or a system prompt with nothing to go in front of.
    leading.extend(pending);
    if !leading.is_empty() {
        if level >= 2 {
            out.insert(0, serde_json::json!({ "role": "user", "content": leading.join("\n\n") }));
        } else {
            out.insert(0, serde_json::json!({ "role": "system", "content": leading.join("\n\n") }));
        }
    }
    let mut body = body.clone();
    body["messages"] = Value::Array(out);
    body
}

/// Any system message: a strict template might refuse it.
fn has_system(body: &Value) -> bool {
    body["messages"].as_array().is_some_and(|m| m.iter().any(|x| x["role"] == "system"))
}

/// POST to Ollama (`path` like `/api/chat`), on the lent PC that runs the body's model if
/// there is one, else here. If the lent PC doesn't pick it up, it's tried here (when the
/// model is here too). A model whose template refuses Outpost's system notes is asked again
/// with them folded in (and from then on, straight away).
pub async fn post(state: &AppState, path: &str, body: &Value, timeout: Option<Duration>) -> Result<Reply, String> {
    let model = body["model"].as_str().unwrap_or_default().to_string();
    let mut level = strictness(&model);
    loop {
        let sent = if level > 0 { fold_system(body, level) } else { body.clone() };
        let reply = post_once(state, path, &sent, timeout).await?;
        if reply.ok() || level >= 2 || !has_system(&sent) {
            return Ok(reply);
        }
        let status = reply.status;
        let bytes = reply.bytes().await.unwrap_or_default();
        if !refuses_system(&String::from_utf8_lossy(&bytes)) {
            return Ok(Reply { status, stream: futures_util::stream::once(async move { Ok(Bytes::from(bytes)) }).boxed() });
        }
        level += 1;
        tracing::info!(model = %model, level, "the model's template refused a system message; folding them in");
        remember_strictness(&model, level);
    }
}

async fn post_once(state: &AppState, path: &str, body: &Value, timeout: Option<Duration>) -> Result<Reply, String> {
    let model = body["model"].as_str().unwrap_or_default().to_string();
    if let Some(helper) = runs_on(state, &model).await {
        match through(state, &helper, path, body).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                let here = state.models.installed().await.unwrap_or_default();
                if !here.iter().any(|m| m.model == model) {
                    return Err(e);
                }
                tracing::warn!(helper = %helper.name, error = %e, "the lent PC didn't answer; using the server's Ollama");
            }
        }
    }
    let url = format!("{}{}", state.config.ollama_url.trim_end_matches('/'), path);
    let mut req = state.llm.post(&url).json(body);
    if let Some(t) = timeout {
        req = req.timeout(t);
    }
    let resp = req.send().await.map_err(|_| "Ollama isn't running. Start it in Settings → Engines.".to_string())?;
    let status = resp.status().as_u16();
    Ok(Reply { status, stream: resp.bytes_stream().map(|c| c.map_err(|e| e.to_string())).boxed() })
}

async fn through(state: &AppState, helper: &Helper, path: &str, body: &Value) -> Result<Reply, String> {
    let lent = &state.lent_chat;
    let id = uuid::Uuid::new_v4().to_string();
    let (picked_tx, picked_rx) = oneshot::channel();
    let (status_tx, status_rx) = oneshot::channel();
    let (tx, rx) = mpsc::channel(256);
    lent.waiting.lock().unwrap_or_else(|p| p.into_inner()).insert(
        id.clone(),
        Waiter { session_id: helper.session_id.clone(), picked: Some(picked_tx), status: Some(status_tx), chunks: tx },
    );
    // The window this card has room for, even when that PC's Ollama wasn't started by Outpost.
    let mut body = body.clone();
    if path == "/api/chat" && body["options"]["num_ctx"].is_null() {
        let window = outpost_engines::versions::context_length_for(helper.vram_mb);
        match body.get_mut("options").and_then(Value::as_object_mut) {
            Some(o) => {
                o.insert("num_ctx".into(), window.into());
            }
            None => body["options"] = serde_json::json!({ "num_ctx": window }),
        }
    }
    lent.push(&helper.session_id, LentRequest { id: id.clone(), path: path.to_string(), body });
    if !matches!(tokio::time::timeout(PICKED_WITHIN, picked_rx).await, Ok(Ok(()))) {
        lent.forget(&id);
        return Err(format!("{} didn't answer. Is the Outpost app open there?", helper.name));
    }
    match tokio::time::timeout(ANSWER_WITHIN, status_rx).await {
        Ok(Ok(status)) => Ok(Reply { status, stream: tokio_stream::wrappers::ReceiverStream::new(rx).boxed() }),
        _ => {
            lent.forget(&id);
            Err(format!("{} stopped answering.", helper.name))
        }
    }
}

fn require_lender(auth: &Auth) -> Result<(), ApiError> {
    if auth.account.role != outpost_core::accounts::Role::Owner {
        return Err(ApiError::forbidden("Only an owner's PC can be a helper."));
    }
    auth.require_person("Being a helper")
}

/// A lent PC asks for the next chat request for its Ollama (waits up to 25 s; 204 if none).
#[utoipa::path(post, path = "/api/helpers/chat/next", tag = "helpers", security(("bearer" = [])),
    responses((status = 200, description = "A request", body = LentRequest), (status = 204, description = "Nothing yet"),
        (status = 403, description = "Not an owner", body = ErrorBody)))]
pub async fn next(State(state): State<AppState>, auth: Auth) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    require_lender(&auth)?;
    let session = auth.session.id.clone();
    let lent = &state.lent_chat;
    let until = tokio::time::Instant::now() + WAIT;
    loop {
        let notified = lent.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let Some(req) = lent.pop(&session) {
            let picked = lent
                .waiting
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get_mut(&req.id)
                .and_then(|w| w.picked.take());
            // Whoever asked gave up already.
            let Some(picked) = picked else { continue };
            if picked.send(()).is_err() {
                lent.forget(&req.id);
                continue;
            }
            return Ok(Json(req).into_response());
        }
        if tokio::time::timeout_at(until, notified).await.is_err() {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    }
}

/// A lent PC's Ollama answer, streamed as it comes (Ollama's status in `x-ollama-status`).
#[utoipa::path(post, path = "/api/helpers/chat/{id}/reply", tag = "helpers", security(("bearer" = [])),
    params(("id" = String, Path, description = "The request")),
    responses((status = 204, description = "Passed on"), (status = 404, description = "Nobody's waiting for it", body = ErrorBody)))]
pub async fn reply(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<StatusCode> {
    require_lender(&auth)?;
    let status: u16 = headers
        .get("x-ollama-status")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let (status_tx, chunks) = {
        let mut waiting = state.lent_chat.waiting.lock().unwrap_or_else(|p| p.into_inner());
        let Some(w) = waiting.get_mut(&id).filter(|w| w.session_id == auth.session.id) else {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "Nobody's waiting for that answer any more."));
        };
        (w.status.take(), w.chunks.clone())
    };
    if let Some(s) = status_tx {
        let _ = s.send(status);
    }
    let mut stream = body.into_data_stream();
    let mut gone = false;
    while let Some(chunk) = stream.next().await {
        let item = chunk.map_err(|e| format!("The lent PC's answer broke off: {e}"));
        let failed = item.is_err();
        if chunks.send(item).await.is_err() {
            // Whoever asked stopped (e.g. Stop was pressed): end the upload, so the PC stops.
            gone = true;
            break;
        }
        if failed {
            break;
        }
    }
    drop(chunks);
    state.lent_chat.forget(&id);
    if gone {
        return Err(ApiError::new(StatusCode::GONE, "stopped", "The reply was stopped."));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outpost_engines::catalog::Badge;
    use std::time::Instant;

    #[test]
    fn strict_templates_get_their_system_notes_folded_in() {
        let body = serde_json::json!({ "model": "devstral", "messages": [
            { "role": "system", "content": "Be terse." },
            { "role": "system", "content": "Summary: Dan." },
            { "role": "user", "content": "hi" },
            { "role": "assistant", "content": "hello" },
            { "role": "system", "content": "It is 8 pm." },
            { "role": "user", "content": "lamp off" },
        ]});
        let roles = |b: &Value| b["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let one = fold_system(&body, 1);
        assert_eq!(roles(&one), ["system", "user", "assistant", "user"]);
        assert_eq!(one["messages"][0]["content"], "Be terse.\n\nSummary: Dan.");
        assert_eq!(one["messages"][3]["content"], "It is 8 pm.\n\nlamp off");
        let none = fold_system(&body, 2);
        assert_eq!(roles(&none), ["user", "assistant", "user"]);
        assert_eq!(none["messages"][0]["content"], "Be terse.\n\nSummary: Dan.\n\nhi");
        assert!(refuses_system("Jinja Exception: Only user, assistant and tool roles are supported, got system."));
        assert!(!refuses_system("model 'x' not found"));
        assert_eq!(fold_system(&serde_json::json!({ "x": 1 }), 1), serde_json::json!({ "x": 1 }));
    }

    fn model(name: &str, badges: Vec<Badge>) -> InstalledModel {
        InstalledModel {
            model: name.into(),
            size_bytes: 1,
            parameter_size: None,
            quantization: None,
            family: None,
            badges,
            here: true,
            lent: None,
        }
    }

    fn helper(name: &str, vram_mb: u64, models: Vec<InstalledModel>) -> Helper {
        Helper {
            session_id: name.into(),
            name: name.into(),
            vram_mb,
            ram_mb: 0,
            ready: Vec::new(),
            paused: false,
            trainer: false,
            chat: true,
            chat_models: models,
            last_seen: Instant::now(),
        }
    }

    #[test]
    fn lent_models_join_the_list_and_bigger_cards_take_shared_ones() {
        let here = vec![model("small:8b", vec![]), model("qwen3-embedding:0.6b", vec![Badge::Embedding])];
        let pc = helper(
            "Dan's PC",
            16 * 1024,
            vec![model("small:8b", vec![]), model("big:14b", vec![Badge::Tools]), model("qwen3-embedding:0.6b", vec![Badge::Embedding])],
        );
        let out = merge(here.clone(), vec![pc.clone()], 8 * 1024);
        let find = |n: &str| out.iter().find(|m| m.model == n).unwrap().clone();
        assert_eq!(out.len(), 3);
        // Only there: runs there.
        assert!(!find("big:14b").here);
        assert_eq!(find("big:14b").lent.as_deref(), Some("Dan's PC"));
        assert_eq!(find("big:14b").badges, vec![Badge::Tools]);
        // In both, and the PC's card is bigger: runs there, still downloaded here.
        assert!(find("small:8b").here);
        assert_eq!(find("small:8b").lent.as_deref(), Some("Dan's PC"));
        // Embeddings stay here.
        assert_eq!(find("qwen3-embedding:0.6b").lent, None);

        // A smaller card than the server's only adds what the server lacks.
        let out = merge(here, vec![helper("Laptop", 6 * 1024, vec![model("small:8b", vec![]), model("tiny:1b", vec![])])], 8 * 1024);
        assert_eq!(out.iter().find(|m| m.model == "small:8b").unwrap().lent, None);
        assert_eq!(out.iter().find(|m| m.model == "tiny:1b").unwrap().lent.as_deref(), Some("Laptop"));
    }

    #[tokio::test]
    async fn a_request_waits_for_its_helper_and_reads_its_answer() {
        let lent = LentChat::default();
        let (picked_tx, picked_rx) = oneshot::channel();
        let (status_tx, status_rx) = oneshot::channel();
        let (tx, rx) = mpsc::channel(4);
        lent.waiting.lock().unwrap().insert(
            "r1".into(),
            Waiter { session_id: "pc".into(), picked: Some(picked_tx), status: Some(status_tx), chunks: tx },
        );
        lent.push("pc", LentRequest { id: "r1".into(), path: "/api/chat".into(), body: Value::Null });
        assert!(lent.pop("other").is_none());
        assert_eq!(lent.pop("pc").unwrap().id, "r1");
        let w = lent.waiting.lock().unwrap().get_mut("r1").unwrap().picked.take().unwrap();
        w.send(()).unwrap();
        picked_rx.await.unwrap();
        let (status, chunks) = {
            let mut waiting = lent.waiting.lock().unwrap();
            let w = waiting.get_mut("r1").unwrap();
            (w.status.take().unwrap(), w.chunks.clone())
        };
        status.send(200).unwrap();
        chunks.send(Ok(Bytes::from_static(b"{\"a\":"))).await.unwrap();
        chunks.send(Ok(Bytes::from_static(b"1}"))).await.unwrap();
        drop(chunks);
        lent.forget("r1");
        let reply = Reply { status: status_rx.await.unwrap(), stream: tokio_stream::wrappers::ReceiverStream::new(rx).boxed() };
        assert!(reply.ok());
        let v: Value = reply.json().await.unwrap();
        assert_eq!(v["a"], 1);
        // A helper leaving drops what it was asked.
        lent.push("pc", LentRequest { id: "r2".into(), path: "/api/chat".into(), body: Value::Null });
        lent.drop_helper("pc");
        assert!(lent.pop("pc").is_none());
    }
}
