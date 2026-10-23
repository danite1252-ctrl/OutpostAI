//! Outpost desktop shell.
//!
//! The window is just a client of the local gateway. On start-up we check whether
//! a gateway already answers on 127.0.0.1:7420 (e.g. the standalone service); if not,
//! we run one inside this process so the app works on its own. When the app exits we
//! stop the engines it started, so Ollama and ComfyUI don't linger holding VRAM.

use std::sync::{Arc, OnceLock};

use outpost_engines::EngineManager;
use outpost_gateway::{AppState, Config};
use tauri::RunEvent;
use tracing_subscriber::EnvFilter;

/// Engines started by the gateway inside this process, if we started one.
static ENGINES: OnceLock<Arc<EngineManager>> = OnceLock::new();

/// The gateway address the UI should call.
#[tauri::command]
fn gateway_url() -> String {
    format!("http://{}", Config::local_default().bind)
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("outpost_gateway=info,outpost_core=info,outpost_engines=info,outpost_desktop_lib=info")
        }))
        .try_init();

    let app = tauri::Builder::default()
        .setup(|_app| {
            let config = Config::local_default();
            tauri::async_runtime::spawn(async move {
                if outpost_gateway::is_running(config.bind).await {
                    tracing::info!(bind = %config.bind, "using the Outpost gateway that is already running");
                    return;
                }
                let state = match AppState::open(config).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "could not open Outpost data");
                        return;
                    }
                };
                let _ = ENGINES.set(state.engines.clone());
                if let Err(e) = outpost_gateway::serve_state(state).await {
                    tracing::error!(error = %e, "Outpost gateway stopped");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![gateway_url])
        .build(tauri::generate_context!())
        .expect("error while building Outpost");

    app.run(|_handle, event| {
        if let RunEvent::Exit = event {
            if let Some(engines) = ENGINES.get() {
                engines.shutdown();
            }
        }
    });
}
