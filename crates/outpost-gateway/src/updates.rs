//! Checking for a newer Outpost on GitHub Releases: the only call Outpost makes to the
//! internet by itself, and it can be switched off (Settings › Updates).
//!
//! Each release carries `latest.json` (the desktop updater's manifest, plus the phone app):
//!
//! ```json
//! { "version": "0.2.0", "notes": "...", "pub_date": "2026-11-01T00:00:00Z",
//!   "platforms": { "windows-x86_64": { "url": "...setup.exe", "signature": "..." } },
//!   "mobile": { "version": "0.2.0", "url": "https://github.com/.../Outpost-0.2.0.apk" } }
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
}

/// The phone app in a release.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MobileRelease {
    pub version: String,
    /// Download link for the APK.
    pub url: String,
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
    error: Option<String>,
}

/// What the last check found.
#[derive(Clone, Default)]
pub struct Updates {
    found: Arc<RwLock<Found>>,
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
    UpdateStatus {
        enabled: enabled(state).await,
        repo: RELEASES_REPO.to_string(),
        releases_page: releases_page(),
        checked_at: found.checked_at,
        update: found.latest.filter(|r| newer(&r.version, &current)),
        mobile: found.mobile,
        error: found.error,
        current,
    }
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

/// Checks a minute after start, then once a day, while checks are on.
pub async fn background(state: AppState) {
    tokio::time::sleep(Duration::from_secs(60)).await;
    loop {
        if enabled(&state).await {
            check(&state).await;
            let s = status(&state).await;
            if let Some(u) = &s.update {
                tracing::info!(version = %u.version, "an Outpost update is ready");
            }
        }
        tokio::time::sleep(EVERY).await;
    }
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
