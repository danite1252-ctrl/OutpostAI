//! Signing in with an account PIN or password, with lockout after repeated failures.

use sqlx::SqlitePool;

use crate::accounts::{self, Account, Role};
use crate::secret::verify_secret;
use crate::sessions::{self, Session, SessionKind};
use crate::{audit, now, CoreError, CoreResult};

/// Wrong attempts allowed before the account locks.
pub const MAX_ATTEMPTS: i64 = 5;
/// First lock lasts this long; each further wrong attempt doubles it, up to `MAX_LOCK_SECS`.
pub const BASE_LOCK_SECS: i64 = 30;
pub const MAX_LOCK_SECS: i64 = 15 * 60;

pub struct SignIn {
    /// Bearer token for the new session. Shown once; only its hash is stored.
    pub token: String,
    pub session: Session,
    pub account: Account,
}

/// Check a PIN or password and, if it is right, start a session.
pub async fn sign_in(
    pool: &SqlitePool,
    account_id: &str,
    secret: &str,
    kind: SessionKind,
    label: &str,
) -> CoreResult<SignIn> {
    let account = check_secret(pool, account_id, secret, kind.as_str()).await?;
    let (token, session) = sessions::create(pool, account_id, kind, label).await?;
    audit::log_quiet(
        pool,
        Some(account_id),
        Some(&session.id),
        "sign_in",
        Some(kind.as_str()),
    )
    .await;
    Ok(SignIn {
        token,
        session,
        account,
    })
}

/// Check a PIN or password without starting a session (e.g. before changing it).
/// Wrong attempts count towards the lockout exactly like a sign-in.
pub async fn check_secret(pool: &SqlitePool, account_id: &str, secret: &str, context: &str) -> CoreResult<Account> {
    let row = accounts::get_row(pool, account_id).await?;
    if Role::parse(&row.role)? == Role::Service {
        return Err(CoreError::Forbidden("Service accounts sign in with API keys.".into()));
    }

    let t = now();
    if let Some(until) = row.locked_until {
        if until > t {
            return Err(CoreError::Locked { retry_after: until - t });
        }
    }

    let hash = row
        .secret_hash
        .clone()
        .ok_or_else(|| CoreError::Forbidden("This account has no PIN or password set.".into()))?;
    let attempt = secret.to_string();
    let ok = tokio::task::spawn_blocking(move || verify_secret(&attempt, &hash))
        .await
        .map_err(|e| CoreError::Internal(e.to_string()))?;

    if !ok {
        let attempts = row.failed_attempts + 1;
        let locked_until = if attempts >= MAX_ATTEMPTS {
            let doublings = (attempts - MAX_ATTEMPTS).min(10) as u32;
            Some(t + (BASE_LOCK_SECS * 2_i64.pow(doublings)).min(MAX_LOCK_SECS))
        } else {
            None
        };
        sqlx::query("UPDATE accounts SET failed_attempts = ?, locked_until = ? WHERE id = ?")
            .bind(attempts)
            .bind(locked_until)
            .bind(account_id)
            .execute(pool)
            .await?;
        audit::log_quiet(pool, Some(account_id), None, "secret_failed", Some(context)).await;
        return Err(match locked_until {
            Some(until) => CoreError::Locked { retry_after: until - t },
            None => CoreError::BadSecret {
                attempts_left: MAX_ATTEMPTS - attempts,
            },
        });
    }

    if row.failed_attempts != 0 || row.locked_until.is_some() {
        sqlx::query("UPDATE accounts SET failed_attempts = 0, locked_until = NULL WHERE id = ?")
            .bind(account_id)
            .execute(pool)
            .await?;
    }
    Account::try_from(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::NewAccount;
    use crate::db::open_in_memory;

    async fn setup() -> (SqlitePool, Account) {
        let pool = open_in_memory().await.unwrap();
        let dan = accounts::create(
            &pool,
            NewAccount {
                name: "Dan".into(),
                role: Role::Owner,
                secret: Some("4821".into()),
            },
        )
        .await
        .unwrap();
        (pool, dan)
    }

    #[tokio::test]
    async fn correct_pin_signs_in_and_token_works() {
        let (pool, dan) = setup().await;
        let s = sign_in(&pool, &dan.id, "4821", SessionKind::Desktop, "Desktop")
            .await
            .unwrap();
        let (session, account) = sessions::authenticate(&pool, &s.token).await.unwrap().unwrap();
        assert_eq!(account.id, dan.id);
        assert_eq!(session.kind, SessionKind::Desktop);

        sessions::revoke(&pool, &session.id).await.unwrap();
        assert!(sessions::authenticate(&pool, &s.token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn locks_after_five_wrong_pins() {
        let (pool, dan) = setup().await;
        for left in (1..MAX_ATTEMPTS).rev() {
            match sign_in(&pool, &dan.id, "0000", SessionKind::Desktop, "").await {
                Err(CoreError::BadSecret { attempts_left }) => assert_eq!(attempts_left, left),
                other => panic!("expected BadSecret, got {:?}", other.err()),
            }
        }
        match sign_in(&pool, &dan.id, "0000", SessionKind::Desktop, "").await {
            Err(CoreError::Locked { retry_after }) => assert_eq!(retry_after, BASE_LOCK_SECS),
            other => panic!("expected Locked, got {:?}", other.err()),
        }
        // Even the right PIN is refused while locked.
        assert!(matches!(
            sign_in(&pool, &dan.id, "4821", SessionKind::Desktop, "").await,
            Err(CoreError::Locked { .. })
        ));
    }

    #[tokio::test]
    async fn changing_pin_signs_out_everywhere() {
        let (pool, dan) = setup().await;
        let s = sign_in(&pool, &dan.id, "4821", SessionKind::Device, "Pixel")
            .await
            .unwrap();
        accounts::set_secret(&pool, &dan.id, "135790".into()).await.unwrap();
        assert!(sessions::authenticate(&pool, &s.token).await.unwrap().is_none());
        assert!(sign_in(&pool, &dan.id, "135790", SessionKind::Device, "Pixel")
            .await
            .is_ok());
    }
}
