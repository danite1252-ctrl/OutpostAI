//! Sign-in sessions. Each signed-in desktop, paired phone or app key is one session
//! holding a bearer token. Only the token's hash is stored.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::accounts::{self, Account};
use crate::secret::{hash_token, new_token};
use crate::{now, CoreError, CoreResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    /// The desktop app on the Outpost PC.
    Desktop,
    /// A paired phone or tablet.
    Device,
    /// An app key, e.g. for Hermes.
    App,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Desktop => "desktop",
            SessionKind::Device => "device",
            SessionKind::App => "app",
        }
    }

    fn parse(s: &str) -> CoreResult<SessionKind> {
        match s {
            "desktop" => Ok(SessionKind::Desktop),
            "device" => Ok(SessionKind::Device),
            "app" => Ok(SessionKind::App),
            other => Err(CoreError::Internal(format!(
                "unknown session kind in database: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Session {
    pub id: String,
    pub account_id: String,
    pub kind: SessionKind,
    /// Human label, e.g. "Outpost desktop" or "Dan's Pixel".
    pub label: String,
    pub created_at: i64,
    pub last_used_at: i64,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    account_id: String,
    kind: String,
    label: String,
    created_at: i64,
    last_used_at: i64,
}

impl TryFrom<SessionRow> for Session {
    type Error = CoreError;
    fn try_from(r: SessionRow) -> CoreResult<Session> {
        Ok(Session {
            id: r.id,
            account_id: r.account_id,
            kind: SessionKind::parse(&r.kind)?,
            label: r.label,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
        })
    }
}

/// Start a session for an account. Returns the bearer token (shown once) and the session.
pub async fn create(
    pool: &SqlitePool,
    account_id: &str,
    kind: SessionKind,
    label: &str,
) -> CoreResult<(String, Session)> {
    let (token, token_hash) = new_token()?;
    let label: String = label.trim().chars().take(60).collect();
    let t = now();
    let session = Session {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        kind,
        label: if label.is_empty() {
            kind.as_str().to_string()
        } else {
            label
        },
        created_at: t,
        last_used_at: t,
    };
    sqlx::query(
        "INSERT INTO sessions (id, account_id, token_hash, kind, label, created_at, last_used_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&session.id)
    .bind(&session.account_id)
    .bind(token_hash)
    .bind(kind.as_str())
    .bind(&session.label)
    .bind(t)
    .bind(t)
    .execute(pool)
    .await?;
    Ok((token, session))
}

/// Look up a live session by its bearer token and mark it used.
pub async fn authenticate(pool: &SqlitePool, token: &str) -> CoreResult<Option<(Session, Account)>> {
    let row: Option<SessionRow> = sqlx::query_as(
        "SELECT id, account_id, kind, label, created_at, last_used_at FROM sessions WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let mut session = Session::try_from(row)?;
    let account = match accounts::get(pool, &session.account_id).await {
        Ok(a) => a,
        Err(CoreError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let t = now();
    // Only write when it has moved, to keep busy clients from hammering the DB.
    if t - session.last_used_at >= 60 {
        sqlx::query("UPDATE sessions SET last_used_at = ? WHERE id = ?")
            .bind(t)
            .bind(&session.id)
            .execute(pool)
            .await?;
        session.last_used_at = t;
    }
    Ok(Some((session, account)))
}

pub async fn revoke(pool: &SqlitePool, session_id: &str) -> CoreResult<()> {
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(now())
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_for_account(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Session>> {
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT id, account_id, kind, label, created_at, last_used_at FROM sessions WHERE account_id = ? AND revoked_at IS NULL ORDER BY last_used_at DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(Session::try_from).collect()
}
