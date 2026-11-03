//! Tools on a PC, for chats from elsewhere (the phone). The desktop app tells the server
//! which tool servers it runs (with their tools), and which ones each chat and persona uses;
//! then it waits for calls (`POST /api/pc-tools/next`) and answers them
//! (`POST /api/pc-tools/answers/{id}`). A phone's message asks for its chat's tools on that
//! PC (`use_pc_tools`), and each call the model makes goes there.
//!
//! Only your own PCs' tools are used, for your own chats: each PC is known by its sign-in.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Notify};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::pc_tools::PcToolServer;
use crate::AppState;

/// A PC that hasn't checked in for this long is away.
const ONLINE_FOR: Duration = Duration::from_secs(60);
/// Longest a PC's ask for work waits.
const WAIT: Duration = Duration::from_secs(25);

/// What a PC says it has.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct Register {
    /// The PC's name ("DESKTOP-DAN").
    pub name: String,
    #[serde(default)]
    pub servers: Vec<PcToolServer>,
    /// Chat id → the tool servers it uses (its own choice).
    #[serde(default)]
    pub chats: HashMap<String, Vec<String>>,
    /// Persona id → its tool servers (chats without their own choice).
    #[serde(default)]
    pub personas: HashMap<String, Vec<String>>,
    /// The chat folders on this PC (their ids): replies from elsewhere reach them through it.
    #[serde(default)]
    pub folders: Vec<String>,
}

#[derive(Debug, Clone)]
struct Device {
    account_id: String,
    seen: Instant,
    has: Register,
}

/// Tool calls waiting for PCs, and their answers.
#[derive(Clone, Default)]
pub struct DeviceTools {
    devices: Arc<Mutex<HashMap<String, Device>>>,
    queue: Arc<Mutex<HashMap<String, VecDeque<Value>>>>,
    waiting: Arc<Mutex<HashMap<String, (String, oneshot::Sender<Value>)>>>,
    notify: Arc<Notify>,
}

/// The tools a chat uses on one of your PCs.
pub struct ChatTools {
    pub session_id: String,
    pub pc: String,
    pub servers: Vec<PcToolServer>,
}

impl DeviceTools {
    fn put(&self, session: &str, account: &str, has: Register) {
        self.devices
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session.into(), Device { account_id: account.into(), seen: Instant::now(), has });
    }

    /// The tools this chat uses on your PC that checked in most recently (if it's on).
    pub fn for_chat(&self, account_id: &str, chat_id: &str, persona_id: Option<&str>) -> Option<ChatTools> {
        let devices = self.devices.lock().unwrap_or_else(|p| p.into_inner());
        let mut mine: Vec<(&String, &Device)> =
            devices.iter().filter(|(_, d)| d.account_id == account_id && d.seen.elapsed() < ONLINE_FOR).collect();
        mine.sort_by_key(|(_, d)| std::cmp::Reverse(d.seen));
        for (session, d) in mine {
            let ids = d.has.chats.get(chat_id).or_else(|| persona_id.and_then(|p| d.has.personas.get(p)));
            let Some(ids) = ids.filter(|i| !i.is_empty()) else { continue };
            let servers: Vec<PcToolServer> = d.has.servers.iter().filter(|s| ids.contains(&s.id)).cloned().collect();
            if !servers.is_empty() {
                return Some(ChatTools { session_id: session.clone(), pc: d.has.name.clone(), servers });
            }
        }
        None
    }

    /// Your PC that has this chat folder and is on now (its session and name).
    pub fn for_folder(&self, account_id: &str, folder_id: &str) -> Option<(String, String)> {
        let devices = self.devices.lock().unwrap_or_else(|p| p.into_inner());
        devices
            .iter()
            .filter(|(_, d)| d.account_id == account_id && d.seen.elapsed() < ONLINE_FOR && d.has.folders.iter().any(|f| f == folder_id))
            .max_by_key(|(_, d)| d.seen)
            .map(|(s, d)| (s.clone(), d.has.name.clone()))
    }

    /// Ask a PC to run something (`request` is `{server, tool, arguments}`); its answer.
    pub async fn call(&self, session_id: &str, mut request: Value, within: Duration) -> Result<Value, String> {
        let id = uuid::Uuid::new_v4().to_string();
        request["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).insert(id.clone(), (session_id.into(), tx));
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).entry(session_id.into()).or_default().push_back(request);
        self.notify.notify_waiters();
        let answer = tokio::time::timeout(within, rx).await;
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
        for q in self.queue.lock().unwrap_or_else(|p| p.into_inner()).values_mut() {
            q.retain(|r| r["id"] != json!(id));
        }
        let v = match answer {
            Ok(Ok(v)) => v,
            _ => return Err("Your PC didn't answer in time (is the Outpost app open on it?).".into()),
        };
        match v.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None => Ok(v),
        }
    }

    fn pop(&self, session: &str) -> Option<Value> {
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).get_mut(session)?.pop_front()
    }

    fn answer(&self, session: &str, id: &str, value: Value) -> bool {
        let mut w = self.waiting.lock().unwrap_or_else(|p| p.into_inner());
        match w.remove(id) {
            Some((s, tx)) if s == session => tx.send(value).is_ok(),
            Some(other) => {
                w.insert(id.into(), other);
                false
            }
            None => false,
        }
    }
}

/// What a phone may know: your PCs that run tools now.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceView {
    pub name: String,
    pub servers: Vec<String>,
}

/// A PC says which tools it has and which chats use them (every 20 s or so, and on changes).
#[utoipa::path(post, path = "/api/pc-tools/register", tag = "chats", security(("bearer" = [])),
    request_body = Register, responses((status = 204, description = "Noted")))]
pub async fn register(State(state): State<AppState>, auth: Auth, Json(req): Json<Register>) -> ApiResult<StatusCode> {
    auth.require_person("Offering tools")?;
    state.device_tools.put(&auth.session.id, &auth.account.id, req);
    Ok(StatusCode::NO_CONTENT)
}

/// Your PCs whose tools are usable now.
#[utoipa::path(get, path = "/api/pc-tools", tag = "chats", security(("bearer" = [])),
    responses((status = 200, description = "PCs", body = [DeviceView])))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> Json<Vec<DeviceView>> {
    let devices = state.device_tools.devices.lock().unwrap_or_else(|p| p.into_inner());
    Json(
        devices
            .values()
            .filter(|d| d.account_id == auth.account.id && d.seen.elapsed() < ONLINE_FOR)
            .map(|d| DeviceView { name: d.has.name.clone(), servers: d.has.servers.iter().map(|s| s.name.clone()).collect() })
            .collect(),
    )
}

/// A PC asks for the next tool call to run (waits up to 25 s; 204 if none).
#[utoipa::path(post, path = "/api/pc-tools/next", tag = "chats", security(("bearer" = [])),
    responses((status = 200, description = "A call: {id, server, tool, arguments}"), (status = 204, description = "Nothing yet")))]
pub async fn next(State(state): State<AppState>, auth: Auth) -> ApiResult<Response> {
    auth.require_person("Offering tools")?;
    let dt = &state.device_tools;
    let session = auth.session.id.clone();
    let until = tokio::time::Instant::now() + WAIT;
    loop {
        let notified = dt.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let Some(req) = dt.pop(&session) {
            return Ok(Json(req).into_response());
        }
        if tokio::time::timeout_at(until, notified).await.is_err() {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    }
}

/// A PC's answer to a call (`{text, is_error}` or `{error}`).
#[utoipa::path(post, path = "/api/pc-tools/answers/{id}", tag = "chats", security(("bearer" = [])),
    params(("id" = String, Path, description = "The call")),
    responses((status = 204, description = "Passed on"), (status = 404, description = "Nobody's waiting", body = ErrorBody)))]
pub async fn answer(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(value): Json<Value>) -> ApiResult<StatusCode> {
    if state.device_tools.answer(&auth.session.id, &id, value) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "Nobody's waiting for that answer any more."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pc_tools::PcTool;

    fn server(id: &str) -> PcToolServer {
        PcToolServer { id: id.into(), name: id.to_uppercase(), tools: vec![PcTool { name: "t".into(), description: String::new(), input_schema: Value::Null, read_only: None }] }
    }

    #[tokio::test]
    async fn a_phones_chat_uses_its_tools_on_your_pc() {
        let dt = DeviceTools::default();
        dt.put(
            "pc1",
            "dan",
            Register {
                name: "DESKTOP".into(),
                servers: vec![server("ha"), server("google")],
                chats: HashMap::from([("c1".to_string(), vec!["google".to_string()])]),
                personas: HashMap::from([("p1".to_string(), vec!["ha".to_string()])]),
                folders: vec!["pc-abc".into()],
            },
        );
        // Its folders are reachable through it, for its owner only.
        assert_eq!(dt.for_folder("dan", "pc-abc"), Some(("pc1".to_string(), "DESKTOP".to_string())));
        assert!(dt.for_folder("sj", "pc-abc").is_none());
        assert!(dt.for_folder("dan", "pc-other").is_none());
        // The chat's own choice, else its persona's; nobody else's PC.
        assert_eq!(dt.for_chat("dan", "c1", Some("p1")).unwrap().servers[0].id, "google");
        assert_eq!(dt.for_chat("dan", "c2", Some("p1")).unwrap().servers[0].id, "ha");
        assert!(dt.for_chat("dan", "c2", None).is_none());
        assert!(dt.for_chat("sj", "c1", Some("p1")).is_none());

        // A call waits for the PC, which answers it.
        let pc = dt.clone();
        let worker = tokio::spawn(async move {
            loop {
                if let Some(req) = pc.pop("pc1") {
                    let id = req["id"].as_str().unwrap().to_string();
                    assert!(!pc.answer("pc2", &id, json!({ "text": "no" })));
                    assert!(pc.answer("pc1", &id, json!({ "text": "Lamp off", "is_error": false })));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        let v = dt.call("pc1", json!({ "server": "ha", "tool": "t", "arguments": {} }), Duration::from_secs(5)).await.unwrap();
        worker.await.unwrap();
        assert_eq!(v["text"], "Lamp off");
        // Nobody answering: a clear error.
        let err = dt.call("pc1", json!({}), Duration::from_millis(50)).await.unwrap_err();
        assert!(err.contains("didn't answer"));
    }
}
