//! Append-only audit log: who did what, from which session, when.

use sqlx::SqlitePool;

use crate::{now, CoreResult};

pub async fn log(
    pool: &SqlitePool,
    account_id: Option<&str>,
    session_id: Option<&str>,
    action: &str,
    detail: Option<&str>,
) -> CoreResult<()> {
    sqlx::query("INSERT INTO audit_log (at, account_id, session_id, action, detail) VALUES (?, ?, ?, ?, ?)")
        .bind(now())
        .bind(account_id)
        .bind(session_id)
        .bind(action)
        .bind(detail)
        .execute(pool)
        .await?;
    Ok(())
}

/// Write an audit entry without failing the caller; a broken log line should never
/// block a sign-in. Errors are reported to the tracing log instead.
pub async fn log_quiet(
    pool: &SqlitePool,
    account_id: Option<&str>,
    session_id: Option<&str>,
    action: &str,
    detail: Option<&str>,
) {
    if let Err(e) = log(pool, account_id, session_id, action, detail).await {
        tracing::warn!(error = %e, action, "could not write audit entry");
    }
}
