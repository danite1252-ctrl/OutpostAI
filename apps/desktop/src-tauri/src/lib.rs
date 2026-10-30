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
use outpost_gateway::{remote, AppState, Config};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
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
    let body = serde_json::json!({ "close_to_tray": on }).to_string();
    std::fs::write(settings_file(), body).map_err(|e| e.to_string())
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
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut builder = TrayIconBuilder::with_id("outpost")
        .tooltip("Outpost")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
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

/// Engines started by the gateway inside this process, if we started one.
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
    /// This app's paired-device token, once signed in.
    #[serde(default)]
    token: Option<String>,
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
}

fn server_file() -> PathBuf {
    data_dir().join("remote.json")
}

fn load_server() -> Option<Server> {
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
        words: remote::words(&s.fingerprint),
        host: s.host,
        port: s.port,
        name: s.name,
        fingerprint: s.fingerprint,
        token: s.token,
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

/// Use this server from now on (the window then restarts the app).
#[tauri::command]
fn save_server(host: String, port: u16, fingerprint: String, name: String, token: Option<String>) -> Result<(), String> {
    write_server(&Server { host, port, fingerprint, name, token })
}

/// Keep (or forget) this app's sign-in on the server.
#[tauri::command]
fn save_server_token(token: Option<String>) -> Result<(), String> {
    let mut server = load_server().ok_or("This app isn't using a server.")?;
    server.token = token;
    write_server(&server)
}

/// Go back to running Outpost on this PC (the window then restarts the app).
#[tauri::command]
fn forget_server() -> Result<(), String> {
    match std::fs::remove_file(server_file()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Using a server: open the tunnel the window will talk to. Bound here, before the
/// window loads, so its address is known when the window asks.
fn start_server_tunnel(server: &Server) -> std::io::Result<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let _ = GATEWAY_URL.set(format!("http://{addr}"));
    let (host, port, fp) = (server.host.clone(), server.port, server.fingerprint.clone());
    tracing::info!(%host, port, "using the Outpost server (no gateway on this PC)");
    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => remote::tunnel(listener, host, port, fp).await,
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
            forget_server
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
