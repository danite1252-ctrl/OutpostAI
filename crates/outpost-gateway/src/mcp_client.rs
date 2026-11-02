//! An MCP client: Outpost's desktop app uses it to run tools (MCP servers) on the user's PC
//! for chats (see [`crate::pc_tools`] for the server's side).
//!
//! Two transports: **stdio** (a program the app starts, one JSON-RPC message per line;
//! `npx` and `uvx` use Outpost's own Node and uv when it has them, see
//! [`outpost_engines::runtimes`]) and **Streamable HTTP** (JSON-RPC POSTed to one address,
//! answered as JSON or an event stream). Connections are kept open and reused, and restarted
//! when a server's settings change or it stops.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

/// The MCP version asked for (servers may answer with an older one they support).
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Starting a server can mean downloading it first (`npx`, `uvx`).
const START_WITHIN: Duration = Duration::from_secs(180);
const LIST_WITHIN: Duration = Duration::from_secs(60);
/// A tool's longest run (a little under the chat's wait for it).
pub const CALL_WITHIN: Duration = Duration::from_secs(170);

/// How to reach one MCP server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    /// `stdio` or `http`.
    pub transport: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

impl ServerConfig {
    fn fingerprint(&self) -> String {
        serde_json::to_string(&(&self.transport, &self.command, &self.args, &self.env, &self.url, &self.headers)).unwrap_or_default()
    }
}

/// One of a server's tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
    /// The server says it only reads (changes nothing).
    #[serde(default)]
    pub read_only: Option<bool>,
}

/// What a tool call gave back, as text for the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CallResult {
    pub text: String,
    pub is_error: bool,
}

/// A runtime the server needs that isn't on this PC (`node` or `uv`).
pub fn missing_runtime(error: &str) -> Option<&'static str> {
    if error.starts_with("needs node:") {
        Some("node")
    } else if error.starts_with("needs uv:") {
        Some("uv")
    } else {
        None
    }
}

/// The program and arguments that start a stdio server, with `npx`/`uvx` resolved.
pub fn resolve_command(cfg: &ServerConfig, engines_root: &Path) -> Result<(PathBuf, Vec<String>, BTreeMap<String, String>), String> {
    use outpost_engines::runtimes;
    let mut env = cfg.env.clone();
    let cmd = cfg.command.trim();
    let path_with = |dir: &Path| {
        let mut parts = vec![dir.to_path_buf()];
        if let Some(p) = std::env::var_os("PATH") {
            parts.extend(std::env::split_paths(&p));
        }
        std::env::join_paths(parts).ok().map(|p| p.to_string_lossy().to_string())
    };
    match cmd {
        "npx" | "node" => {
            if let Some(node) = runtimes::node_bin(engines_root) {
                // Its own folder first on the PATH, so what it starts finds node too.
                if let Some(p) = node.parent().and_then(path_with) {
                    env.entry("PATH".into()).or_insert(p);
                }
                if cmd == "node" {
                    return Ok((node, cfg.args.clone(), env));
                }
                if let Some(script) = runtimes::npx_script(engines_root) {
                    let mut args = vec![script.to_string_lossy().to_string()];
                    args.extend(cfg.args.iter().cloned());
                    return Ok((node, args, env));
                }
            }
            match runtimes::on_path(cmd) {
                Some(p) => Ok((p, cfg.args.clone(), env)),
                None => Err("needs node: this tool runs on Node.js, which isn't on this PC yet.".into()),
            }
        }
        "uvx" | "uv" => {
            if let Some(uv) = runtimes::uv_bin(engines_root) {
                let uv_root = engines_root.join("uv");
                for (k, v) in [
                    ("UV_PYTHON_INSTALL_DIR", uv_root.join("python")),
                    ("UV_CACHE_DIR", uv_root.join("cache")),
                    ("UV_TOOL_DIR", uv_root.join("tools")),
                ] {
                    env.entry(k.into()).or_insert(v.to_string_lossy().to_string());
                }
                env.entry("UV_PYTHON_PREFERENCE".into()).or_insert("managed".into());
                env.entry("UV_NO_PROGRESS".into()).or_insert("1".into());
                let mut args: Vec<String> = if cmd == "uvx" { vec!["tool".into(), "run".into()] } else { Vec::new() };
                args.extend(cfg.args.iter().cloned());
                return Ok((uv, args, env));
            }
            match runtimes::on_path(cmd) {
                Some(p) => Ok((p, cfg.args.clone(), env)),
                None => Err("needs uv: this tool runs on Python (through uv), which isn't on this PC yet.".into()),
            }
        }
        "" => Err("Give the command that starts it.".into()),
        other => Ok((PathBuf::from(other), cfg.args.clone(), env)),
    }
}

type Waiting = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

struct StdioConn {
    stdin: tokio::sync::Mutex<ChildStdin>,
    waiting: Waiting,
    alive: Arc<AtomicBool>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    _child: Mutex<Child>,
}

struct HttpConn {
    client: reqwest::Client,
    url: String,
    headers: BTreeMap<String, String>,
    session: Mutex<Option<String>>,
    version: Mutex<Option<String>>,
    alive: AtomicBool,
}

enum Conn {
    Stdio(StdioConn),
    Http(HttpConn),
}

/// A started, initialised server.
struct Session {
    fingerprint: String,
    conn: Conn,
    next: AtomicU64,
    tools: Mutex<Option<Vec<ToolInfo>>>,
}

fn rpc_error(v: &Value) -> Option<String> {
    let e = v.get("error")?;
    let msg = e.get("message").and_then(Value::as_str).unwrap_or("The tool server reported an error.");
    Some(msg.to_string())
}

impl Session {
    fn alive(&self) -> bool {
        match &self.conn {
            Conn::Stdio(c) => c.alive.load(Ordering::Relaxed),
            Conn::Http(c) => c.alive.load(Ordering::Relaxed),
        }
    }

    fn why_stopped(&self) -> String {
        match &self.conn {
            Conn::Stdio(c) => {
                let tail: Vec<String> = c.stderr.lock().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect();
                if tail.is_empty() {
                    "The tool server stopped.".into()
                } else {
                    format!("The tool server stopped: {}", tail.join(" | "))
                }
            }
            Conn::Http(_) => "The tool server closed the connection.".into(),
        }
    }

    async fn request(&self, method: &str, params: Value, within: Duration) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let answer = match &self.conn {
            Conn::Stdio(c) => {
                let (tx, rx) = oneshot::channel();
                c.waiting.lock().unwrap_or_else(|p| p.into_inner()).insert(id, tx);
                let line = format!("{msg}\n");
                let wrote = {
                    let mut stdin = c.stdin.lock().await;
                    match stdin.write_all(line.as_bytes()).await {
                        Ok(()) => stdin.flush().await,
                        Err(e) => Err(e),
                    }
                };
                if wrote.is_err() {
                    c.waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                    return Err(self.why_stopped());
                }
                match tokio::time::timeout(within, rx).await {
                    Ok(Ok(r)) => r,
                    Ok(Err(_)) => Err(self.why_stopped()),
                    Err(_) => {
                        c.waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                        Err(format!("The tool took too long ({} s).", within.as_secs()))
                    }
                }
            }
            Conn::Http(c) => match tokio::time::timeout(within, c.post(&msg, Some(id))).await {
                Ok(r) => r,
                Err(_) => Err(format!("The tool took too long ({} s).", within.as_secs())),
            },
        }?;
        match rpc_error(&answer) {
            Some(e) => Err(e),
            None => Ok(answer.get("result").cloned().unwrap_or(Value::Null)),
        }
    }

    async fn notify(&self, method: &str) {
        let msg = json!({ "jsonrpc": "2.0", "method": method });
        match &self.conn {
            Conn::Stdio(c) => {
                let mut stdin = c.stdin.lock().await;
                let _ = stdin.write_all(format!("{msg}\n").as_bytes()).await;
                let _ = stdin.flush().await;
            }
            Conn::Http(c) => {
                let _ = c.post(&msg, None).await;
            }
        }
    }

    async fn initialize(&self) -> Result<(), String> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "Outpost", "version": env!("CARGO_PKG_VERSION") }
                }),
                START_WITHIN,
            )
            .await?;
        if let Conn::Http(c) = &self.conn {
            let v = result.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION).to_string();
            *c.version.lock().unwrap_or_else(|p| p.into_inner()) = Some(v);
        }
        self.notify("notifications/initialized").await;
        Ok(())
    }

    async fn list_tools(&self) -> Result<Vec<ToolInfo>, String> {
        if let Some(t) = self.tools.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            return Ok(t);
        }
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let page = self.request("tools/list", params, LIST_WITHIN).await?;
            for t in page.get("tools").and_then(Value::as_array).cloned().unwrap_or_default() {
                let Some(name) = t.get("name").and_then(Value::as_str) else { continue };
                out.push(ToolInfo {
                    name: name.to_string(),
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .or_else(|| t.get("title").and_then(Value::as_str))
                        .unwrap_or_default()
                        .to_string(),
                    input_schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                    read_only: t.get("annotations").and_then(|a| a.get("readOnlyHint")).and_then(Value::as_bool),
                });
            }
            cursor = page.get("nextCursor").and_then(Value::as_str).map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        *self.tools.lock().unwrap_or_else(|p| p.into_inner()) = Some(out.clone());
        Ok(out)
    }
}

/// A tool result's content, as text.
pub fn result_text(result: &Value) -> CallResult {
    let mut parts: Vec<String> = Vec::new();
    for c in result.get("content").and_then(Value::as_array).cloned().unwrap_or_default() {
        let kind = c.get("type").and_then(Value::as_str).unwrap_or_default();
        let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        match kind {
            "text" => parts.push(s("text")),
            "image" => parts.push(format!("[an image ({})]", s("mimeType"))),
            "audio" => parts.push(format!("[audio ({})]", s("mimeType"))),
            "resource" => {
                let r = c.get("resource").cloned().unwrap_or_default();
                match r.get("text").and_then(Value::as_str) {
                    Some(t) => parts.push(t.to_string()),
                    None => parts.push(format!("[{}]", r.get("uri").and_then(Value::as_str).unwrap_or("a file"))),
                }
            }
            "resource_link" => parts.push(format!("[{}: {}]", s("name"), s("uri"))),
            _ => {}
        }
    }
    if parts.is_empty() {
        if let Some(sc) = result.get("structuredContent") {
            parts.push(sc.to_string());
        }
    }
    CallResult { text: parts.join("\n\n"), is_error: result.get("isError").and_then(Value::as_bool).unwrap_or(false) }
}

impl HttpConn {
    /// POST one message; for a request (`id`), its answer (as JSON or from an event stream).
    async fn post(&self, msg: &Value, id: Option<u64>) -> Result<Value, String> {
        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        for (k, v) in &self.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(s) = self.session.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            req = req.header("Mcp-Session-Id", s);
        }
        if let Some(v) = self.version.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            req = req.header("MCP-Protocol-Version", v);
        }
        let resp = req.json(msg).send().await.map_err(|e| format!("Can't reach the tool server: {e}"))?;
        if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *self.session.lock().unwrap_or_else(|p| p.into_inner()) = Some(s.to_string());
        }
        let status = resp.status().as_u16();
        if status == 404 && self.session.lock().unwrap_or_else(|p| p.into_inner()).is_some() {
            // Its session ended: start again next time.
            self.alive.store(false, Ordering::Relaxed);
            return Err("The tool server's session ended; try again.".into());
        }
        if status == 401 || status == 403 {
            return Err(format!("The tool server refused the key or token ({status}). Check it in Tools."));
        }
        let Some(id) = id else { return Ok(Value::Null) };
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("The tool server said {status}: {}", text.chars().take(300).collect::<String>()));
        }
        let is_stream = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let matches = |v: &Value| v.get("id").and_then(Value::as_u64) == Some(id) && (v.get("result").is_some() || v.get("error").is_some());
        if !is_stream {
            let v: Value = resp.json().await.map_err(|e| format!("The tool server's answer was garbled: {e}"))?;
            if let Some(list) = v.as_array() {
                return list.iter().find(|x| matches(x)).cloned().ok_or_else(|| "The tool server didn't answer.".into());
            }
            return Ok(v);
        }
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut data = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("The tool server's answer broke off: {e}"))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim_end_matches(['\r', '\n']);
                if let Some(d) = line.strip_prefix("data:") {
                    data.push_str(d.trim_start());
                } else if line.is_empty() && !data.is_empty() {
                    if let Ok(v) = serde_json::from_str::<Value>(&data) {
                        if matches(&v) {
                            return Ok(v);
                        }
                    }
                    data.clear();
                }
            }
        }
        if let Ok(v) = serde_json::from_str::<Value>(&data) {
            if matches(&v) {
                return Ok(v);
            }
        }
        Err("The tool server didn't answer.".into())
    }
}

async fn start(cfg: &ServerConfig, engines_root: &Path) -> Result<Session, String> {
    let conn = match cfg.transport.as_str() {
        "stdio" => {
            let (program, args, env) = resolve_command(cfg, engines_root)?;
            let mut cmd = Command::new(&program);
            cmd.args(&args)
                .envs(&env)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            #[cfg(windows)]
            {
                // No console window for it.
                cmd.creation_flags(0x0800_0000);
            }
            let mut child = cmd.spawn().map_err(|e| format!("Couldn't start {}: {e}", program.display()))?;
            let stdin = child.stdin.take().ok_or("No input to the tool server.")?;
            let stdout = child.stdout.take().ok_or("No output from the tool server.")?;
            let stderr = child.stderr.take();
            let waiting: Waiting = Default::default();
            let alive = Arc::new(AtomicBool::new(true));
            let tail: Arc<Mutex<VecDeque<String>>> = Default::default();
            if let Some(err) = stderr {
                let tail = tail.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(err).lines();
                    while let Ok(Some(l)) = lines.next_line().await {
                        let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
                        t.push_back(l.chars().take(300).collect());
                        if t.len() > 6 {
                            t.pop_front();
                        }
                    }
                });
            }
            tokio::spawn(read_stdio(stdout, waiting.clone(), alive.clone()));
            Conn::Stdio(StdioConn {
                stdin: tokio::sync::Mutex::new(stdin),
                waiting,
                alive,
                stderr: tail,
                _child: Mutex::new(child),
            })
        }
        "http" => Conn::Http(HttpConn {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .build()
                .map_err(|e| e.to_string())?,
            url: cfg.url.trim().to_string(),
            headers: cfg.headers.clone(),
            session: Mutex::new(None),
            version: Mutex::new(None),
            alive: AtomicBool::new(true),
        }),
        _ => return Err("A tool server runs as a program (stdio) or at an address (http).".into()),
    };
    let session = Session { fingerprint: cfg.fingerprint(), conn, next: AtomicU64::new(1), tools: Mutex::new(None) };
    session.initialize().await?;
    Ok(session)
}

/// Read a stdio server's messages: answers go to whoever asked; its own requests (ping,
/// roots) get a short answer; notifications are ignored.
async fn read_stdio(stdout: tokio::process::ChildStdout, waiting: Waiting, alive: Arc<AtomicBool>) {
    let mut lines = BufReader::new(stdout).lines();
    let reply_to = |v: &Value| -> Option<(u64, Result<Value, String>)> {
        let id = v.get("id")?.as_u64()?;
        if v.get("method").is_some() {
            return None;
        }
        Some((id, Ok(v.clone())))
    };
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if let Some((id, answer)) = reply_to(&v) {
            if let Some(tx) = waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(&id) {
                let _ = tx.send(answer);
            }
        }
        // Requests from the server (ping, roots/list...) aren't answered here: servers carry
        // on without them (we offer no client capabilities).
    }
    alive.store(false, Ordering::Relaxed);
    // Wake everyone still waiting.
    waiting.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

/// The app's open connections to its tool servers.
pub struct Pool {
    engines_root: PathBuf,
    sessions: tokio::sync::Mutex<HashMap<String, Arc<Session>>>,
}

impl Pool {
    /// `engines_root` is where Outpost keeps Node and uv (`<data>/engines`).
    pub fn new(engines_root: PathBuf) -> Self {
        Self { engines_root, sessions: Default::default() }
    }

    async fn session(&self, cfg: &ServerConfig) -> Result<Arc<Session>, String> {
        let mut sessions = self.sessions.lock().await;
        if let Some(s) = sessions.get(&cfg.id) {
            if s.alive() && s.fingerprint == cfg.fingerprint() {
                return Ok(s.clone());
            }
            sessions.remove(&cfg.id);
        }
        let s = Arc::new(start(cfg, &self.engines_root).await?);
        sessions.insert(cfg.id.clone(), s.clone());
        Ok(s)
    }

    /// A server's tools (asked once per connection).
    pub async fn tools(&self, cfg: &ServerConfig) -> Result<Vec<ToolInfo>, String> {
        let s = self.session(cfg).await?;
        match s.list_tools().await {
            Err(_) if !s.alive() => self.session(cfg).await?.list_tools().await,
            other => other,
        }
    }

    /// Run a tool. A server that had stopped is started again once.
    pub async fn call(&self, cfg: &ServerConfig, tool: &str, arguments: Value) -> Result<CallResult, String> {
        let params = json!({ "name": tool, "arguments": arguments });
        let s = self.session(cfg).await?;
        let result = match s.request("tools/call", params.clone(), CALL_WITHIN).await {
            Err(_) if !s.alive() => self.session(cfg).await?.request("tools/call", params, CALL_WITHIN).await?,
            other => other?,
        };
        Ok(result_text(&result))
    }

    /// Close a server's connection (stopping its program).
    pub async fn close(&self, id: &str) {
        self.sessions.lock().await.remove(id);
    }

    /// Close every connection whose server isn't in `keep` (removed or switched off).
    pub async fn keep_only(&self, keep: &[String]) {
        self.sessions.lock().await.retain(|id, _| keep.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_results_read_as_text() {
        let r = result_text(&json!({ "content": [
            { "type": "text", "text": "Sunny, 24°C" },
            { "type": "image", "data": "...", "mimeType": "image/png" },
            { "type": "resource", "resource": { "uri": "file:///a.txt", "text": "inside" } }
        ] }));
        assert_eq!(r.text, "Sunny, 24°C\n\n[an image (image/png)]\n\ninside");
        assert!(!r.is_error);
        let e = result_text(&json!({ "content": [{ "type": "text", "text": "no such repo" }], "isError": true }));
        assert!(e.is_error);
        let s = result_text(&json!({ "structuredContent": { "temp": 24 } }));
        assert_eq!(s.text, "{\"temp\":24}");
    }

    #[test]
    fn npx_and_uvx_need_their_runtimes() {
        let root = std::env::temp_dir().join(format!("outpost-mcp-rt-{}", std::process::id()));
        let cfg = ServerConfig { id: "a".into(), name: "A".into(), transport: "stdio".into(), command: "uvx".into(), args: vec!["mcp-server-time".into()], ..Default::default() };
        match resolve_command(&cfg, &root) {
            // A uvx on this PATH is fine too.
            Ok((p, args, _)) => {
                assert!(p.to_string_lossy().contains("uvx"));
                assert_eq!(args, vec!["mcp-server-time".to_string()]);
            }
            Err(e) => assert_eq!(missing_runtime(&e), Some("uv")),
        }
        let other = ServerConfig { command: "/opt/tool".into(), args: vec!["--x".into()], ..cfg.clone() };
        let (p, args, _) = resolve_command(&other, &root).unwrap();
        assert_eq!(p, PathBuf::from("/opt/tool"));
        assert_eq!(args, vec!["--x".to_string()]);
        // Outpost's own uv runs uvx as `uv tool run`.
        let bin = root.join("uv").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(outpost_engines::versions::exe("uv")), b"").unwrap();
        let (p, args, env) = resolve_command(&cfg, &root).unwrap();
        assert!(p.ends_with(outpost_engines::versions::exe("uv")));
        assert_eq!(args, vec!["tool".to_string(), "run".into(), "mcp-server-time".into()]);
        assert!(env.contains_key("UV_CACHE_DIR"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
