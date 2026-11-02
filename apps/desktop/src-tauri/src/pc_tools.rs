//! Tools on this PC for chats: MCP servers the app runs here (web search, GitHub, a
//! browser...). Their settings (and keys) stay in this PC's app settings; the server only
//! sees each tool's name and description, sent with a message, and asks the app to run a tool
//! (a `file_request` with `op: "mcp"`) while the reply streams.
//!
//! Which servers a chat uses: its persona's set, unless the chat was given its own.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use outpost_gateway::mcp_client::{self, Pool, ServerConfig, ToolInfo};
use outpost_gateway::Config;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A tool server as kept here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// The library entry it came from, if any.
    #[serde(default)]
    pub catalog_id: Option<String>,
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
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Tools switched off (never offered to the model).
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

fn yes() -> bool {
    true
}

impl McpServer {
    fn config(&self) -> ServerConfig {
        ServerConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            transport: self.transport.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            env: self.env.clone(),
            url: self.url.clone(),
            headers: self.headers.clone(),
        }
    }
}

fn engines_root() -> std::path::PathBuf {
    Config::local_default().data_dir.join("engines")
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(engines_root()))
}

fn all() -> Vec<McpServer> {
    serde_json::from_value(crate::read_settings()["mcp_servers"].clone()).unwrap_or_default()
}

fn save_all(list: &[McpServer]) -> Result<(), String> {
    crate::save_setting_value("mcp_servers", serde_json::to_value(list).map_err(|e| e.to_string())?)
}

fn find(id: &str) -> Result<McpServer, String> {
    all().into_iter().find(|s| s.id == id).ok_or_else(|| "That tool isn't set up on this PC any more.".into())
}

fn selections(key: &str) -> HashMap<String, Vec<String>> {
    serde_json::from_value(crate::read_settings()[key].clone()).unwrap_or_default()
}

fn new_id() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("mcp-{t:x}")
}

/// The tool servers set up on this PC.
#[tauri::command]
pub fn mcp_list() -> Vec<McpServer> {
    all()
}

/// Add a tool server, or change one (same id). Its connection restarts if how it's reached
/// changed.
#[tauri::command]
pub async fn mcp_save(server: McpServer) -> Result<McpServer, String> {
    let mut s = server;
    s.name = s.name.trim().chars().take(60).collect();
    if s.name.is_empty() {
        return Err("Give it a name.".into());
    }
    match s.transport.as_str() {
        "stdio" if s.command.trim().is_empty() => return Err("Give the command that starts it (e.g. npx).".into()),
        "http" if !(s.url.starts_with("https://") || s.url.starts_with("http://")) => {
            return Err("Give its address (https://...).".into())
        }
        "stdio" | "http" => {}
        _ => return Err("A tool server runs as a program (stdio) or at an address (http).".into()),
    }
    let mut list = all();
    if s.id.is_empty() {
        s.id = new_id();
        list.push(s.clone());
    } else if let Some(old) = list.iter_mut().find(|x| x.id == s.id) {
        *old = s.clone();
    } else {
        list.push(s.clone());
    }
    save_all(&list)?;
    if !s.enabled {
        pool().close(&s.id).await;
    }
    Ok(s)
}

/// Remove a tool server (and stop it).
#[tauri::command]
pub async fn mcp_remove(id: String) -> Result<(), String> {
    let list: Vec<McpServer> = all().into_iter().filter(|s| s.id != id).collect();
    save_all(&list)?;
    pool().close(&id).await;
    Ok(())
}

/// A server's tools (starting it if need be). An error that starts `needs node:` or
/// `needs uv:` means that runtime should be installed first.
#[tauri::command]
pub async fn mcp_tools(id: String) -> Result<Vec<ToolInfo>, String> {
    let s = find(&id)?;
    pool().tools(&s.config()).await
}

/// Run a tool for a reply (the chat's `file_request` with `op: "mcp"`). Answers as the server
/// expects: `{text, is_error}`, or `{error}`.
#[tauri::command]
pub async fn mcp_call(id: String, tool: String, arguments: Value) -> Value {
    let s = match find(&id) {
        Ok(s) => s,
        Err(e) => return json!({ "error": e }),
    };
    if !s.enabled || s.disabled_tools.contains(&tool) {
        return json!({ "error": format!("{tool} is switched off on this PC.") });
    }
    match pool().call(&s.config(), &tool, arguments).await {
        Ok(r) => json!({ "text": r.text, "is_error": r.is_error }),
        Err(e) => json!({ "error": e }),
    }
}

/// The servers a chat uses: the chat's own choice if it has one, else its persona's.
#[tauri::command]
pub fn mcp_selection(chat_id: Option<String>, persona_id: Option<String>) -> Vec<String> {
    let have: Vec<String> = all().into_iter().map(|s| s.id).collect();
    let chosen = chat_id
        .and_then(|c| selections("mcp_chats").remove(&c))
        .or_else(|| persona_id.and_then(|p| selections("mcp_personas").remove(&p)))
        .unwrap_or_default();
    chosen.into_iter().filter(|i| have.contains(i)).collect()
}

/// Give a chat its own set (`None`: back to its persona's).
#[tauri::command]
pub fn mcp_select_chat(chat_id: String, ids: Option<Vec<String>>) -> Result<(), String> {
    let mut map = selections("mcp_chats");
    match ids {
        Some(ids) => {
            map.insert(chat_id, ids);
        }
        None => {
            map.remove(&chat_id);
        }
    }
    crate::save_setting_value("mcp_chats", serde_json::to_value(map).map_err(|e| e.to_string())?)
}

/// A persona's default set (new chats with it start with these).
#[tauri::command]
pub fn mcp_select_persona(persona_id: String, ids: Vec<String>) -> Result<(), String> {
    let mut map = selections("mcp_personas");
    map.insert(persona_id, ids);
    crate::save_setting_value("mcp_personas", serde_json::to_value(map).map_err(|e| e.to_string())?)
}

/// One server's tools for a message (switched-off tools left out), or why it can't be used.
#[derive(Debug, Serialize)]
pub struct MessageTools {
    pub id: String,
    pub name: String,
    pub tools: Vec<ToolInfo>,
    pub error: Option<String>,
}

/// The tools of these servers, to send with a message (servers start if need be, together).
#[tauri::command]
pub async fn mcp_message_tools(ids: Vec<String>) -> Vec<MessageTools> {
    let list = all();
    let wanted: Vec<McpServer> = ids.iter().filter_map(|i| list.iter().find(|s| &s.id == i && s.enabled).cloned()).collect();
    let jobs = wanted.into_iter().map(|s| async move {
        match pool().tools(&s.config()).await {
            Ok(tools) => MessageTools {
                tools: tools.into_iter().filter(|t| !s.disabled_tools.contains(&t.name)).collect(),
                id: s.id,
                name: s.name,
                error: None,
            },
            Err(e) => MessageTools { id: s.id, name: s.name, tools: Vec::new(), error: Some(e) },
        }
    });
    futures_util::future::join_all(jobs).await
}

/// Node and uv here (for `npx` and `uvx` tools), and a download in progress.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Runtimes {
    pub node: bool,
    pub uv: bool,
    /// `node` or `uv` while downloading.
    pub installing: Option<String>,
    pub percent: Option<f32>,
    pub error: Option<String>,
}

static RUNTIMES: Mutex<Option<Runtimes>> = Mutex::new(None);

fn edit_runtimes(f: impl FnOnce(&mut Runtimes)) {
    let mut g = RUNTIMES.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(Default::default));
}

#[tauri::command]
pub fn mcp_runtimes() -> Runtimes {
    use outpost_engines::runtimes;
    let root = engines_root();
    let mut r = RUNTIMES.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_default();
    r.node = runtimes::node_bin(&root).is_some() || runtimes::on_path("npx").is_some();
    r.uv = runtimes::uv_bin(&root).is_some() || runtimes::on_path("uvx").is_some();
    r
}

/// Download Node or uv for tools (in the background; [`mcp_runtimes`] shows progress).
#[tauri::command]
pub fn mcp_install_runtime(which: String) -> Result<(), String> {
    if which != "node" && which != "uv" {
        return Err("Outpost can install node or uv.".into());
    }
    if mcp_runtimes().installing.is_some() {
        return Err("A download is running already.".into());
    }
    edit_runtimes(|r| {
        r.installing = Some(which.clone());
        r.percent = None;
        r.error = None;
    });
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(20)).build().expect("HTTP client");
        let root = engines_root();
        let progress = |done: u64, total: Option<u64>| {
            edit_runtimes(|r| r.percent = total.filter(|t| *t > 0).map(|t| done as f32 * 100.0 / t as f32));
        };
        let result = if which == "node" {
            outpost_engines::runtimes::ensure_node(&client, &root, progress).await
        } else {
            outpost_engines::runtimes::ensure_uv(&client, &root, progress).await
        };
        edit_runtimes(|r| {
            r.installing = None;
            r.percent = None;
            r.error = result.err().map(|e| format!("{e:#}"));
        });
    });
    Ok(())
}

/// What a start error means for the app: a runtime to install (`node`/`uv`), if any.
#[tauri::command]
pub fn mcp_missing_runtime(error: String) -> Option<String> {
    mcp_client::missing_runtime(&error).map(str::to_string)
}
