//! Outpost desktop shell.
//!
//! The window is just a client of the local gateway. On start-up we check whether
//! a gateway already answers on 127.0.0.1:7420 (e.g. the standalone service); if not,
//! we run one inside this process so the app works on its own. It listens on this PC
//! only, or on 0.0.0.0:7420 when "Allow other devices on my network" is on
//! (`OUTPOST_BIND` overrides both). When the app exits we
//! stop the engines it started, so Ollama and ComfyUI don't linger holding VRAM.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use outpost_engines::EngineManager;
use outpost_gateway::{AppState, Config};
use tauri::RunEvent;

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

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|_app| {
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
        .invoke_handler(tauri::generate_handler![gateway_url, before_update])
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
