//! Outpost desktop shell.
//!
//! The window is just a client of the local gateway. On start-up we check whether
//! a gateway already answers on 127.0.0.1:7420 (e.g. the standalone service); if not,
//! we run one inside this process so the app works on its own. It listens on this PC
//! only, or on 0.0.0.0:7420 when "Allow other devices on my network" is on
//! (`OUTPOST_BIND` overrides both). When the app exits we
//! stop the engines it started, so Ollama and ComfyUI don't linger holding VRAM.
//!
//! With "Keep running in the system tray" on (the default), closing the window only hides
//! it: phones, apps and the queue keep working, and the tray icon opens it again or quits.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use outpost_engines::EngineManager;
use outpost_gateway::{AppState, Config};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

/// Closing the window hides it to the tray instead of quitting.
static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(true);

/// This window's own settings (not the gateway's): `<data>/desktop-settings.json`.
fn settings_file() -> PathBuf {
    Config::local_default().data_dir.join("desktop-settings.json")
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

/// The gateway address the UI should call.
#[tauri::command]
fn gateway_url() -> String {
    format!("http://{}", Config::local_default().bind)
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
    outpost_gateway::diagnostics::init(
        &Config::local_default().data_dir,
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
            let config = Config::local_default();
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
        .invoke_handler(tauri::generate_handler![gateway_url, before_update, get_close_to_tray, set_close_to_tray])
        .build(tauri::generate_context!())
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
