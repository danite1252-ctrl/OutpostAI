//! Outpost desktop shell.
//!
//! The window is just a client of the local gateway. On start-up we check whether
//! a gateway already answers on 127.0.0.1:7420 (e.g. the standalone service); if not,
//! we run one inside this process so the app works on its own. It listens on this PC
//! only, or on 0.0.0.0:7420 when "Allow other devices on my network" is on
//! (`OUTPOST_BIND` overrides both). When the app exits we
//! stop the engines it started, so Ollama and ComfyUI don't linger holding VRAM.
//!
//! **Using a server instead** (`<data>/remote.json`): the app doesn't run a gateway at all.
//! It opens a tunnel on 127.0.0.1 to the server's encrypted port, pinned to the certificate
//! the person checked when connecting, and the window talks to that (see
//! `outpost_gateway::remote`). The window is then a paired device of the server.
//!
//! With "Keep running in the system tray" on (the default), closing the window only hides
//! it: phones, apps and the queue keep working, and the tray icon opens it again or quits.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use outpost_engines::EngineManager;
use outpost_gateway::helper_pc::HelperPc;
use outpost_gateway::{remote, AppState, Config};
use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

/// Closing the window hides it to the tray instead of quitting.
static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(true);

/// The data folder: `%LOCALAPPDATA%\Outpost`, or `Outpost Test` for a test build (made by
/// the test runner), so a test build installs and runs beside the real one without touching it.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

fn data_dir() -> PathBuf {
    DATA_DIR.get().cloned().unwrap_or_else(|| Config::local_default().data_dir)
}

/// This window's own settings (not the gateway's): `<data>/desktop-settings.json`.
fn settings_file() -> PathBuf {
    data_dir().join("desktop-settings.json")
}

fn load_settings() {
    let on = std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["close_to_tray"].as_bool())
        .unwrap_or(true);
    CLOSE_TO_TRAY.store(on, Ordering::Relaxed);
}

/// Whether closing the window keeps Outpost running in the tray.
#[tauri::command]
fn get_close_to_tray() -> bool {
    CLOSE_TO_TRAY.load(Ordering::Relaxed)
}

#[tauri::command]
fn set_close_to_tray(on: bool) -> Result<(), String> {
    CLOSE_TO_TRAY.store(on, Ordering::Relaxed);
    save_setting("close_to_tray", on)
}

fn read_settings() -> serde_json::Value {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// Change one setting, keeping the others.
fn save_setting(key: &str, on: bool) -> Result<(), String> {
    let mut all = read_settings();
    all[key] = serde_json::Value::Bool(on);
    std::fs::write(settings_file(), all.to_string()).map_err(|e| e.to_string())
}

// ------------------------------------------------ lending this PC's graphics card to the server

/// This PC as a helper for the server (only while the app uses one).
static HELPER: OnceLock<Arc<HelperPc>> = OnceLock::new();

/// The server, as the helper reaches it: through the tunnel, as whoever is signed in here.
struct ThroughTunnel;

impl outpost_gateway::helper_pc::Server for ThroughTunnel {
    fn url(&self) -> String {
        gateway_url()
    }
    fn tokens(&self) -> Vec<String> {
        let Some(server) = load_server() else { return Vec::new() };
        let mut out: Vec<String> = server.token.into_iter().collect();
        for p in server.people {
            if !out.contains(&p.token) {
                out.push(p.token);
            }
        }
        out
    }
}

fn start_helper(app: &AppHandle) {
    let settings = read_settings();
    let enabled = settings["helper"].as_bool().unwrap_or(false);
    let paused = settings["helper_paused"].as_bool().unwrap_or(false);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // This PC's own Outpost folder (its ComfyUI and models), even in a test build.
        let dir = Config::local_default().data_dir;
        match HelperPc::new(&dir, enabled, paused).await {
            Ok(pc) => {
                let _ = ENGINES.set(pc.engines());
                let _ = HELPER.set(pc.clone());
                set_tray_pause(&app, enabled, paused);
                pc.run(ThroughTunnel).await;
            }
            Err(e) => tracing::warn!(error = %e, "couldn't set up this PC as a helper"),
        }
    });
}

fn helper() -> Result<&'static Arc<HelperPc>, String> {
    HELPER.get().ok_or_else(|| "This PC can lend its graphics card only while Outpost uses a server.".into())
}

/// The helper's state, or nothing when the app runs Outpost on this PC.
#[tauri::command]
fn helper_status() -> Option<outpost_gateway::helper_pc::HelperStatus> {
    HELPER.get().map(|h| h.status())
}

/// Lend this PC's graphics card to the server, or not; and pause it (e.g. for gaming).
#[tauri::command]
fn set_helper(app: AppHandle, enabled: bool, paused: bool) -> Result<(), String> {
    helper()?.set(enabled, paused);
    save_setting("helper", enabled)?;
    save_setting("helper_paused", paused)?;
    set_tray_pause(&app, enabled, paused);
    Ok(())
}

/// Every workflow, as this PC sees it (downloaded here, fits this card).
#[tauri::command]
async fn helper_packs() -> Result<Vec<outpost_gateway::helper_pc::LocalPack>, String> {
    Ok(helper()?.packs().await)
}

/// Download a workflow's models onto this PC.
#[tauri::command]
fn helper_install_pack(id: String) -> Result<(), String> {
    helper()?.install_pack(&id).map_err(|e| format!("{e:#}"))
}

/// Install ComfyUI on this PC.
#[tauri::command]
fn helper_install_comfyui() -> Result<(), String> {
    helper()?.install_comfyui().map_err(|e| format!("{e:#}"))
}

/// The tray's "Pause lending this PC" item, ticked while paused and shown while lending.
static TRAY_PAUSE: OnceLock<CheckMenuItem<tauri::Wry>> = OnceLock::new();

fn set_tray_pause(_app: &AppHandle, enabled: bool, paused: bool) {
    if let Some(item) = TRAY_PAUSE.get() {
        let _ = item.set_enabled(enabled);
        let _ = item.set_checked(enabled && paused);
    }
}

/// From the tray: pause or carry on lending.
fn toggle_tray_pause(app: &AppHandle) {
    let Some(h) = HELPER.get() else { return };
    let s = h.status();
    let paused = !s.paused;
    if s.enabled {
        let _ = set_helper(app.clone(), true, paused);
        // Tell the window, so Settings shows it.
        let _ = tauri::Emitter::emit(app, "helper-changed", paused);
    }
}

/// Bring the window back from the tray (or from behind other windows).
fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// The tray icon: click to open; right-click for Open and Quit.
fn tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Outpost", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Outpost", true, None::<&str>)?;
    let menu = if load_server().is_some() {
        // Using a server: pausing the lent graphics card (for a game) is one click away.
        let pause = CheckMenuItem::with_id(app, "pause", "Pause lending this PC", false, false, None::<&str>)?;
        let line = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(app, &[&open, &pause, &line, &quit])?;
        let _ = TRAY_PAUSE.set(pause);
        menu
    } else {
        Menu::with_items(app, &[&open, &quit])?
    };
    let mut builder = TrayIconBuilder::with_id("outpost")
        .tooltip("Outpost")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "pause" => toggle_tray_pause(app),
            // Quitting runs the normal exit: engines stop, and it's noted as a clean close.
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Engines started inside this process (by the gateway, or by the helper), if any.
static ENGINES: OnceLock<Arc<EngineManager>> = OnceLock::new();

/// Where the window's API calls go: this PC's gateway, or the tunnel to the server.
static GATEWAY_URL: OnceLock<String> = OnceLock::new();

/// The gateway address the UI should call.
#[tauri::command]
fn gateway_url() -> String {
    GATEWAY_URL
        .get()
        .cloned()
        .unwrap_or_else(|| format!("http://{}", Config::local_default().bind))
}

/// The Outpost server this app uses instead of its own, saved in `<data>/remote.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Server {
    host: String,
    port: u16,
    /// The certificate the person checked (SHA-256, hex); nothing else is accepted.
    fingerprint: String,
    /// Its name, as shown when connecting.
    name: String,
    /// The sign-in in use now (a paired-device token).
    #[serde(default)]
    token: Option<String>,
    /// Everyone paired on this PC (a shared PC), each with their own token.
    #[serde(default)]
    people: Vec<Person>,
    /// All the server's addresses, best first (its home-network address, then Tailscale),
    /// as it reports them. `host` (what was typed) is tried too.
    #[serde(default)]
    hosts: Vec<String>,
    /// In use. "Use Outpost on this PC instead" switches it off but keeps it, sign-ins and
    /// all, so going back needs no pairing.
    #[serde(default = "in_use")]
    active: bool,
}

fn in_use() -> bool {
    true
}

impl Server {
    /// The addresses to try, in order.
    fn all_hosts(&self) -> Vec<String> {
        let mut out = self.hosts.clone();
        if !out.iter().any(|h| h.eq_ignore_ascii_case(&self.host)) {
            out.push(self.host.clone());
        }
        out
    }
}

/// The tunnel's addresses, changed in place when the server reports new ones.
static HOSTS: OnceLock<outpost_gateway::remote::Hosts> = OnceLock::new();

/// One person's sign-in on a shared PC.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Person {
    account_id: String,
    name: String,
    avatar_color: String,
    token: String,
}

/// What the window is told about the server (with the check words to show in Settings).
#[derive(Debug, Serialize)]
struct ServerView {
    host: String,
    port: u16,
    name: String,
    words: Vec<String>,
    fingerprint: String,
    token: Option<String>,
    people: Vec<Person>,
    hosts: Vec<String>,
}

fn server_file() -> PathBuf {
    data_dir().join("remote.json")
}

/// The server this app uses now (none while it runs Outpost on this PC).
fn load_server() -> Option<Server> {
    load_saved().filter(|s| s.active)
}

/// The saved server, in use or not.
fn load_saved() -> Option<Server> {
    let text = std::fs::read_to_string(server_file()).ok()?;
    serde_json::from_str(&text).ok()
}

/// Saved readable by this user only (it holds a sign-in token).
fn write_server(server: &Server) -> Result<(), String> {
    let path = server_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(server).map_err(|e| e.to_string())?;
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// The server this app uses, or nothing when it runs Outpost on this PC.
#[tauri::command]
fn server_status() -> Option<ServerView> {
    load_server().map(|s| ServerView {
        hosts: s.all_hosts(),
        words: remote::words(&s.fingerprint),
        host: s.host,
        port: s.port,
        name: s.name,
        fingerprint: s.fingerprint,
        token: s.token,
        people: s.people,
    })
}

/// First look at a server: its certificate's check words and whether it needs an owner.
#[tauri::command]
async fn probe_server(address: String) -> Result<remote::Probe, String> {
    remote::probe(&address).await.map_err(|e| format!("{e:#}"))
}

/// The tunnel used while connecting (before switching over), replaced on each try.
static TRIAL: Mutex<Option<tokio::task::JoinHandle<()>>> = Mutex::new(None);

/// Open a tunnel to a server whose words the person checked, for setting it up or pairing
/// before switching over. Returns the address to call.
#[tauri::command]
async fn try_server(host: String, port: u16, fingerprint: String) -> Result<String, String> {
    let (addr, task) = remote::start_tunnel(host, port, fingerprint)
        .await
        .map_err(|e| format!("{e:#}"))?;
    if let Ok(mut trial) = TRIAL.lock() {
        if let Some(old) = trial.replace(task) {
            old.abort();
        }
    }
    Ok(format!("http://{addr}"))
}

/// Use this server from now on (the window then restarts the app), signed in as `person`.
/// Someone added on a PC already using this server joins the people already there.
#[tauri::command]
fn save_server(
    host: String,
    port: u16,
    fingerprint: String,
    name: String,
    token: Option<String>,
    person: Option<Person>,
) -> Result<(), String> {
    let same = load_saved().filter(|old| old.fingerprint.eq_ignore_ascii_case(&fingerprint));
    let hosts = same.as_ref().map(|old| old.hosts.clone()).unwrap_or_default();
    let mut people = same.map(|old| old.people).unwrap_or_default();
    if let Some(p) = person {
        people.retain(|q| q.account_id != p.account_id);
        people.push(p);
    }
    write_server(&Server { host, port, fingerprint, name, token, people, hosts, active: true })
}

/// The server's current addresses (from `GET /api/connection`), best first: kept for next
/// time, and used by the tunnel straight away.
#[tauri::command]
fn save_server_hosts(hosts: Vec<String>) -> Result<(), String> {
    let mut server = load_server().ok_or("This app isn't using a server.")?;
    let clean: Vec<String> = hosts
        .into_iter()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty() && h.len() < 256 && !h.contains(char::is_whitespace))
        .collect();
    if clean.is_empty() {
        return Ok(());
    }
    server.hosts = clean;
    if let Some(live) = HOSTS.get() {
        if let Ok(mut h) = live.write() {
            *h = server.all_hosts();
        }
    }
    write_server(&server)
}

/// Switch to (or, with `None`, sign out of) a sign-in on the server. Signing out also
/// forgets that person on this PC.
#[tauri::command]
fn save_server_token(token: Option<String>) -> Result<(), String> {
    let mut server = load_server().ok_or("This app isn't using a server.")?;
    if token.is_none() {
        if let Some(old) = &server.token {
            server.people.retain(|p| &p.token != old);
        }
    }
    server.token = token;
    write_server(&server)
}

/// Forget one person on this PC (their device sign-in stays on the server until unpaired).
#[tauri::command]
fn forget_person(account_id: String) -> Result<(), String> {
    let mut server = load_server().ok_or("This app isn't using a server.")?;
    if let Some(p) = server.people.iter().find(|p| p.account_id == account_id) {
        if server.token.as_ref() == Some(&p.token) {
            server.token = None;
        }
    }
    server.people.retain(|p| p.account_id != account_id);
    write_server(&server)
}

/// Go back to running Outpost on this PC (the window then restarts the app).
#[tauri::command]
fn forget_server(keep: Option<bool>) -> Result<(), String> {
    // Kept (the default): switched off, so going back later needs no pairing.
    if keep.unwrap_or(true) {
        if let Some(mut server) = load_saved() {
            server.active = false;
            return write_server(&server);
        }
        return Ok(());
    }
    match std::fs::remove_file(server_file()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// The server this app used before, while it runs Outpost on this PC (to go back to it).
#[tauri::command]
fn saved_server() -> Option<ServerView> {
    load_saved().filter(|s| !s.active).map(|s| ServerView {
        hosts: s.all_hosts(),
        words: remote::words(&s.fingerprint),
        host: s.host,
        port: s.port,
        name: s.name,
        fingerprint: s.fingerprint,
        token: None,
        people: Vec::new(),
    })
}

/// Use the saved server again (the window then restarts the app); its sign-ins still work.
#[tauri::command]
fn use_saved_server() -> Result<(), String> {
    let mut server = load_saved().ok_or("There's no saved server.")?;
    server.active = true;
    write_server(&server)
}

// ------------------------------------------------ this PC's gallery, for copying to a server

/// This PC's own Outpost (the normal install's data, even in a test build).
fn this_pc_db() -> Result<(PathBuf, PathBuf), String> {
    let dir = Config::local_default().data_dir;
    let db = dir.join("outpost.db");
    if !db.is_file() {
        return Err("There's no Outpost data on this PC to copy from.".into());
    }
    Ok((db, dir.join("files")))
}

#[derive(Debug, Serialize)]
struct LocalGallery {
    account_id: String,
    name: String,
    avatar_color: String,
    items: usize,
    bytes: i64,
    personas: usize,
}

/// The accounts on this PC's own Outpost, with how many gallery items and personas each has.
#[tauri::command]
async fn local_galleries() -> Result<Vec<LocalGallery>, String> {
    let (db, _) = this_pc_db()?;
    let pool = outpost_core::db::open_read_only(&db).await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for a in outpost_core::accounts::list(&pool).await.map_err(|e| e.to_string())? {
        if a.role == outpost_core::accounts::Role::Service {
            continue;
        }
        let items = outpost_core::media::all(&pool, &a.id).await.map_err(|e| e.to_string())?;
        let personas = outpost_core::personas::list_existing(&pool, &a.id).await.map(|p| p.len()).unwrap_or(0);
        if items.is_empty() && personas == 0 {
            continue;
        }
        out.push(LocalGallery {
            bytes: items.iter().map(|m| m.bytes).sum(),
            items: items.len(),
            personas,
            account_id: a.id,
            name: a.name,
            avatar_color: a.avatar_color,
        });
    }
    pool.close().await;
    Ok(out)
}

/// An account's personas on this PC, with the memories they were given by hand.
#[derive(Debug, Serialize)]
struct LocalPersonas {
    personas: Vec<outpost_core::personas::Persona>,
    /// Manual memories: each one's persona (`None` for every persona) and text.
    memories: Vec<outpost_core::memory::Memory>,
}

/// One account's personas and hand-added memories on this PC (after its PIN was given).
#[tauri::command]
async fn local_personas(account_id: String) -> Result<LocalPersonas, String> {
    if !UNLOCKED.lock().map(|u| u.contains(&account_id)).unwrap_or(false) {
        return Err("Enter that account's PIN first.".into());
    }
    let (db, _) = this_pc_db()?;
    let pool = outpost_core::db::open_read_only(&db).await.map_err(|e| e.to_string())?;
    let personas = outpost_core::personas::list_existing(&pool, &account_id).await.map_err(|e| e.to_string());
    let memories = outpost_core::memory::list(&pool, &account_id).await.map_err(|e| e.to_string());
    pool.close().await;
    Ok(LocalPersonas {
        personas: personas?,
        memories: memories?.into_iter().filter(|m| m.source == "manual").collect(),
    })
}

/// Accounts whose PIN was given in this run of the app (so copying asks once).
static UNLOCKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// One account's gallery on this PC, oldest first. Needs that account's PIN on this PC, so
/// on a shared PC nobody copies someone else's pictures.
#[tauri::command]
async fn local_gallery_items(account_id: String, pin: String) -> Result<Vec<outpost_core::media::Media>, String> {
    let (db, _) = this_pc_db()?;
    let pool = outpost_core::db::open_read_only(&db).await.map_err(|e| e.to_string())?;
    let ok = outpost_core::auth::secret_matches(&pool, &account_id, &pin).await.unwrap_or(false);
    if !ok {
        pool.close().await;
        // Slow down guessing (this database is read-only, so it has no lockout counter).
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        return Err("That isn't the PIN for that account on this PC.".into());
    }
    if let Ok(mut u) = UNLOCKED.lock() {
        if !u.contains(&account_id) {
            u.push(account_id.clone());
        }
    }
    let items = outpost_core::media::all(&pool, &account_id).await.map_err(|e| e.to_string());
    pool.close().await;
    items
}

/// One gallery file from this PC, as raw bytes (after `local_gallery_items` took the PIN).
#[tauri::command]
async fn local_gallery_file(account_id: String, id: String) -> Result<tauri::ipc::Response, String> {
    if !UNLOCKED.lock().map(|u| u.contains(&account_id)).unwrap_or(false) {
        return Err("Enter that account's PIN first.".into());
    }
    let (db, files) = this_pc_db()?;
    let pool = outpost_core::db::open_read_only(&db).await.map_err(|e| e.to_string())?;
    let found = outpost_core::media::get(&pool, &files, &account_id, &id).await;
    pool.close().await;
    let (_, path) = found.map_err(|_| "That picture isn't on this PC any more.".to_string())?;
    let data = tokio::fs::read(&path).await.map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    Ok(tauri::ipc::Response::new(data))
}

/// Using a server: open the tunnel the window will talk to. Bound here, before the
/// window loads, so its address is known when the window asks.
fn start_server_tunnel(server: &Server) -> std::io::Result<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let _ = GATEWAY_URL.set(format!("http://{addr}"));
    let (port, fp) = (server.port, server.fingerprint.clone());
    let all = server.all_hosts();
    tracing::info!(hosts = %all.join(", "), port, "using the Outpost server (no gateway on this PC)");
    let hosts: remote::Hosts = Arc::new(std::sync::RwLock::new(all));
    let _ = HOSTS.set(hosts.clone());
    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => remote::tunnel(listener, hosts, port, fp).await,
            Err(e) => tracing::error!(error = %e, "couldn't open the tunnel to the server"),
        }
    });
    Ok(())
}

/// Just before an update installs (Windows closes Outpost to replace it): stop the engines
/// and note a normal exit, so nothing is left holding the graphics card and the next start
/// doesn't report a crash.
#[tauri::command]
fn before_update() {
    if let Some(engines) = ENGINES.get() {
        engines.shutdown();
    }
    outpost_gateway::diagnostics::clean_exit();
}

pub fn run() {
    // Logs to the console and <data>/logs; crash reports to <data>/crash-reports (never sent anywhere).
    let context = tauri::generate_context!();
    let mut dir = Config::local_default().data_dir;
    if context.config().identifier.ends_with(".test") {
        dir.set_file_name("Outpost Test");
    }
    let _ = DATA_DIR.set(dir);
    outpost_gateway::diagnostics::init(
        &data_dir(),
        "desktop",
        "outpost_gateway=info,outpost_core=info,outpost_engines=info,outpost_desktop_lib=info",
    );

    load_settings();
    let app = tauri::Builder::default()
        // Opening Outpost again (Start menu, shortcut) shows the one already running.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if CLOSE_TO_TRAY.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            if let Err(e) = tray(app) {
                // No tray (rare): closing the window quits as before.
                tracing::warn!(error = %e, "couldn't add the tray icon");
                CLOSE_TO_TRAY.store(false, Ordering::Relaxed);
            }
            if let Some(server) = load_server() {
                if let Err(e) = start_server_tunnel(&server) {
                    tracing::error!(error = %e, "couldn't open the tunnel to the server");
                }
                start_helper(app.handle());
                return Ok(());
            }
            let mut config = Config::local_default();
            config.data_dir = data_dir();
            tauri::async_runtime::spawn(async move {
                if outpost_gateway::is_running(config.bind).await {
                    tracing::info!(bind = %config.bind, "using the Outpost gateway that is already running");
                    return;
                }
                let mut state = match AppState::open(config).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "could not open Outpost data");
                        return;
                    }
                };
                match std::env::var("OUTPOST_BIND")
                    .ok()
                    .and_then(|v| v.parse::<SocketAddr>().ok())
                {
                    Some(bind) => Arc::make_mut(&mut state.config).bind = bind,
                    None => {
                        if let Err(e) = state.apply_network_setting().await {
                            tracing::warn!(error = %e, "could not read the network setting; staying on this PC");
                        }
                    }
                }
                let _ = ENGINES.set(state.engines.clone());
                if let Err(e) = outpost_gateway::serve_state(state).await {
                    tracing::error!(error = %e, "Outpost gateway stopped");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            gateway_url,
            before_update,
            get_close_to_tray,
            set_close_to_tray,
            server_status,
            probe_server,
            try_server,
            save_server,
            save_server_token,
            save_server_hosts,
            saved_server,
            use_saved_server,
            forget_person,
            forget_server,
            local_galleries,
            local_gallery_items,
            local_gallery_file,
            local_personas,
            helper_status,
            set_helper,
            helper_packs,
            helper_install_pack,
            helper_install_comfyui
        ])
        .build(context)
        .expect("error while building Outpost");

    app.run(|_handle, event| {
        if let RunEvent::Exit = event {
            if let Some(engines) = ENGINES.get() {
                engines.shutdown();
            }
            outpost_gateway::diagnostics::clean_exit();
        }
    });
}
