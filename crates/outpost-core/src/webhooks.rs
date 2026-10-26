//! Webhooks: an account's own URLs that Outpost tells when something finishes. Each
//! message is signed with the webhook's secret (HMAC-SHA256 of the body), so the
//! receiver can check it came from this Outpost.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

/// What a webhook can be told about.
pub const EVENTS: [&str; 3] = ["job.finished", "training.finished", "character.created"];

/// Most webhooks one account may have.
pub const MAX_PER_ACCOUNT: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Webhook {
    pub id: String,
    pub account_id: String,
    pub url: String,
    #[serde(skip)]
    pub secret: String,
    /// The events it's told about (stored comma-separated).
    #[schema(value_type = Vec<String>)]
    #[serde(serialize_with = "split_events")]
    pub events: String,
    pub created_at: i64,
    pub last_at: Option<i64>,
    /// HTTP status of the last delivery (0: couldn't connect).
    pub last_status: Option<i64>,
    pub last_error: Option<String>,
}

fn split_events<S: serde::Serializer>(events: &str, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let list: Vec<&str> = events.split(',').filter(|e| !e.is_empty()).collect();
    let mut seq = s.serialize_seq(Some(list.len()))?;
    for e in list {
        seq.serialize_element(e)?;
    }
    seq.end()
}

impl Webhook {
    pub fn wants(&self, event: &str) -> bool {
        self.events.split(',').any(|e| e == event)
    }
}

const COLS: &str = "id, account_id, url, secret, events, created_at, last_at, last_status, last_error";

fn clean_url(url: &str) -> CoreResult<String> {
    let url = url.trim();
    let ok = (url.starts_with("http://") || url.starts_with("https://"))
        && url.len() <= 500
        && !url.chars().any(char::is_whitespace)
        && url.split("://").nth(1).is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'));
    if !ok {
        return Err(CoreError::Invalid("Give a full http:// or https:// address.".into()));
    }
    Ok(url.to_string())
}

/// Make a webhook. Returns it with its secret (shown to the person once).
pub async fn create(pool: &SqlitePool, account_id: &str, url: &str, events: &[String]) -> CoreResult<(Webhook, String)> {
    let url = clean_url(url)?;
    let mut chosen: Vec<&str> = EVENTS.iter().copied().filter(|e| events.iter().any(|x| x == e)).collect();
    chosen.dedup();
    if chosen.is_empty() {
        return Err(CoreError::Invalid(format!("Pick at least one event: {}.", EVENTS.join(", "))));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhooks WHERE account_id = ?")
        .bind(account_id)
        .fetch_one(pool)
        .await?;
    if count >= MAX_PER_ACCOUNT {
        return Err(CoreError::Conflict(format!("An account can have up to {MAX_PER_ACCOUNT} webhooks.")));
    }
    let secret = format!("whsec_{}", hex(&crate::secret::random_bytes::<24>()?));
    let w = Webhook {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.into(),
        url,
        secret: secret.clone(),
        events: chosen.join(","),
        created_at: now(),
        last_at: None,
        last_status: None,
        last_error: None,
    };
    sqlx::query("INSERT INTO webhooks (id, account_id, url, secret, events, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&w.id)
        .bind(&w.account_id)
        .bind(&w.url)
        .bind(&w.secret)
        .bind(&w.events)
        .bind(w.created_at)
        .execute(pool)
        .await?;
    Ok((w, secret))
}

pub async fn list(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Webhook>> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM webhooks WHERE account_id = ? ORDER BY created_at"))
        .bind(account_id)
        .fetch_all(pool)
        .await?)
}

pub async fn get(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Webhook> {
    sqlx::query_as(&format!("SELECT {COLS} FROM webhooks WHERE id = ? AND account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

pub async fn delete(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<()> {
    let n = sqlx::query("DELETE FROM webhooks WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// The account's webhooks that want this event.
pub async fn for_event(pool: &SqlitePool, account_id: &str, event: &str) -> CoreResult<Vec<Webhook>> {
    Ok(list(pool, account_id).await?.into_iter().filter(|w| w.wants(event)).collect())
}

/// Note how a delivery went.
pub async fn record(pool: &SqlitePool, id: &str, status: i64, error: Option<&str>) -> CoreResult<()> {
    sqlx::query("UPDATE webhooks SET last_at = ?, last_status = ?, last_error = ? WHERE id = ?")
        .bind(now())
        .bind(status)
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// HMAC-SHA256 (RFC 2104) of `body` with `secret`, as lower-case hex. Sent as
/// `X-Outpost-Signature: sha256=<hex>`.
pub fn sign(secret: &str, body: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut key = secret.as_bytes().to_vec();
    if key.len() > BLOCK {
        key = Sha256::digest(&key).to_vec();
    }
    key.resize(BLOCK, 0);
    let inner: Vec<u8> = key.iter().map(|b| b ^ 0x36).collect();
    let outer: Vec<u8> = key.iter().map(|b| b ^ 0x5c).collect();
    let inner_hash = Sha256::new().chain_update(&inner).chain_update(body).finalize();
    let mac = Sha256::new().chain_update(&outer).chain_update(inner_hash).finalize();
    hex(&mac)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;

    #[test]
    fn signature_matches_the_rfc_4231_example() {
        // RFC 4231 test case 2.
        assert_eq!(
            sign("Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[tokio::test]
    async fn webhooks_keep_their_events_and_secret() {
        let pool = open_in_memory().await.unwrap();
        let dan = accounts::create(
            &pool,
            NewAccount {
                name: "Dan".into(),
                role: Role::Member,
                secret: Some("4821".into()),
            },
        )
        .await
        .unwrap()
        .id;
        assert!(create(&pool, &dan, "ftp://x", &["job.finished".into()]).await.is_err());
        assert!(create(&pool, &dan, "http://n8n.local/hook", &["nope".into()]).await.is_err());
        let (w, secret) = create(&pool, &dan, " http://n8n.local:5678/webhook/outpost ", &["job.finished".into()])
            .await
            .unwrap();
        assert!(secret.starts_with("whsec_"));
        assert!(w.wants("job.finished") && !w.wants("training.finished"));
        assert_eq!(for_event(&pool, &dan, "job.finished").await.unwrap().len(), 1);
        assert!(for_event(&pool, &dan, "training.finished").await.unwrap().is_empty());
        let json = serde_json::to_value(&w).unwrap();
        assert!(json.get("secret").is_none(), "the secret never goes out again");
        assert_eq!(json["events"], serde_json::json!(["job.finished"]));
        record(&pool, &w.id, 200, None).await.unwrap();
        assert_eq!(get(&pool, &dan, &w.id).await.unwrap().last_status, Some(200));
        delete(&pool, &dan, &w.id).await.unwrap();
        assert!(list(&pool, &dan).await.unwrap().is_empty());
    }
}
