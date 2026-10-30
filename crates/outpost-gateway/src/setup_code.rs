//! First-time setup on a server with no screen: until Outpost has an owner, it keeps a
//! one-time **setup code** that lets another device (the desktop app or a phone, over the
//! encrypted port) create the first owner account.
//!
//! - The code is shown only on the server itself: in its log and in
//!   `<data>/setup-code.txt` (readable by the server's user only), or with
//!   `outpost-gateway --setup-code`. Reading it needs access to the server.
//! - It lasts [`TTL_SECS`], works once, and [`MAX_WRONG`] wrong tries replace it.
//! - Once an owner exists there is no code at all; new people join with invites.
//!
//! On the PC Outpost runs on, setup still works without a code, as before.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::AppState;

pub const TTL_SECS: i64 = 30 * 60;
pub const MAX_WRONG: u32 = 5;

#[derive(Debug, Clone)]
struct Live {
    code: String,
    expires_at: i64,
    wrong: u32,
}

/// The current setup code, if Outpost has no owner yet.
#[derive(Clone, Default)]
pub struct SetupCodes {
    live: Arc<Mutex<Option<Live>>>,
}

pub fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("setup-code.txt")
}

/// `12345678` → `1234-5678`.
fn pretty(code: &str) -> String {
    format!("{}-{}", &code[..4], &code[4..])
}

/// Digits only; spaces and dashes people type are ignored.
pub fn normalise(code: &str) -> Option<String> {
    let digits: String = code.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    (digits.len() == 8 && digits.chars().all(|c| c.is_ascii_digit())).then_some(digits)
}

fn new_code() -> String {
    let bytes = outpost_core::secret::random_bytes::<4>().unwrap_or([7, 3, 1, 9]);
    format!("{:08}", u32::from_le_bytes(bytes) % 100_000_000)
}

/// Equal in constant time (no early exit to time).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl SetupCodes {
    /// Make a fresh code (and show it) when there is none or it ran out.
    pub async fn refresh(&self, state: &AppState) {
        let owner = outpost_core::accounts::owner_exists(&state.pool).await.unwrap_or(true);
        let mut live = self.live.lock().await;
        if owner {
            if live.take().is_some() || file(&state.config.data_dir).exists() {
                let _ = std::fs::remove_file(file(&state.config.data_dir));
            }
            return;
        }
        let now = outpost_core::now();
        if live.as_ref().is_some_and(|l| l.expires_at > now) {
            return;
        }
        let l = Live { code: new_code(), expires_at: now + TTL_SECS, wrong: 0 };
        announce(state, &l);
        *live = Some(l);
    }

    /// Check a code someone sent; a correct one is used up. Too many wrong tries replace it.
    pub async fn take(&self, state: &AppState, sent: &str) -> bool {
        let Some(sent) = normalise(sent) else { return false };
        let mut live = self.live.lock().await;
        let now = outpost_core::now();
        let Some(l) = live.as_mut().filter(|l| l.expires_at > now) else { return false };
        if same(&l.code, &sent) {
            *live = None;
            let _ = std::fs::remove_file(file(&state.config.data_dir));
            return true;
        }
        l.wrong += 1;
        if l.wrong >= MAX_WRONG {
            tracing::warn!("too many wrong setup codes: making a new one");
            let fresh = Live { code: new_code(), expires_at: now + TTL_SECS, wrong: 0 };
            announce(state, &fresh);
            *live = Some(fresh);
        }
        false
    }
}

/// Show the code where only someone with access to the server can see it.
fn announce(state: &AppState, l: &Live) {
    let words = state
        .tls
        .get()
        .map(|t| outpost_core::pairing::words(&t.fingerprint).join(" "))
        .unwrap_or_default();
    let until = crate::diagnostics::rfc3339(l.expires_at);
    tracing::info!(
        code = %pretty(&l.code),
        check_words = %words,
        valid_until = %until,
        "Outpost has no owner yet. To set it up from another device, choose \"Connect to an Outpost server\" and enter this setup code"
    );
    let text = format!(
        "Outpost setup code: {}\nValid until: {until} (UTC)\nCheck words: {}\n\nIn the Outpost desktop app or phone app, choose \"Connect to an Outpost server\",\ncheck the words match, and enter this code to create the owner account.\nIt works once; a new one appears here when it runs out.\n",
        pretty(&l.code),
        if words.is_empty() { "(shown once the encrypted port is on)" } else { &words },
    );
    let path = file(&state.config.data_dir);
    if std::fs::write(&path, text).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// Keeps a valid code while there is no owner (checks each minute).
pub async fn keep_fresh(state: AppState) {
    loop {
        state.setup.refresh(&state).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_eight_digits_typed_any_way() {
        assert_eq!(normalise("1234-5678").as_deref(), Some("12345678"));
        assert_eq!(normalise(" 1234 5678 ").as_deref(), Some("12345678"));
        assert!(normalise("1234-567").is_none());
        assert!(normalise("abcd-efgh").is_none());
        assert_eq!(pretty("12345678"), "1234-5678");
        assert_eq!(new_code().len(), 8);
        assert!(same("12345678", "12345678"));
        assert!(!same("12345678", "12345679"));
    }
}
