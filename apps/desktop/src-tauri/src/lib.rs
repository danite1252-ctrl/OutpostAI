//! Outpost desktop shell.
//!
//! The window is just a client of the local gateway. On start-up we check whether
//! a gateway already answers on 127.0.0.1:7420 (e.g. the standalone service); if not,
//! we run one inside this process so the app works on its own.

use outpost_gateway::Config;
use tracing_subscriber::EnvFilter;

/// The gateway address the UI should call.
#[tauri::command]
fn gateway_url() -> String {
    format!("http://{}", Config::local_default().bind)
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("outpost_gateway=info,outpost_core=info,outpost_desktop_lib=info")),
        )
        .try_init();

    tauri::Builder::default()
        .setup(|_app| {
            let config = Config::local_default();
            tauri::async_runtime::spawn(async move {
                if outpost_gateway::is_running(config.bind).await {
                    tracing::info!(bind = %config.bind, "using the Outpost gateway that is already running");
                    return;
                }
                if let Err(e) = outpost_gateway::serve(config).await {
                    tracing::error!(error = %e, "Outpost gateway stopped");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![gateway_url])
        .run(tauri::generate_context!())
        .expect("error while running Outpost");
}
