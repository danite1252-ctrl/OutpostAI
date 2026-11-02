//! Tools on this PC for chats: MCP servers the app runs here (web search, GitHub, a
//! browser...). Their settings (and keys) stay in this PC's app settings; the server only
//! sees each tool's name and description, sent with a message, and asks the app to run a tool
//! (a `file_request` with `op: "mcp"`) while the reply streams.
//!
//! Which servers a chat uses: its persona's set, unless the chat was given its own.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use outpost_gateway::mcp_client::{self, Pool, ServerConfig, ToolInfo};
use outpost_gateway::google_tools;
use outpost_gateway::mcp_oauth::{self, OAuthState};
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
    /// Its OAuth sign-in (tokens included): kept here, never sent to the app's pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthState>,
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
            headers: {
                let mut h = self.headers.clone();
                // Signed in: its token, over any key typed in.
                if let Some(token) = self.oauth.as_ref().and_then(|o| o.access_token.as_ref()) {
                    h.retain(|k, _| !k.eq_ignore_ascii_case("authorization"));
                    h.insert("Authorization".into(), format!("Bearer {token}"));
                }
                h
            },
        }
    }

    /// For the app's pages: without the sign-in's tokens, but saying whether it's signed in.
    fn for_page(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.remove("oauth");
            let st = self.oauth.as_ref();
            o.insert("signed_in".into(), json!(st.is_some_and(OAuthState::signed_in)));
            // An OAuth client the user made (not one Outpost registered).
            o.insert("oauth_client".into(), json!(st.filter(|s| !s.registered && !s.client_id.is_empty()).map(|s| s.client_id.clone())));
            // Built-in Google: does the sign-in cover what's switched on (else sign in again)?
            if self.is_google() {
                let ok = st.and_then(|s| s.scope.as_deref()).is_some_and(|g| google_tools::covers(g, &self.args));
                o.insert("scope_ok".into(), json!(ok));
            }
        }
        v
    }

    /// Outpost's own Google tools (not an MCP server).
    fn is_google(&self) -> bool {
        self.transport == "builtin" && self.command == "google"
    }

    /// The signed-in token, or the sign-in to ask for.
    fn token(&self) -> Result<String, String> {
        self.oauth
            .as_ref()
            .and_then(|o| o.access_token.clone())
            .ok_or_else(|| format!("needs sign-in: sign in to {} first (Tools › On this PC).", self.name))
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(20)).build().expect("HTTP client")
}

fn store_oauth(id: &str, oauth: Option<OAuthState>) -> Result<(), String> {
    let mut list = all();
    if let Some(s) = list.iter_mut().find(|s| s.id == id) {
        s.oauth = oauth;
    }
    save_all(&list)
}

/// Its token refreshed if it's run out (or `force`), and kept. One refresh at a time: some
/// services (Notion) replace the refresh token each time and cut off one used twice.
async fn refreshed(s: McpServer, force: bool) -> Result<McpServer, String> {
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Some(had) = s.oauth.as_ref().and_then(|o| o.access_token.clone()).filter(|_| s.oauth.as_ref().is_some_and(|o| o.refresh_token.is_some())) else {
        return Ok(s);
    };
    if !force && !s.oauth.as_ref().is_some_and(|o| o.needs_refresh(now())) {
        return Ok(s);
    }
    let _turn = ONE_AT_A_TIME.lock().await;
    // Another call may have refreshed it while this one waited: use that.
    let mut s = find(&s.id)?;
    let Some(st) = s.oauth.clone() else { return Ok(s) };
    if st.access_token.as_deref() != Some(had.as_str()) && !st.needs_refresh(now()) {
        return Ok(s);
    }
    let next = mcp_oauth::refresh(&http(), &st).await?;
    s.oauth = Some(next);
    store_oauth(&s.id, s.oauth.clone())?;
    Ok(s)
}

/// Whether a sign-in error is worth a refresh and another try.
fn can_refresh(s: &McpServer, e: &str) -> bool {
    mcp_client::needs_sign_in(e) && s.oauth.as_ref().is_some_and(|o| o.refresh_token.is_some())
}

async fn tools_of(s: McpServer) -> Result<Vec<ToolInfo>, String> {
    if s.is_google() {
        // Nothing to start: its tools follow its settings, once signed in.
        s.token()?;
        return Ok(google_tools::tools(&s.args));
    }
    let s = refreshed(s, false).await?;
    match pool().tools(&s.config()).await {
        Err(e) if can_refresh(&s, &e) => {
            let s = refreshed(s, true).await?;
            pool().tools(&s.config()).await
        }
        other => other,
    }
}

async fn call_on(s: McpServer, tool: &str, arguments: Value) -> Result<mcp_client::CallResult, String> {
    if s.is_google() {
        let s = refreshed(s, false).await?;
        return match google_tools::call(&http(), &s.token()?, &s.args, tool, &arguments).await {
            Err(e) if can_refresh(&s, &e) => {
                let s = refreshed(s, true).await?;
                google_tools::call(&http(), &s.token()?, &s.args, tool, &arguments).await
            }
            other => other,
        };
    }
    let s = refreshed(s, false).await?;
    match pool().call(&s.config(), tool, arguments.clone()).await {
        Err(e) if can_refresh(&s, &e) => {
            let s = refreshed(s, true).await?;
            pool().call(&s.config(), tool, arguments).await
        }
        other => other,
    }
}

/// Show an address in the default browser.
fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    r.map(|_| ()).map_err(|e| format!("Couldn't open the browser: {e}"))
}

/// Sign in to a tool server with OAuth: the browser opens at the service's sign-in page and
/// comes back to Outpost. `client_id` (and `client_secret`) for services where you make your
/// own OAuth client (Google); others register Outpost themselves. An error that starts
/// `needs client:` means this one needs a client ID.
#[tauri::command]
pub async fn mcp_sign_in(
    id: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    scope: Option<String>,
) -> Result<(), String> {
    let s = find(&id)?;
    if s.is_google() {
        return sign_in_google(s, client_id, client_secret).await;
    }
    if s.transport != "http" {
        return Err("Only tools at an address (http) sign in this way.".into());
    }
    let previous = s.oauth.clone();
    let manual = match client_id.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) {
        Some(c) => Some((c, client_secret.map(|x| x.trim().to_string()).filter(|x| !x.is_empty()))),
        // The client made for it last time.
        None => previous
            .as_ref()
            .filter(|p| !p.registered && !p.client_id.is_empty())
            .map(|p| (p.client_id.clone(), p.client_secret.clone())),
    };
    // The scopes asked for last time, unless new ones are given.
    let scope = scope.filter(|x| !x.trim().is_empty()).or_else(|| previous.as_ref().filter(|p| !p.registered).and_then(|p| p.scope.clone()));
    let st = mcp_oauth::sign_in(&http(), &s.url, manual, scope, previous.as_ref(), open_in_browser).await?;
    store_oauth(&id, Some(st))?;
    pool().close(&id).await;
    Ok(())
}

/// Sign in to Google for the built-in tools, asking for what its settings need.
async fn sign_in_google(s: McpServer, client_id: Option<String>, client_secret: Option<String>) -> Result<(), String> {
    let previous = s.oauth.clone().filter(|p| !p.client_id.is_empty());
    let (id, secret) = match client_id.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) {
        Some(c) => (c, client_secret),
        None => match &previous {
            Some(p) => (p.client_id.clone(), p.client_secret.clone()),
            None => return Err("needs client: enter your Google OAuth client ID (the library's steps say how to make one).".into()),
        },
    };
    let st = mcp_oauth::google_state(&id, secret, &google_tools::scopes(&s.args));
    let client = http();
    let st = mcp_oauth::sign_in_known(&client, st, open_in_browser).await?;
    if st.refresh_token.is_none() {
        tracing::warn!("Google gave no refresh token; the sign-in lasts an hour");
    }
    let account = match &st.access_token {
        Some(t) => google_tools::account_email(&client, t).await,
        None => None,
    };
    let mut list = all();
    if let Some(x) = list.iter_mut().find(|x| x.id == s.id) {
        x.oauth = Some(st);
        match account {
            Some(a) => {
                x.env.insert("account".into(), a);
            }
            None => {
                x.env.remove("account");
            }
        }
    }
    save_all(&list)
}

/// Sign out (its tokens are forgotten; a client ID you entered is kept).
#[tauri::command]
pub async fn mcp_sign_out(id: String) -> Result<(), String> {
    let s = find(&id)?;
    let kept = s.oauth.map(|o| OAuthState { access_token: None, refresh_token: None, expires_at: None, ..o });
    store_oauth(&id, kept)?;
    pool().close(&id).await;
    Ok(())
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
pub fn mcp_list() -> Vec<Value> {
    all().iter().map(McpServer::for_page).collect()
}

/// Add a tool server, or change one (same id). Its connection restarts if how it's reached
/// changed.
#[tauri::command]
pub async fn mcp_save(server: McpServer) -> Result<Value, String> {
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
        "builtin" if s.command != "google" => return Err("There's no built-in tool called that.".into()),
        "stdio" | "http" | "builtin" => {}
        _ => return Err("A tool server runs as a program (stdio) or at an address (http).".into()),
    }
    let mut list = all();
    // The sign-in stays (pages never send it), unless the address changed.
    s.oauth = None;
    if s.id.is_empty() {
        s.id = new_id();
        list.push(s.clone());
    } else if let Some(old) = list.iter_mut().find(|x| x.id == s.id) {
        if old.url == s.url {
            s.oauth = old.oauth.clone();
        }
        *old = s.clone();
    } else {
        list.push(s.clone());
    }
    save_all(&list)?;
    if !s.enabled {
        pool().close(&s.id).await;
    }
    Ok(s.for_page())
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
    tools_of(find(&id)?).await
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
    match call_on(s, &tool, arguments).await {
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

/// Every chat's and persona's tool servers (for chats from the phone, which run them here).
#[tauri::command]
pub fn mcp_all_selections() -> Value {
    json!({ "chats": selections("mcp_chats"), "personas": selections("mcp_personas") })
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
        let (id, name, off) = (s.id.clone(), s.name.clone(), s.disabled_tools.clone());
        match tools_of(s).await {
            Ok(tools) => MessageTools {
                tools: tools.into_iter().filter(|t| !off.contains(&t.name)).collect(),
                id,
                name,
                error: None,
            },
            Err(e) => MessageTools { id, name, tools: Vec::new(), error: Some(e) },
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
