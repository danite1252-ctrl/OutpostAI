//! Checking for a newer Outpost on GitHub Releases: the only call Outpost makes to the
//! internet by itself, and it can be switched off (Settings › Updates).
//!
//! Each release carries `latest.json` (the desktop updater's manifest, plus the phone app):
//!
//! ```json
//! { "version": "0.2.0", "notes": "...", "pub_date": "2026-11-01T00:00:00Z",
//!   "platforms": { "windows-x86_64": { "url": "...setup.exe", "signature": "..." } },
//!   "mobile": { "version": "0.2.0", "url": "https://github.com/.../Outpost-0.2.0.apk", "sha256": "..." } }
//! ```
//!
//! Outpost only reads it and says an update is ready; installing is the desktop app's job,
//! after you press Update, and the installer's signature is checked against the public key
//! built into the app.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;

/// Where releases are published: `owner/repo` on GitHub.
pub const RELEASES_REPO: &str = "danite1252-ctrl/OutpostAI";

/// Setting: "1" when the user switched update checks off.
pub const UPDATES_OFF: &str = "updates.off";

/// How often to look (and once, a minute after start).
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// A server (left running on its own) checks this often, and installs by itself.
const SERVER_EVERY: Duration = Duration::from_secs(60);
/// "off" to stop a server installing updates by itself (on until turned off).
pub const AUTO_INSTALL: &str = "updates.auto_install";
/// After a failed automatic install, wait this long before trying that version again.
const RETRY_AFTER: i64 = 60 * 60;

pub fn repo_configured() -> bool {
    !RELEASES_REPO.starts_with("OWNER/")
}

pub fn manifest_url() -> String {
    format!("https://github.com/{RELEASES_REPO}/releases/latest/download/latest.json")
}

pub fn releases_page() -> String {
    format!("https://github.com/{RELEASES_REPO}/releases")
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    pub_date: Option<String>,
    #[serde(default)]
    mobile: Option<MobileRelease>,
    #[serde(default)]
    platforms: std::collections::HashMap<String, Platform>,
}

/// One platform's download in a release, signed with the update key.
#[derive(Debug, Clone, Deserialize)]
pub struct Platform {
    pub url: String,
    /// The download's signature (minisign, base64), as the desktop updater uses.
    pub signature: String,
}

/// The release key's public half: the same one the desktop app checks its updates with.
fn update_public_key() -> Option<String> {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../../apps/desktop/src-tauri/tauri.conf.json")).ok()?;
    conf["plugins"]["updater"]["pubkey"].as_str().map(str::to_string)
}

/// Is `signature` (base64 minisign, from latest.json) a good signature of `data` by the
/// release key?
pub fn signature_ok(data: &[u8], signature: &str) -> bool {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let decode = |s: &str| b64.decode(s.trim()).ok().and_then(|b| String::from_utf8(b).ok());
    let (Some(key), Some(sig)) = (update_public_key().as_deref().and_then(decode), decode(signature)) else {
        return false;
    };
    let (Ok(key), Ok(sig)) = (
        minisign_verify::PublicKey::decode(&key),
        minisign_verify::Signature::decode(&sig),
    ) else {
        return false;
    };
    key.verify(data, &sig, true).is_ok()
}

/// This server's platform in `latest.json`.
pub fn server_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

/// The phone app in a release.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MobileRelease {
    pub version: String,
    /// Download link for the APK.
    pub url: String,
    /// SHA-256 of the APK (lowercase hex), checked by the phone after downloading.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// A newer release.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Release {
    pub version: String,
    /// What's new (Markdown).
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Found {
    checked_at: Option<i64>,
    latest: Option<Release>,
    mobile: Option<MobileRelease>,
    /// This server's download in the newest release.
    server: Option<Platform>,
    error: Option<String>,
}

/// What the last check found.
#[derive(Clone, Default)]
pub struct Updates {
    found: Arc<RwLock<Found>>,
    /// A server install in progress (or its failure).
    installing: Arc<RwLock<Option<String>>>,
}

/// Update status for the apps.
#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateStatus {
    /// This Outpost's version.
    pub current: String,
    /// Checks are on.
    pub enabled: bool,
    /// Releases come from `github.com/<repo>`; checks stay off until it's set.
    pub repo: String,
    pub releases_page: String,
    /// Unix seconds of the last check.
    pub checked_at: Option<i64>,
    /// The newest release, when it's newer than this one.
    pub update: Option<Release>,
    /// The newest phone app (the phone compares it with its own version).
    pub mobile: Option<MobileRelease>,
    /// This is a server (`--server`) that can install `update` itself (`POST /api/updates/install`).
    pub server_can_install: bool,
    /// A server install in progress, or why the last one failed.
    pub installing: Option<String>,
    /// A server installs updates by itself, at a quiet moment (checking every minute).
    pub auto_install: bool,
    /// Why the last check failed.
    pub error: Option<String>,
}

pub async fn enabled(state: &AppState) -> bool {
    repo_configured()
        && !outpost_core::settings::get_bool(&state.pool, UPDATES_OFF)
            .await
            .unwrap_or(false)
}

pub async fn status(state: &AppState) -> UpdateStatus {
    let found = state.updates.found.read().map(|f| f.clone()).unwrap_or_default();
    let current = env!("CARGO_PKG_VERSION").to_string();
    let update = found.latest.filter(|r| newer(&r.version, &current));
    UpdateStatus {
        enabled: enabled(state).await,
        repo: RELEASES_REPO.to_string(),
        releases_page: releases_page(),
        checked_at: found.checked_at,
        server_can_install: state.config.server && update.is_some() && found.server.is_some(),
        installing: state.updates.installing.read().ok().and_then(|i| i.clone()),
        auto_install: auto_install(state).await,
        update,
        mobile: found.mobile,
        error: found.error,
        current,
    }
}

/// Install the newest release on this server: download it, check its signature against the
/// release key built into Outpost, put it in place of this program, and restart (the
/// service manager starts the new one). Runs in the background; see `installing`.
pub fn install_server(state: &AppState) -> Result<(), String> {
    if !state.config.server {
        return Err("Only a server (started with --server) updates itself. On a PC, the desktop app installs updates.".into());
    }
    let found = state.updates.found.read().map(|f| f.clone()).unwrap_or_default();
    let (Some(latest), Some(platform)) = (found.latest, found.server) else {
        return Err("There's no server update to install. Check for updates first.".into());
    };
    if !newer(&latest.version, env!("CARGO_PKG_VERSION")) {
        return Err("This server is already up to date.".into());
    }
    {
        let mut busy = state.updates.installing.write().map_err(|_| "busy")?;
        if busy.as_deref().is_some_and(|b| !b.starts_with("Failed")) {
            return Err("An update is already being installed.".into());
        }
        *busy = Some(format!("Downloading Outpost {}", latest.version));
    }
    let state = state.clone();
    tokio::spawn(async move {
        match download_and_replace(&state, &platform, &latest.version).await {
            Ok(()) => {
                tracing::info!(version = %latest.version, "installed the update; restarting");
                restart(&state).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "server update failed");
                if let Ok(mut i) = state.updates.installing.write() {
                    *i = Some(format!("Failed: {e}"));
                }
            }
        }
    });
    Ok(())
}

async fn download_and_replace(state: &AppState, platform: &Platform, version: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .user_agent(concat!("Outpost/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&platform.url).send().await.map_err(|_| "Couldn't download the update.".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("The download failed ({}).", resp.status().as_u16()));
    }
    let data = resp.bytes().await.map_err(|_| "The download was cut off.".to_string())?;
    if !signature_ok(&data, &platform.signature) {
        return Err("The download's signature doesn't match Outpost's release key, so it wasn't installed.".into());
    }
    if let Ok(mut i) = state.updates.installing.write() {
        *i = Some(format!("Installing Outpost {version}"));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_file_name(".outpost-gateway.new");
    tokio::fs::write(&new, &data).await.map_err(|e| format!("Couldn't save the update: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
    }
    // On Linux a running program can be replaced; the old one keeps running until it exits.
    tokio::fs::rename(&new, &exe).await.map_err(|e| format!("Couldn't put the update in place: {e}"))?;
    Ok(())
}

/// Exit code that asks the service manager to start Outpost again (`Restart=on-failure`).
pub const RESTART_EXIT_CODE: i32 = 75;

/// Stop the engines, note a normal close, and exit so systemd starts Outpost again.
pub async fn restart(state: &AppState) {
    tokio::time::sleep(Duration::from_millis(800)).await;
    state.engines.shutdown();
    crate::diagnostics::clean_exit();
    std::process::exit(RESTART_EXIT_CODE);
}

/// Look now (whether or not checks are on: this is the "Check now" button).
pub async fn check(state: &AppState) {
    let result = fetch().await;
    if let Ok(mut f) = state.updates.found.write() {
        f.checked_at = Some(outpost_core::now());
        match result {
            Ok(m) => {
                f.latest = Some(Release { version: m.version.trim_start_matches('v').to_string(), notes: m.notes, date: m.pub_date });
                f.mobile = m.mobile;
                f.server = server_platform().and_then(|p| m.platforms.get(p).cloned());
                f.error = None;
            }
            Err(e) => f.error = Some(e),
        }
    }
}

async fn fetch() -> Result<Manifest, String> {
    if !repo_configured() {
        return Err("No releases address is set yet.".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("Outpost/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(manifest_url())
        .send()
        .await
        .map_err(|_| "Couldn't reach GitHub to check for updates.".to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("No release has been published yet.".into());
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub said no ({}).", resp.status().as_u16()));
    }
    resp.json::<Manifest>()
        .await
        .map_err(|_| "The release information couldn't be read.".to_string())
}

/// A server installs updates by itself (until the owner turns it off).
pub async fn auto_install(state: &AppState) -> bool {
    state.config.server
        && outpost_core::settings::get(&state.pool, AUTO_INSTALL).await.ok().flatten().as_deref() != Some("off")
}

/// Checks a minute after start, then once a day (a server: every minute, installing what it
/// finds at a quiet moment), while checks are on. The apps' Check now looks at once.
pub async fn background(state: AppState) {
    tokio::time::sleep(Duration::from_secs(60)).await;
    let mut announced: Option<String> = None;
    // The version an automatic install last failed for, and when.
    let mut failed: Option<(String, i64)> = None;
    loop {
        if enabled(&state).await {
            check(&state).await;
            let s = status(&state).await;
            if let Some(u) = &s.update {
                if announced.as_deref() != Some(u.version.as_str()) {
                    tracing::info!(version = %u.version, "an Outpost update is ready");
                    announced = Some(u.version.clone());
                }
                let retry_ok = match &failed {
                    Some((v, at)) if *v == u.version => outpost_core::now() - at > RETRY_AFTER,
                    _ => true,
                };
                let busy = s.installing.as_deref().is_some_and(|i| !i.starts_with("Failed"));
                if s.server_can_install && s.auto_install && !busy && retry_ok && quiet(&state).await {
                    tracing::info!(version = %u.version, "installing the update by itself (nothing is running)");
                    if let Err(e) = install_server(&state) {
                        tracing::warn!(error = %e, "couldn't start the automatic update");
                    }
                    // The install restarts Outpost when it's done; if it fails, wait before trying again.
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    let now = status(&state).await;
                    if now.installing.as_deref().is_some_and(|i| i.starts_with("Failed")) {
                        failed = Some((u.version.clone(), outpost_core::now()));
                    }
                }
            }
        }
        tokio::time::sleep(if state.config.server { SERVER_EVERY } else { EVERY }).await;
    }
}

/// Nothing a restart would interrupt: no job running (here or on a helper PC), no
/// training, and no chat in the last couple of minutes.
async fn quiet(state: &AppState) -> bool {
    state.queue.running().is_none()
        && state.queue.remote_runs().is_empty()
        && !crate::gallery_index::chat_recently(120)
}

/// `a` is a newer version than `b` (`0.10.0` > `0.9.2`; `1.0.0` > `1.0.0-beta.1`).
pub fn newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre)
    }
    let (mut x, xpre) = parts(a);
    let (mut y, ypre) = parts(b);
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    match x.cmp(&y) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        // Same numbers: a release beats a pre-release.
        std::cmp::Ordering::Equal => ypre && !xpre,
    }
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn unsigned_or_tampered_downloads_are_refused() {
        // The release key's public half is built in.
        assert!(super::update_public_key().is_some_and(|k| k.len() > 40));
        assert!(!super::signature_ok(b"anything", ""));
        assert!(!super::signature_ok(b"anything", "bm90IGEgc2lnbmF0dXJl"));
        assert_eq!(super::server_platform().is_some(), cfg!(all(target_os = "linux", target_arch = "x86_64")));
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("v1.0.0", "0.9.0"));
        assert!(newer("1.0.0", "1.0.0-beta.1"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(!newer("1.0.0-beta.2", "1.0.0"));
    }
}
