//! Pairing a phone: someone at the Outpost PC asks for a one-time 6-digit code; the
//! phone sends it back with an account and that account's PIN, and gets a device
//! session that stays signed in.
//!
//! Only the code's hash is stored. A code works once, lasts [`CODE_TTL_SECS`], and
//! wrong guesses count against every live code, so after [`MAX_WRONG_GUESSES`] they
//! are all dead and someone at the PC must make a new one.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::auth::{self, SignIn};
use crate::secret::random_bytes;
use crate::sessions::{self, SessionKind};
use crate::{audit, now, CoreError, CoreResult};

pub const CODE_TTL_SECS: i64 = 10 * 60;

/// Code checks run one at a time, so the wrong-guess limit can't be raced.
fn checks() -> &'static tokio::sync::Mutex<()> {
    static CHECKS: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    CHECKS.get_or_init(|| tokio::sync::Mutex::new(()))
}
pub const MAX_WRONG_GUESSES: i64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct PairingCode {
    pub id: String,
    pub created_by: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    /// The device session it made, once used.
    pub session_id: Option<String>,
    pub wrong_guesses: i64,
    /// For an invite: `join` (a new account) or `reset` (a new PIN for an existing one).
    pub invite_kind: Option<String>,
    /// The account it resets, or (once used) the account it made.
    pub invite_account: Option<String>,
    /// For a join: the new account's name and role.
    pub invite_name: Option<String>,
    pub invite_role: Option<String>,
}

const COLS: &str =
    "id, created_by, created_at, expires_at, used_at, session_id, wrong_guesses, invite_kind, invite_account, invite_name, invite_role";

/// How long an invite works: long enough to send to someone.
pub const INVITE_TTL_SECS: i64 = 24 * 60 * 60;

/// Who an invite is for.
#[derive(Debug, Clone)]
pub enum Invite {
    /// A new account, made when the person picks their PIN.
    Join { name: String, role: crate::accounts::Role },
    /// A new PIN for an existing account (it signs that account out everywhere else).
    Reset { account_id: String },
}

fn hash(code: &str) -> String {
    let digest = Sha256::digest(format!("outpost-pairing:{code}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Codes are exactly six digits; spaces and dashes people type are ignored.
pub fn normalise(code: &str) -> Option<String> {
    let digits: String = code.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_digit())).then_some(digits)
}

/// A new code for `account_id` (who is at the PC). Their older unused codes stop working.
pub async fn create(pool: &SqlitePool, account_id: &str) -> CoreResult<(String, PairingCode)> {
    let t = now();
    sqlx::query("UPDATE pairing_codes SET expires_at = ? WHERE created_by = ? AND used_at IS NULL AND expires_at > ? AND invite_kind IS NULL")
        .bind(t)
        .bind(account_id)
        .bind(t)
        .execute(pool)
        .await?;
    let n = u32::from_le_bytes(random_bytes::<4>()?) % 1_000_000;
    let code = format!("{n:06}");
    let row = PairingCode {
        id: uuid::Uuid::new_v4().to_string(),
        created_by: account_id.into(),
        created_at: t,
        expires_at: t + CODE_TTL_SECS,
        used_at: None,
        session_id: None,
        wrong_guesses: 0,
        invite_kind: None,
        invite_account: None,
        invite_name: None,
        invite_role: None,
    };
    sqlx::query(
        "INSERT INTO pairing_codes (id, code_hash, created_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(hash(&code))
    .bind(account_id)
    .bind(row.created_at)
    .bind(row.expires_at)
    .execute(pool)
    .await?;
    audit::log_quiet(pool, Some(account_id), None, "pairing_code", None).await;
    Ok((code, row))
}

/// A new invite, made by `created_by` (an owner, or the account itself for a reset made
/// on the server). It replaces earlier unused invites for the same person.
pub async fn create_invite(pool: &SqlitePool, created_by: &str, invite: Invite) -> CoreResult<(String, PairingCode)> {
    let t = now();
    let (kind, account, name, role) = match &invite {
        Invite::Join { name, role } => {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 40 {
                return Err(CoreError::Invalid("Give them a name of up to 40 characters.".into()));
            }
            if *role == crate::accounts::Role::Service {
                return Err(CoreError::Invalid("Service accounts use app keys, not invites.".into()));
            }
            let taken = crate::accounts::list(pool)
                .await?
                .iter()
                .any(|a| a.name.eq_ignore_ascii_case(name));
            if taken {
                return Err(CoreError::Conflict(format!("There's already an account called {name}.")));
            }
            ("join", None, Some(name.to_string()), Some(role.as_str().to_string()))
        }
        Invite::Reset { account_id } => {
            let a = crate::accounts::get(pool, account_id).await?;
            if a.role == crate::accounts::Role::Service {
                return Err(CoreError::Invalid("Service accounts use app keys, not a PIN.".into()));
            }
            ("reset", Some(a.id), Some(a.name), None)
        }
    };
    // Only the newest invite for a person works.
    sqlx::query(
        "UPDATE pairing_codes SET expires_at = ? WHERE used_at IS NULL AND expires_at > ? AND invite_kind = ?
         AND (invite_account = ? OR (invite_account IS NULL AND invite_name = ?))",
    )
    .bind(t)
    .bind(t)
    .bind(kind)
    .bind(&account)
    .bind(&name)
    .execute(pool)
    .await?;
    let n = u32::from_le_bytes(random_bytes::<4>()?) % 1_000_000;
    let code = format!("{n:06}");
    let row = PairingCode {
        id: uuid::Uuid::new_v4().to_string(),
        created_by: created_by.into(),
        created_at: t,
        expires_at: t + INVITE_TTL_SECS,
        used_at: None,
        session_id: None,
        wrong_guesses: 0,
        invite_kind: Some(kind.into()),
        invite_account: account,
        invite_name: name,
        invite_role: role,
    };
    sqlx::query(
        "INSERT INTO pairing_codes (id, code_hash, created_by, created_at, expires_at, invite_kind, invite_account, invite_name, invite_role)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(hash(&code))
    .bind(created_by)
    .bind(row.created_at)
    .bind(row.expires_at)
    .bind(&row.invite_kind)
    .bind(&row.invite_account)
    .bind(&row.invite_name)
    .bind(&row.invite_role)
    .execute(pool)
    .await?;
    audit::log_quiet(pool, Some(created_by), None, "invite", Some(&format!("{kind} {}", row.invite_name.as_deref().unwrap_or("")))).await;
    Ok((code, row))
}

/// Use an invite: the person's new PIN makes their account (join) or replaces the old
/// one (reset), and this device is paired to it as `device`.
pub async fn accept_invite(pool: &SqlitePool, code: &str, new_secret: &str, device: &str) -> CoreResult<SignIn> {
    let _one_at_a_time = checks().lock().await;
    let invite = check_locked(pool, code).await?;
    crate::secret::validate_secret(new_secret)?;
    let account_id = match invite.invite_kind.as_deref() {
        Some("join") => {
            let role = crate::accounts::Role::parse(invite.invite_role.as_deref().unwrap_or("member"))?;
            let a = crate::accounts::create(
                pool,
                crate::accounts::NewAccount {
                    name: invite.invite_name.clone().unwrap_or_default(),
                    role,
                    secret: Some(new_secret.to_string()),
                },
            )
            .await?;
            a.id
        }
        Some("reset") => {
            let id = invite.invite_account.clone().ok_or(CoreError::NotFound)?;
            crate::accounts::set_secret(pool, &id, new_secret.to_string()).await?;
            id
        }
        _ => {
            return Err(CoreError::Invalid(
                "That's a pairing code, not an invite: pick your account and enter your PIN.".into(),
            ))
        }
    };
    let label = if device.trim().is_empty() { "Device" } else { device };
    let signed_in = auth::sign_in(pool, &account_id, new_secret, SessionKind::Device, label).await?;
    sqlx::query("UPDATE pairing_codes SET used_at = ?, session_id = ?, invite_account = ? WHERE id = ?")
        .bind(now())
        .bind(&signed_in.session.id)
        .bind(&account_id)
        .bind(&invite.id)
        .execute(pool)
        .await?;
    audit::log_quiet(
        pool,
        Some(&account_id),
        Some(&signed_in.session.id),
        if invite.invite_kind.as_deref() == Some("join") { "invite_joined" } else { "pin_reset_by_invite" },
        Some(label),
    )
    .await;
    Ok(signed_in)
}

/// One code by id (for the PC to see whether its code was used).
pub async fn get(pool: &SqlitePool, id: &str) -> CoreResult<PairingCode> {
    sqlx::query_as(&format!("SELECT {COLS} FROM pairing_codes WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

fn live_clause(t: i64) -> String {
    format!("used_at IS NULL AND expires_at > {t} AND wrong_guesses < {MAX_WRONG_GUESSES}")
}

/// Is this a live code? A wrong one counts against every live code.
pub async fn check(pool: &SqlitePool, code: &str) -> CoreResult<PairingCode> {
    let _one_at_a_time = checks().lock().await;
    check_locked(pool, code).await
}

async fn check_locked(pool: &SqlitePool, code: &str) -> CoreResult<PairingCode> {
    let wrong = || CoreError::Invalid("That code isn't right, or it has expired. Make a new one on the Outpost PC.".into());
    let t = now();
    let live = live_clause(t);
    let Some(code) = normalise(code) else {
        return Err(wrong());
    };
    let found: Option<PairingCode> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM pairing_codes WHERE code_hash = ? AND {live}"
    ))
    .bind(hash(&code))
    .fetch_optional(pool)
    .await?;
    match found {
        Some(c) => Ok(c),
        None => {
            sqlx::query(&format!("UPDATE pairing_codes SET wrong_guesses = wrong_guesses + 1 WHERE {live}"))
                .execute(pool)
                .await?;
            audit::log_quiet(pool, None, None, "pairing_wrong_code", None).await;
            Err(wrong())
        }
    }
}

/// Use a code: check the account's PIN and start a device session named `device`.
/// A wrong PIN leaves the code alive (the account's own lockout applies).
pub async fn complete(
    pool: &SqlitePool,
    code: &str,
    account_id: &str,
    secret: &str,
    device: &str,
) -> CoreResult<SignIn> {
    // One at a time from the code check to claiming it, so a code can't be used twice
    // or after it died.
    let _one_at_a_time = checks().lock().await;
    let pairing = check_locked(pool, code).await?;
    if pairing.invite_kind.is_some() {
        return Err(CoreError::Invalid("That's an invite: choose your PIN instead.".into()));
    }
    let label = if device.trim().is_empty() { "Phone" } else { device };
    let signed_in = match auth::sign_in(pool, account_id, secret, SessionKind::Device, label).await {
        Ok(s) => s,
        Err(e) => {
            // A wrong PIN also counts against the code.
            if matches!(e, CoreError::BadSecret { .. } | CoreError::Locked { .. }) {
                sqlx::query("UPDATE pairing_codes SET wrong_guesses = wrong_guesses + 1 WHERE id = ?")
                    .bind(&pairing.id)
                    .execute(pool)
                    .await?;
            }
            return Err(e);
        }
    };
    let used = sqlx::query(&format!(
        "UPDATE pairing_codes SET used_at = ?, session_id = ? WHERE id = ? AND {}",
        live_clause(now())
    ))
    .bind(now())
    .bind(&signed_in.session.id)
    .bind(&pairing.id)
    .execute(pool)
    .await?;
    if used.rows_affected() == 0 {
        // The code died while the PIN was checked (expired or replaced): no session.
        sessions::revoke(pool, &signed_in.session.id).await?;
        return Err(CoreError::Invalid("That code has just expired. Make a new one on the Outpost PC.".into()));
    }
    audit::log_quiet(pool, Some(account_id), Some(&signed_in.session.id), "paired_device", Some(label)).await;
    Ok(signed_in)
}

/// Three short words from a certificate fingerprint, shown on the PC and the phone
/// so people can see they're talking to the same Outpost.
pub fn words(fingerprint_hex: &str) -> [&'static str; 3] {
    let digest = Sha256::digest(format!("outpost-words:{}", fingerprint_hex.to_ascii_lowercase()).as_bytes());
    [WORDS[digest[0] as usize], WORDS[digest[1] as usize], WORDS[digest[2] as usize]]
}

/// 256 short, distinct, easy-to-say words. The phone app has the same list.
pub const WORDS: [&str; 256] = [
    "acorn", "adobe", "agent", "album", "alley", "amber", "anchor", "angle", "apple", "apron", "arrow", "aspen",
    "atlas", "attic", "autumn", "badge", "bagel", "baker", "bamboo", "banjo", "barn", "basil", "beach", "beacon",
    "bean", "bear", "bell", "berry", "bike", "birch", "bison", "blade", "blanket", "bloom", "boat", "bonus",
    "boot", "bottle", "brick", "bridge", "brook", "brush", "bucket", "buffalo", "bugle", "cabin", "cactus", "camel",
    "candle", "canoe", "canyon", "carpet", "castle", "cedar", "cello", "chalk", "cherry", "chess", "cider", "cinema",
    "circle", "clock", "cloud", "clover", "coast", "cobalt", "comet", "copper", "coral", "cotton", "cowboy", "crane",
    "crayon", "cricket", "crown", "cubby", "daisy", "delta", "denim", "desert", "diamond", "dingo", "dolphin", "domino",
    "donkey", "dragon", "drum", "dune", "eagle", "easel", "echo", "elbow", "ember", "emerald", "engine", "falcon",
    "feather", "fern", "fiddle", "fig", "flame", "flute", "forest", "fossil", "fox", "frost", "galaxy", "garden",
    "garlic", "gecko", "ginger", "glacier", "globe", "goose", "granite", "grape", "gravel", "guitar", "hammer", "harbor",
    "harp", "hazel", "helmet", "heron", "hickory", "honey", "horizon", "husky", "igloo", "island", "ivory", "jacket",
    "jaguar", "jasmine", "jelly", "jewel", "jigsaw", "jungle", "kayak", "kettle", "kiwi", "koala", "ladder", "lagoon",
    "lantern", "laser", "lemon", "lily", "lime", "lizard", "lobster", "locket", "lotus", "magnet", "mango", "maple",
    "marble", "meadow", "melon", "meteor", "mint", "mirror", "mitten", "moose", "mosaic", "muffin", "nectar", "needle",
    "nickel", "noodle", "nutmeg", "oasis", "ocean", "olive", "onion", "opal", "orbit", "orchid", "otter", "owl",
    "paddle", "panda", "parrot", "peach", "pebble", "pencil", "pepper", "piano", "pickle", "pigeon", "pillow", "pine",
    "planet", "plum", "pocket", "pony", "poppy", "prism", "puffin", "pumpkin", "quartz", "quilt", "rabbit", "radar",
    "raven", "reef", "ribbon", "river", "robin", "rocket", "rose", "ruby", "saddle", "salmon", "sandal", "satin",
    "seal", "shell", "silver", "sketch", "sled", "snail", "sparrow", "spider", "spoon", "spruce", "squid", "stone",
    "storm", "sugar", "summit", "sunset", "swan", "tango", "teapot", "thistle", "thunder", "tiger", "timber", "toast",
    "tomato", "topaz", "torch", "tulip", "tundra", "turtle", "umbrella", "valley", "velvet", "violet", "walnut", "walrus",
    "willow", "window", "wizard", "yak",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;

    async fn account(pool: &SqlitePool, name: &str, role: Role) -> String {
        accounts::create(
            pool,
            NewAccount {
                name: name.into(),
                role,
                secret: Some("4821".into()),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn a_code_pairs_one_phone_once() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan", Role::Owner).await;
        let eve = account(&pool, "Eve", Role::Member).await;
        let (code, row) = create(&pool, &dan).await.unwrap();
        assert_eq!(code.len(), 6);
        assert!(check(&pool, &code).await.is_ok());

        // Wrong PIN: refused, code still alive.
        assert!(matches!(
            complete(&pool, &code, &eve, "0000", "Eve's Pixel").await,
            Err(CoreError::BadSecret { .. })
        ));
        let paired = complete(&pool, &format!("{} {}", &code[..3], &code[3..]), &eve, "4821", "Eve's Pixel")
            .await
            .unwrap();
        assert_eq!(paired.session.kind, SessionKind::Device);
        assert_eq!(paired.session.label, "Eve's Pixel");
        assert_eq!(paired.account.id, eve);
        let used = get(&pool, &row.id).await.unwrap();
        assert_eq!(used.session_id.as_deref(), Some(paired.session.id.as_str()));

        // Used: never again.
        assert!(complete(&pool, &code, &eve, "4821", "Again").await.is_err());
    }

    #[tokio::test]
    async fn wrong_guesses_kill_live_codes_and_new_codes_replace_old() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan", Role::Owner).await;
        let (old, _) = create(&pool, &dan).await.unwrap();
        let (code, _) = create(&pool, &dan).await.unwrap();
        if old != code {
            assert!(check(&pool, &old).await.is_err(), "a new code replaces the old one");
        }
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..MAX_WRONG_GUESSES {
            assert!(check(&pool, wrong).await.is_err());
        }
        assert!(check(&pool, &code).await.is_err(), "too many wrong guesses");
        assert!(check(&pool, "12345").await.is_err());
        assert!(check(&pool, "abcdef").await.is_err());
    }

    #[tokio::test]
    async fn invites_let_people_pick_their_own_pin() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan", Role::Owner).await;

        // Join: Sam's account appears when Sam picks a PIN.
        let (code, _) = create_invite(&pool, &dan, Invite::Join { name: "Sam".into(), role: Role::Member }).await.unwrap();
        let normal = create(&pool, &dan).await.unwrap().0;
        // A new pairing code doesn't kill the invite, and each is used its own way.
        assert!(complete(&pool, &code, &dan, "4821", "x").await.is_err());
        assert!(accept_invite(&pool, &normal, "2468", "x").await.is_err());
        assert!(accept_invite(&pool, &code, "12", "Sam's phone").await.is_err(), "PIN too short");
        let sam = accept_invite(&pool, &code, "2468", "Sam's phone").await.unwrap();
        assert_eq!(sam.account.name, "Sam");
        assert_eq!(sam.account.role, Role::Member);
        assert_eq!(sam.session.kind, SessionKind::Device);
        assert!(accept_invite(&pool, &code, "2468", "again").await.is_err(), "used once");
        // Names stay unique.
        assert!(create_invite(&pool, &dan, Invite::Join { name: "sam".into(), role: Role::Member }).await.is_err());

        // Reset: Dan forgot his PIN.
        let (old, _) = create_invite(&pool, &dan, Invite::Reset { account_id: dan.clone() }).await.unwrap();
        let (reset, row) = create_invite(&pool, &dan, Invite::Reset { account_id: dan.clone() }).await.unwrap();
        if old != reset {
            assert!(check(&pool, &old).await.is_err(), "the newest invite replaces the old");
        }
        assert_eq!(row.expires_at - row.created_at, INVITE_TTL_SECS);
        let back = accept_invite(&pool, &reset, "13579", "Dan's desktop").await.unwrap();
        assert_eq!(back.account.id, dan);
        assert!(auth::sign_in(&pool, &dan, "4821", SessionKind::Desktop, "old").await.is_err());
        assert!(auth::sign_in(&pool, &dan, "13579", SessionKind::Desktop, "new").await.is_ok());
    }

    #[test]
    fn words_are_stable_and_distinct() {
        let mut sorted = WORDS.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 256);
        let fp = "ab".repeat(32);
        assert_eq!(words(&fp), words(&fp.to_uppercase()));
        assert_ne!(words(&fp), words(&"cd".repeat(32)));
        assert_eq!(normalise(" 123-456 "), Some("123456".into()));
    }
}
