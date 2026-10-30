//! The desktop app using an Outpost server on another machine: the first look at the
//! server, and the pinned tunnel the window talks through. Runs a real encrypted port.

use std::net::SocketAddr;

use outpost_gateway::{remote, router, tls, AppState, Config};

fn temp_data_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "outpost-remote-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ))
}

async fn state(server: bool) -> AppState {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = temp_data_dir();
    config.ollama_url = "http://127.0.0.1:9".into();
    config.server = server;
    AppState::with_pool(pool, config)
}

/// An Outpost answering on an encrypted port on 127.0.0.1; returns its port and fingerprint.
async fn secure_server() -> (u16, String) {
    let state = state(false).await;
    let identity = tls::load_or_create(&state.config.data_dir).unwrap();
    let config = tls::server_config(&identity).unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(tls::serve(listener, config, router(state)));
    (port, identity.fingerprint)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

#[tokio::test]
async fn first_look_shows_the_certificate_and_whether_it_needs_an_owner() {
    let (port, fingerprint) = secure_server().await;
    let probe = remote::probe(&format!("127.0.0.1:{port}")).await.unwrap();
    assert_eq!(probe.fingerprint, fingerprint);
    assert_eq!(probe.words, remote::words(&fingerprint));
    assert_eq!(probe.words.len(), 3);
    assert_eq!(probe.health["setup_required"], true);
    assert_eq!(probe.port, port);

    // Nothing listening: a plain explanation, not a hang.
    let closed = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let dead = closed.local_addr().unwrap().port();
    drop(closed);
    let err = remote::probe(&format!("127.0.0.1:{dead}")).await.unwrap_err().to_string();
    assert!(err.contains("Couldn't reach"), "{err}");
}

#[tokio::test]
async fn the_tunnel_only_reaches_the_pinned_server() {
    let (port, fingerprint) = secure_server().await;

    let (addr, _task) = remote::start_tunnel("127.0.0.1".into(), port, fingerprint.clone()).await.unwrap();
    assert!(addr.ip().is_loopback());
    let resp = client().get(format!("http://{addr}/api/health")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let health: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(health["setup_required"], true);
    // Several requests on one kept-alive connection, and a request body, pass through.
    let resp = client()
        .post(format!("http://{addr}/api/setup/owner"))
        .json(&serde_json::json!({"name": "Dan", "secret": "4821"}))
        .send()
        .await
        .unwrap();
    // The server sees this PC (127.0.0.1 here), so it allows setup without a code.
    assert_eq!(resp.status(), 201);

    // A server showing a different certificate (someone in between, or a reinstall) is refused.
    let (wrong, _task2) = remote::start_tunnel("127.0.0.1".into(), port, "00".repeat(32)).await.unwrap();
    let resp = client().get(format!("http://{wrong}/api/health")).send().await.unwrap();
    assert_eq!(resp.status(), 502);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "server_unreachable");
}

#[tokio::test]
async fn a_server_has_network_access_on_until_the_owner_turns_it_off() {
    let mut desktop = state(false).await;
    desktop.apply_network_setting().await.unwrap();
    assert!(desktop.config.bind.ip().is_loopback());

    let mut server = state(true).await;
    server.apply_network_setting().await.unwrap();
    assert!(server.config.bind.ip().is_unspecified());

    outpost_core::settings::set_bool(&server.pool, outpost_core::settings::NETWORK_ACCESS, false)
        .await
        .unwrap();
    server.apply_network_setting().await.unwrap();
    assert_eq!(server.config.bind, SocketAddr::from(([127, 0, 0, 1], 7420)));
}

#[tokio::test]
async fn the_tunnel_falls_back_to_the_next_address_and_remembers() {
    let (port, fingerprint) = secure_server().await;
    // The first address goes nowhere (like the home network seen from away); the second works.
    let hosts: remote::Hosts = std::sync::Arc::new(std::sync::RwLock::new(vec!["10.255.255.1".into(), "127.0.0.1".into()]));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(remote::tunnel(listener, hosts.clone(), port, fingerprint));

    let started = std::time::Instant::now();
    let resp = client().get(format!("http://{addr}/api/health")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "fell back quickly");
    // The dead address is skipped for a while, so the next request is quick.
    let started = std::time::Instant::now();
    let resp = client().get(format!("http://{addr}/api/health")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(started.elapsed() < std::time::Duration::from_millis(1000), "{:?}", started.elapsed());
    // New addresses apply straight away.
    *hosts.write().unwrap() = vec!["127.0.0.1".into()];
    let resp = client().get(format!("http://{addr}/api/health")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
}
