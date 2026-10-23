//! Outpost gateway: the one HTTP API every client uses (desktop, phone, Hermes, scripts).
//!
//! Run it on its own with the `outpost-gateway` binary, or embed it with [`serve`]
//! (the desktop app does this when no gateway is already running).

pub mod config;
pub mod error;
pub mod extract;
pub mod llm;
pub mod openapi;
pub mod routes;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{header, HeaderValue, Method};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

pub use config::Config;
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
        AppState {
            llm,
            engines: EngineManager::new(&config.data_dir),
            models: ModelStore::new(),
            pool,
            config: Arc::new(config),
            http,
            setup_lock: Arc::new(Mutex::new(())),
        }
    }
}

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
    use routes::{accounts, auth, chat, engines, health, keys, models, openai, setup};

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            DESKTOP_ORIGINS.into_iter().map(HeaderValue::from_static),
        ))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::PATCH])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    Router::new()
        .route("/api/health", get(health::health))
        .route("/api/setup/owner", post(setup::create_owner))
        .route("/api/accounts/directory", get(auth::directory))
        .route("/api/auth/sign-in", post(auth::sign_in))
        .route("/api/auth/sign-out", post(auth::sign_out))
        .route("/api/me", get(auth::me))
        .route("/api/me/secret", put(auth::change_my_secret))
        .route("/api/accounts", get(accounts::list).post(accounts::create))
        .route("/api/accounts/{id}", delete(accounts::delete))
        .route("/api/accounts/{id}/secret", put(accounts::reset_secret))
        .route("/api/engines", get(engines::status))
        .route("/api/engines/{id}/install", post(engines::install))
        .route("/api/engines/{id}/start", post(engines::start))
        .route("/api/engines/{id}/stop", post(engines::stop))
        .route("/v1/models", get(models::list_models))
        .route("/v1/chat/completions", post(openai::chat_completions))
        .route("/api/personas", get(chat::list_personas).post(chat::create_persona))
        .route(
            "/api/personas/{id}",
            patch(chat::update_persona).delete(chat::delete_persona),
        )
        .route("/api/chats", get(chat::list_chats).post(chat::create_chat))
        .route(
            "/api/chats/{id}",
            get(chat::get_chat).patch(chat::update_chat).delete(chat::delete_chat),
        )
        .route("/api/chats/{id}/messages", post(chat::send_message))
        .route("/api/memory", get(chat::list_memory).post(chat::add_memory))
        .route("/api/memory/{id}", delete(chat::delete_memory))
        .route("/api/keys", get(keys::list).post(keys::create))
        .route("/api/keys/{id}", delete(keys::revoke))
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
    tracing::info!(%bind, "Outpost gateway listening");
    let starter = engines.clone();
    tokio::spawn(async move { starter.init().await });
    let result = axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await;
    engines.shutdown();
    result?;
    Ok(())
}

/// Is an Outpost gateway already answering at `bind`?
pub async fn is_running(bind: SocketAddr) -> bool {
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
