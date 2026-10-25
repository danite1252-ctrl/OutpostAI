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

    let hash = row
        .secret_hash
        .clone()
        .ok_or_else(|| CoreError::Forbidden("This account has no PIN or password set.".into()))?;

    // Count the attempt before checking it, in one statement, so a burst of guesses
    // sent at once can't all slip past the lockout: the attempt that reaches the limit
    // locks the account there and then, and later ones are refused.
    let t = now();
    let reserved: Option<i64> = sqlx::query_scalar(
        "UPDATE accounts SET failed_attempts = failed_attempts + 1,
             locked_until = CASE WHEN failed_attempts + 1 >= ?1
                 THEN ?2 + MIN(?3, ?4 * (1 << MIN(failed_attempts + 1 - ?1, 10)))
                 ELSE locked_until END
         WHERE id = ?5 AND (locked_until IS NULL OR locked_until <= ?2)
         RETURNING failed_attempts",
    )
    .bind(MAX_ATTEMPTS)
    .bind(t)
    .bind(MAX_LOCK_SECS)
    .bind(BASE_LOCK_SECS)
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    let Some(attempts) = reserved else {
        return Err(CoreError::Locked {
            retry_after: locked_for(pool, account_id, t).await?,
        });
    };

    let attempt = secret.to_string();
    let ok = tokio::task::spawn_blocking(move || verify_secret(&attempt, &hash))
        .await
        .map_err(|e| CoreError::Internal(e.to_string()))?;

    if !ok {
        audit::log_quiet(pool, Some(account_id), None, "secret_failed", Some(context)).await;
        return Err(if attempts >= MAX_ATTEMPTS {
            CoreError::Locked {
                retry_after: locked_for(pool, account_id, t).await?,
            }
        } else {
            CoreError::BadSecret {
                attempts_left: MAX_ATTEMPTS - attempts,
            }
        });
    }

    sqlx::query("UPDATE accounts SET failed_attempts = 0, locked_until = NULL WHERE id = ?")
        .bind(account_id)
        .execute(pool)
        .await?;
    Account::try_from(row)
}

/// Seconds left on an account's lock (at least 1 while locked).
async fn locked_for(pool: &SqlitePool, account_id: &str, t: i64) -> CoreResult<i64> {
    let until: Option<i64> = sqlx::query_scalar("SELECT locked_until FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_one(pool)
        .await?;
    Ok(until.map(|u| (u - t).max(1)).unwrap_or(1))
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
    async fn a_burst_of_guesses_still_locks() {
        let (pool, dan) = setup().await;
        let tries: Vec<_> = (0..20)
            .map(|_| {
                let (pool, id) = (pool.clone(), dan.id.clone());
                tokio::spawn(async move { check_secret(&pool, &id, "0000", "test").await })
            })
            .collect();
        let mut checked = 0;
        for t in tries {
            if matches!(t.await.unwrap(), Err(CoreError::BadSecret { .. })) {
                checked += 1;
            }
        }
        // Only the attempts before the lock were actually checked.
        assert!(checked < MAX_ATTEMPTS, "{checked} guesses got through");
        assert!(matches!(
            check_secret(&pool, &dan.id, "4821", "test").await,
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
