//! Outpost gateway: the one HTTP API every client uses (desktop, phone, Hermes, scripts).
//!
//! Run it on its own with the `outpost-gateway` binary, or embed it with [`serve`]
//! (the desktop app does this when no gateway is already running).

pub mod config;
pub mod create;
pub mod diagnostics;
pub mod discovery;
pub mod error;
pub mod extract;
pub mod helper_pc;
pub mod helpers;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod openapi;
pub mod remote;
pub mod routes;
pub mod setup_code;
pub mod tailscale;
pub mod tls;
pub mod train;
pub mod updates;
pub mod webhooks;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

pub use config::Config;
use outpost_engines::assets::AssetStore;
use outpost_engines::comfy::Comfy;
use outpost_engines::models::ModelStore;
use outpost_engines::EngineManager;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    /// Client for quick calls to local engines (3 s timeout).
    pub http: reqwest::Client,
    /// Client for model replies, which can stream for minutes (no overall timeout).
    pub llm: reqwest::Client,
    /// Serialises first-run setup so two owners can't be created at once.
    pub setup_lock: Arc<Mutex<()>>,
    /// Installs, starts and supervises Ollama and ComfyUI.
    pub engines: Arc<EngineManager>,
    /// Chat and embedding models in Ollama: list, download, remove.
    pub models: Arc<ModelStore>,
    /// The address the gateway actually bound, once it is listening.
    pub listening: Arc<OnceLock<SocketAddr>>,
    /// ComfyUI's API.
    pub comfy: Arc<Comfy>,
    /// Model files for workflow packs.
    pub assets: Arc<AssetStore>,
    /// The Create job queue (the worker runs in [`serve_state`]).
    pub queue: Arc<create::Queue>,
    /// The encrypted port for phones, once it is running (network access on).
    pub tls: Arc<OnceLock<tls::TlsInfo>>,
    /// This PC's Tailscale address, once Outpost answers on it.
    pub tailnet: Arc<OnceLock<IpAddr>>,
    /// Dataset captioning in progress.
    pub captions: routes::training::Captions,
    /// What the last update check found.
    pub updates: updates::Updates,
    /// The one-time code for setting up a server from another device (while there's no owner).
    pub setup: setup_code::SetupCodes,
    /// Helper PCs lending their graphics card for Create jobs.
    pub helpers: helpers::Helpers,
}

impl AppState {
    /// Open the database in `config.data_dir` and build the state.
    pub async fn open(config: Config) -> anyhow::Result<Self> {
        let pool = outpost_core::db::open(&config.db_path()).await?;
        Ok(Self::with_pool(pool, config))
    }

    pub fn with_pool(pool: SqlitePool, config: Config) -> Self {
        // Engines are local; never route them through a system or corporate proxy.
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("HTTP client");
        let llm = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("HTTP client");
        let assets = AssetStore::new(&config.data_dir.join("models"));
        AppState {
            llm,
            engines: EngineManager::new(&config.data_dir),
            models: ModelStore::new(),
            pool,
            config: Arc::new(config),
            http,
            setup_lock: Arc::new(Mutex::new(())),
            listening: Arc::new(OnceLock::new()),
            comfy: Arc::new(Comfy::new()),
            assets,
            queue: create::new_queue(),
            tls: Arc::new(OnceLock::new()),
            tailnet: Arc::new(OnceLock::new()),
            captions: Default::default(),
            updates: Default::default(),
            setup: Default::default(),
            helpers: Default::default(),
        }
    }

    /// Apply the saved "Allow other devices on my network" setting: listen on every
    /// network interface (same port) when it is on, this PC only when it is off.
    /// A server (`config.server`) has it on until the owner turns it off.
    pub async fn apply_network_setting(&mut self) -> anyhow::Result<()> {
        let saved = outpost_core::settings::get(&self.pool, outpost_core::settings::NETWORK_ACCESS).await?;
        let open = match saved.as_deref() {
            Some(v) => v == "on",
            None => self.config.server,
        };
        let port = self.config.bind.port();
        let ip = if open {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        Arc::make_mut(&mut self.config).bind = SocketAddr::new(ip, port);
        Ok(())
    }
}

/// Largest request body for routes that take images: four 10 MB images as base64
/// plus the rest of the message.
pub const MAX_UPLOAD_BODY: usize = 64 * 1024 * 1024;

/// Origins allowed to call the API from a browser context: the desktop app in dev
/// and in release builds (Windows and Linux webviews use different schemes).
const DESKTOP_ORIGINS: [&str; 5] = [
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
];

pub fn router(state: AppState) -> Router {
    use routes::{
        accounts, auth, chat, create, downloads, engines, health, keys, models, network, openai, pairing, setup, system, training,
        voice, webhooks,
    };

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            DESKTOP_ORIGINS.into_iter().map(HeaderValue::from_static),
        ))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::PATCH])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::HeaderName::from_static("x-outpost-pin"),
        ]);

    Router::new()
        .route("/api/health", get(health::health))
        .route("/api/setup/owner", post(setup::create_owner))
        .route("/api/accounts/directory", get(auth::directory))
        .route("/api/auth/sign-in", post(auth::sign_in))
        .route("/api/auth/sign-out", post(auth::sign_out))
        .route("/api/me", get(auth::me))
        .route("/api/me/check-pin", post(auth::check_pin))
        .route("/api/me/secret", put(auth::change_my_secret))
        .route("/api/accounts", get(accounts::list).post(accounts::create))
        .route("/api/accounts/{id}", delete(accounts::delete))
        .route("/api/accounts/{id}/secret", put(accounts::reset_secret))
        .route("/api/engines", get(engines::status))
        .route("/api/engines/{id}/install", post(engines::install))
        .route("/api/engines/{id}/start", post(engines::start))
        .route("/api/engines/{id}/stop", post(engines::stop))
        .route("/v1/models", get(models::list_models))
        .route(
            "/v1/chat/completions",
            post(openai::chat_completions).layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY)),
        )
        .route("/api/personas", get(chat::list_personas).post(chat::create_persona))
        .route("/api/personas/{id}/send", post(chat::send_persona))
        .route("/api/personas/{id}/copy", post(chat::copy_persona_for_me))
        .route("/api/people", get(chat::people))
        .route(
            "/api/personas/{id}",
            patch(chat::update_persona).delete(chat::delete_persona),
        )
        .route("/api/chats", get(chat::list_chats).post(chat::create_chat))
        .route(
            "/api/chats/{id}",
            get(chat::get_chat).patch(chat::update_chat).delete(chat::delete_chat),
        )
        // Messages may carry images, so allow bigger bodies here (4 × 10 MB, base64).
        .route(
            "/api/chats/{id}/messages",
            post(chat::send_message).layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY)),
        )
        .route("/api/chats/{id}/messages/{message_id}", delete(chat::delete_message))
        .route("/api/files/{id}", get(chat::get_file))
        .route("/api/memory", get(chat::list_memory).post(chat::add_memory))
        .route(
            "/api/memory/settings",
            get(chat::memory_settings).put(chat::set_memory_settings),
        )
        .route("/api/memory/auto", delete(chat::clear_auto_memory))
        .route("/api/memory/{id}", delete(chat::delete_memory))
        .route("/api/keys", get(keys::list).post(keys::create))
        .route("/api/keys/{id}", delete(keys::revoke))
        .route(
            "/api/settings/network",
            get(network::get_network).put(network::set_network),
        )
        .route("/api/packs", get(create::list_packs))
        .route("/api/packs/{id}/install", post(create::install_pack))
        .route(
            "/api/loras",
            // A LoRA can be a couple of GB; the handler streams it to disk and caps it.
            get(create::list_loras).post(create::upload_lora).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/jobs",
            get(create::list_jobs)
                .post(create::create_job)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY)),
        )
        .route("/api/jobs/{id}", get(create::get_job).delete(create::cancel_job))
        .route("/api/queue", get(create::queue))
        .route("/api/gallery", get(create::gallery))
        .route("/api/gallery/shared", get(create::shared_gallery))
        .route("/api/media/{id}/share", post(create::share_media))
        .route(
            "/api/media/import",
            post(create::import_media).layer(DefaultBodyLimit::max(create::MAX_IMPORT_BYTES)),
        )
        .route("/api/media/{id}", get(create::get_media).delete(create::delete_media))
        .route("/api/downloads", get(downloads::list))
        .route("/api/pairing", post(pairing::create_code))
        .route("/api/pairing/{id}", get(pairing::code_status))
        .route("/api/invites", post(pairing::create_invite))
        .route("/api/pair/accept", post(pairing::accept))
        .route("/api/pair/hello", post(pairing::hello))
        .route("/api/pair/complete", post(pairing::complete))
        .route("/api/devices", get(pairing::list_devices))
        .route("/api/connection", get(network::connection))
        .route("/api/devices/{id}", delete(pairing::revoke_device))
        .route("/api/training", get(training::status).put(training::set_settings))
        .route("/api/training/trigger-check", get(training::trigger_check))
        .route("/api/datasets", get(training::list_datasets).post(training::create_dataset))
        .route(
            "/api/datasets/{id}",
            get(training::get_dataset)
                .patch(training::update_dataset)
                .delete(training::delete_dataset),
        )
        .route(
            "/api/datasets/{id}/images",
            post(training::add_image).layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY)),
        )
        .route(
            "/api/datasets/{id}/images/{image_id}",
            get(training::get_image)
                .patch(training::set_caption)
                .delete(training::delete_image),
        )
        .route("/api/datasets/{id}/import", post(training::import_folder))
        .route("/api/datasets/{id}/caption", post(training::caption))
        .route("/api/datasets/{id}/train", post(training::start_training))
        .route("/api/trainings/{id}", get(training::get_training))
        .route("/api/trainings/{id}/samples/{file}", get(training::get_sample))
        .route("/api/trainings/{id}/use", post(training::use_checkpoint))
        .route("/api/characters", get(training::list_characters).post(training::add_character))
        .route("/api/characters/lora-files", get(training::lora_files))
        .route(
            "/api/characters/{id}",
            patch(training::rename_character).delete(training::delete_character),
        )
        .route("/api/voice", get(voice::status))
        .route(
            "/api/voice/transcribe",
            post(voice::transcribe).layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY)),
        )
        .route("/api/voice/speak", post(voice::speak))
        .route("/v1/audio/speech", post(voice::openai_speech))
        .route(
            "/v1/audio/transcriptions",
            post(voice::openai_transcriptions).layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY)),
        )
        .route("/mcp", post(mcp::handle).get(mcp::no_stream))
        .route("/api/integrations", get(mcp::integrations))
        .route("/api/webhooks", get(webhooks::list).post(webhooks::create))
        .route("/api/webhooks/{id}", delete(webhooks::delete))
        .route("/api/webhooks/{id}/test", post(webhooks::test))
        .route("/api/diagnostics", get(system::diagnostics))
        .route("/api/diagnostics/seen", post(system::mark_seen))
        .route("/api/diagnostics/log", get(system::recent_log))
        .route("/api/diagnostics/crashes", delete(system::delete_all_crashes))
        .route("/api/diagnostics/crashes/{id}", get(system::crash).delete(system::delete_crash))
        .route("/api/helpers", get(helpers::list))
        .route("/api/helpers/poll", post(helpers::poll))
        .route("/api/helpers/jobs/{id}/input/{key}", get(helpers::input))
        .route("/api/helpers/jobs/{id}/lora", get(helpers::lora))
        .route("/api/helpers/jobs/{id}/progress", post(helpers::progress))
        .route(
            "/api/helpers/jobs/{id}/result",
            post(helpers::result).layer(DefaultBodyLimit::max(create::MAX_IMPORT_BYTES)),
        )
        .route("/api/helpers/jobs/{id}/finish", post(helpers::finish))
        .route("/api/updates", get(system::update_status))
        .route("/api/updates/check", post(system::check_updates))
        .route("/api/updates/install", post(system::install_server_update))
        .route("/api/system/restart", post(system::restart))
        .route("/api/updates/settings", put(system::update_settings))
        .route("/api/hardware", get(models::hardware))
        .route("/api/models", get(models::overview))
        .route("/api/models/pull", post(models::pull))
        .route("/api/models/remove", post(models::remove))
        .route(
            "/openapi.json",
            get(|| async { Json(<openapi::ApiDoc as utoipa::OpenApi>::openapi()) }),
        )
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

/// Run the gateway until Ctrl+C.
pub async fn serve(config: Config) -> anyhow::Result<()> {
    tracing::info!(data_dir = %config.data_dir.display(), "opening Outpost data");
    let state = AppState::open(config).await?;
    serve_state(state).await
}

/// Run the gateway with state the caller already holds (the desktop app keeps the
/// engine manager so it can stop engines when the window closes).
pub async fn serve_state(state: AppState) -> anyhow::Result<()> {
    let bind = state.config.bind;
    let engines = state.engines.clone();
    // Bind first: if the port is taken, fail before starting any engines.
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let _ = state.listening.set(listener.local_addr().unwrap_or(bind));
    if bind.ip().is_loopback() {
        tracing::info!(%bind, "Outpost gateway listening (this PC only)");
    } else {
        tracing::info!(%bind, lan = ?routes::network::lan_ip(), "Outpost gateway listening on the network (app keys only)");
    }
    let starter = engines.clone();
    tokio::spawn(async move { starter.init().await });
    // LoRAs are per account now: move the ones with a known owner out of the shared folder.
    routes::create::claim_shared_loras(&state).await;
    tokio::spawn(create::worker(state.clone()));
    tokio::spawn(helpers::watchdog(state.clone()));
    // The one call to the internet: a daily look for a newer release (can be switched off).
    tokio::spawn(updates::background(state.clone()));
    // With network access on, phones get the encrypted port next door, and the
    // Outpost announces itself on the network.
    let background = start_remote_access(&state).await;
    // No owner yet (a server set up from another device): keep a setup code ready.
    tokio::spawn(setup_code::keep_fresh(state.clone()));
    let result = axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;
    // Stop the encrypted port, the Tailscale listener and the announcement with the main one.
    for handle in background {
        handle.abort();
    }
    engines.shutdown();
    result?;
    Ok(())
}

/// Ctrl+C, or (on Linux) the stop signal a service manager sends: `systemctl stop`, a
/// reboot. Either way Outpost closes normally, so the next start doesn't report a crash.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => tracing::info!("asked to stop: closing Outpost"),
                }
                return;
            }
            Err(e) => tracing::warn!(error = %e, "can't listen for the stop signal"),
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// The port phones use: the API port plus one (7421 by default).
pub fn secure_port(api_port: u16) -> u16 {
    api_port.wrapping_add(1)
}

/// Everything that answers beyond this PC: with network access on, the encrypted port on
/// every interface (the plain port already listens everywhere); with only Tailscale
/// allowed, both ports on the Tailscale address alone.
async fn start_remote_access(state: &AppState) -> Vec<tokio::task::JoinHandle<()>> {
    let bind = state.config.bind;
    let lan = !bind.ip().is_loopback();
    // A server answers on Tailscale too unless the owner said no (so phones reach it away
    // from home), like network access.
    let tailscale_on = match outpost_core::settings::get(&state.pool, outpost_core::settings::TAILSCALE_ACCESS).await {
        Ok(Some(v)) => v == "on",
        _ => state.config.server,
    };
    let ts_ip = if tailscale_on { tailscale::ip() } else { None };
    if tailscale_on && ts_ip.is_none() {
        tracing::warn!("Tailscale access is on, but this PC has no Tailscale address right now");
    }
    let mut handles = Vec::new();
    if lan {
        match start_secure_port(state, bind.ip(), true).await {
            Ok(h) => handles.push(h),
            Err(e) => tracing::warn!(error = %e, "couldn't start the encrypted port for phones"),
        }
        if let Some(ip) = ts_ip {
            // Listening everywhere already covers the Tailscale address.
            let _ = state.tailnet.set(IpAddr::V4(ip));
        }
    } else if let Some(ip) = ts_ip {
        let ip = IpAddr::V4(ip);
        match tokio::net::TcpListener::bind(SocketAddr::new(ip, bind.port())).await {
            Ok(listener) => {
                tracing::info!(%ip, port = bind.port(), "API listening on Tailscale (app keys only)");
                let app = router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
                handles.push(tokio::spawn(async move {
                    let _ = axum::serve(listener, app).await;
                }));
                match start_secure_port(state, ip, false).await {
                    Ok(h) => {
                        handles.push(h);
                        let _ = state.tailnet.set(ip);
                    }
                    Err(e) => tracing::warn!(error = %e, "couldn't start the encrypted port on Tailscale"),
                }
            }
            Err(e) => tracing::warn!(error = %e, %ip, "couldn't listen on the Tailscale address"),
        }
    }
    handles
}

async fn start_secure_port(state: &AppState, ip: IpAddr, announce: bool) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let bind = state.config.bind;
    let identity = tls::load_or_create(&state.config.data_dir)?;
    let config = tls::server_config(&identity)?;
    let addr = SocketAddr::new(ip, secure_port(bind.port()));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(addr.port());
    let _ = state.tls.set(tls::TlsInfo {
        port,
        fingerprint: identity.fingerprint.clone(),
    });
    tracing::info!(%addr, fingerprint = %identity.fingerprint, "encrypted port for phones listening");
    let app = router(state.clone());

    // Announce it; a failure here only means phones must type the address.
    let hw = routes::models::measure(state).await;
    let tier = format!("{:?}", hw.tier).to_lowercase();
    let daemon = routes::network::lan_ip().filter(|_| announce).and_then(|ip| {
        let name = routes::health::machine_name();
        discovery::advertise(&name, ip, port, bind.port(), &tier)
            .map_err(|e| tracing::warn!(error = %e, "couldn't announce Outpost on the network"))
            .ok()
    });
    // One task owns both, so aborting it closes the port and ends the announcement.
    Ok(tokio::spawn(async move {
        struct Announce(Option<mdns_sd::ServiceDaemon>);
        impl Drop for Announce {
            fn drop(&mut self) {
                if let Some(d) = self.0.take() {
                    let _ = d.shutdown();
                }
            }
        }
        let _announce = Announce(daemon);
        tls::serve(listener, config, app).await;
    }))
}

/// Is an Outpost gateway already answering at `bind`? A gateway listening on every
/// interface (0.0.0.0) is checked through 127.0.0.1.
pub async fn is_running(bind: SocketAddr) -> bool {
    let bind = local_address(bind);
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(800))
        .build()
    else {
        return false;
    };
    match client.get(format!("http://{bind}/api/health")).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// The address to reach a gateway from this PC: 0.0.0.0 / :: become loopback.
pub fn local_address(bind: SocketAddr) -> SocketAddr {
    match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), bind.port()),
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), bind.port())
        }
        _ => bind,
    }
}
