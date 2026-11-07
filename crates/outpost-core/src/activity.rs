//! The activity log: every tool call a persona makes (who, from which chat or routine, the
//! action exactly as the model sent it, and what came of it). Owners see everyone's; people
//! see their own. Kept for 90 days.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreResult};

/// How long entries are kept.
pub const KEEP_DAYS: i64 = 90;

/// One call, to record.
#[derive(Debug, Clone, Default)]
pub struct NewEntry {
    pub account_id: String,
    pub chat_id: Option<String>,
    pub routine_id: Option<String>,
    /// chat, hands_free or routine.
    pub source: String,
    /// The tool ("Home Assistant").
    pub server: String,
    /// Its action ("HassTurnOff").
    pub tool: String,
    /// The arguments as the model sent them (JSON text).
    pub arguments: String,
    /// done, failed, not_allowed, skipped or needs_ok.
    pub outcome: String,
    /// What it said back (shortened).
    pub result: String,
}

/// One call, as the apps see it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Entry {
    pub id: String,
    pub at: i64,
    pub account_id: String,
    /// Who it was for.
    pub person: Option<String>,
    pub chat_id: Option<String>,
    pub chat_title: Option<String>,
    pub routine_id: Option<String>,
    pub routine_name: Option<String>,
    pub source: String,
    pub server: String,
    pub tool: String,
    pub arguments: String,
    pub outcome: String,
    pub result: String,
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Record a call (never fails the caller: a log line mustn't stop a reply).
pub async fn record(pool: &SqlitePool, e: NewEntry) {
    let result = sqlx::query(
        "INSERT INTO activity (id, at, account_id, chat_id, routine_id, source, server, tool, arguments, outcome, result) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(now())
    .bind(&e.account_id)
    .bind(&e.chat_id)
    .bind(&e.routine_id)
    .bind(&e.source)
    .bind(cut(&e.server, 120))
    .bind(cut(&e.tool, 120))
    .bind(cut(&e.arguments, 4000))
    .bind(&e.outcome)
    .bind(cut(&e.result, 1000))
    .execute(pool)
    .await;
    if let Err(err) = result {
        tracing::warn!(error = %err, tool = %e.tool, "couldn't record a tool call");
    }
}

/// What to show.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Only this person's (everyone's when `None` and the viewer may see all).
    pub person: Option<String>,
    /// Older than this (paging back).
    pub before: Option<i64>,
    /// Only this tool ("Home Assistant").
    pub server: Option<String>,
    pub limit: i64,
}

/// The newest calls first. `everyone`: the viewer is an owner (else only their own).
pub async fn list(pool: &SqlitePool, viewer_id: &str, everyone: bool, f: &Filter) -> CoreResult<Vec<Entry>> {
    let person = if everyone { f.person.clone() } else { Some(viewer_id.to_string()) };
    Ok(sqlx::query_as(
        "SELECT a.id, a.at, a.account_id, ac.name AS person, a.chat_id, c.title AS chat_title, a.routine_id, \
                r.name AS routine_name, a.source, a.server, a.tool, a.arguments, a.outcome, a.result \
         FROM activity a \
         LEFT JOIN accounts ac ON ac.id = a.account_id \
         LEFT JOIN chats c ON c.id = a.chat_id \
         LEFT JOIN routines r ON r.id = a.routine_id \
         WHERE (? IS NULL OR a.account_id = ?) AND (? IS NULL OR a.at < ?) AND (? IS NULL OR a.server = ?) \
         ORDER BY a.at DESC, a.rowid DESC LIMIT ?",
    )
    .bind(&person)
    .bind(&person)
    .bind(f.before)
    .bind(f.before)
    .bind(&f.server)
    .bind(&f.server)
    .bind(f.limit.clamp(1, 200))
    .fetch_all(pool)
    .await?)
}

/// Forget calls older than [`KEEP_DAYS`].
pub async fn prune(pool: &SqlitePool) -> CoreResult<u64> {
    let done = sqlx::query("DELETE FROM activity WHERE at < ?").bind(now() - KEEP_DAYS * 86_400).execute(pool).await?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};

    #[tokio::test]
    async fn owners_see_everyones_calls_and_people_their_own() {
        let pool = crate::db::open_in_memory().await.unwrap();
        let dan = accounts::create(&pool, NewAccount { name: "Dan".into(), role: Role::Owner, secret: Some("4821".into()) }).await.unwrap();
        let sj = accounts::create(&pool, NewAccount { name: "SJ".into(), role: Role::Member, secret: Some("1234".into()) }).await.unwrap();
        let call = |who: &str, tool: &str, args: &str, outcome: &str| NewEntry {
            account_id: who.into(),
            source: "chat".into(),
            server: "Home Assistant".into(),
            tool: tool.into(),
            arguments: args.into(),
            outcome: outcome.into(),
            result: "ok".into(),
            ..Default::default()
        };
        record(&pool, call(&dan.id, "HassTurnOff", r#"{"name":"kitchen"}"#, "done")).await;
        record(&pool, call(&sj.id, "HassTurnOn", r#"{"name":"aircon"}"#, "done")).await;
        record(&pool, call(&sj.id, "HassTurnOff", r#"{"domain":["lock"]}"#, "skipped")).await;

        let all = list(&pool, &dan.id, true, &Filter { limit: 50, ..Default::default() }).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].outcome, "skipped", "newest first");
        assert_eq!(all[0].person.as_deref(), Some("SJ"));
        // The exact name the model sent is kept.
        assert!(all.iter().any(|e| e.arguments == r#"{"name":"kitchen"}"#));
        let only_sj = list(&pool, &dan.id, true, &Filter { person: Some(sj.id.clone()), limit: 50, ..Default::default() }).await.unwrap();
        assert_eq!(only_sj.len(), 2);
        // SJ only sees hers, whatever she asks for.
        let hers = list(&pool, &sj.id, false, &Filter { person: Some(dan.id.clone()), limit: 50, ..Default::default() }).await.unwrap();
        assert_eq!(hers.len(), 2);
        assert!(hers.iter().all(|e| e.account_id == sj.id));
        assert_eq!(prune(&pool).await.unwrap(), 0);
    }
}
