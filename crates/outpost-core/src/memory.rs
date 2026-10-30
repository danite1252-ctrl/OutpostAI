//! Memory: short facts about the user that chats should know ("lives in Sydney",
//! "call me Dan"). Shared by all personas unless pinned to one.
//!
//! Two kinds:
//! - **manual**: added by the user. Always included in the prompt.
//! - **auto**: learned from chats, kept with the persona that learned it (so a
//!   role-play persona's chats don't colour the others). Recalled by relevance to
//!   the current message when an embedding model is installed, else the newest few.
//!   Deleting the chat a memory came from forgets it too.
//!
//! Embeddings are stored unit-length, so cosine similarity is a dot product.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Memory {
    pub id: String,
    pub account_id: String,
    /// `None`: every persona sees it.
    pub persona_id: Option<String>,
    pub text: String,
    /// `manual` (you added it) or `auto` (learned from a chat).
    pub source: String,
    /// The chat it was learned in, for `auto` memories.
    pub chat_id: Option<String>,
    pub created_at: i64,
}

/// Manual memories in a prompt, newest first.
pub const MANUAL_LIMIT: usize = 40;
/// Automatic memories recalled into a prompt.
pub const AUTO_RECALL: usize = 8;
/// Below this cosine similarity an automatic memory isn't relevant enough to recall.
pub const RECALL_THRESHOLD: f32 = 0.30;
/// At or above this similarity a new fact counts as already known.
pub const DUPLICATE_THRESHOLD: f32 = 0.88;
/// Automatic memories kept per account; the oldest go first.
pub const AUTO_LIMIT: i64 = 500;
/// Longest memory text.
pub const MAX_CHARS: usize = 500;

const COLS: &str = "id, account_id, persona_id, text, source, chat_id, created_at";

pub async fn list(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Memory>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM memories WHERE account_id = ? ORDER BY created_at DESC, rowid DESC"
    ))
    .bind(account_id)
    .fetch_all(pool)
    .await?)
}

/// Add a memory you typed yourself.
pub async fn add(pool: &SqlitePool, account_id: &str, persona_id: Option<&str>, text: &str) -> CoreResult<Memory> {
    insert(pool, account_id, persona_id, text, "manual", None, None).await
}

/// Add a memory learned from a chat with `persona_id`, with its embedding when there
/// is one. Trims the account's automatic memories to [`AUTO_LIMIT`].
pub async fn add_auto(
    pool: &SqlitePool,
    account_id: &str,
    persona_id: Option<&str>,
    chat_id: Option<&str>,
    text: &str,
    embedding: Option<(&str, &[f32])>,
) -> CoreResult<Memory> {
    let m = insert(pool, account_id, persona_id, text, "auto", chat_id, embedding).await?;
    sqlx::query(
        "DELETE FROM memories WHERE account_id = ? AND source = 'auto' AND id NOT IN
         (SELECT id FROM memories WHERE account_id = ? AND source = 'auto' ORDER BY created_at DESC, rowid DESC LIMIT ?)",
    )
    .bind(account_id)
    .bind(account_id)
    .bind(AUTO_LIMIT)
    .execute(pool)
    .await?;
    Ok(m)
}

async fn insert(
    pool: &SqlitePool,
    account_id: &str,
    persona_id: Option<&str>,
    text: &str,
    source: &str,
    chat_id: Option<&str>,
    embedding: Option<(&str, &[f32])>,
) -> CoreResult<Memory> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_CHARS {
        return Err(CoreError::Invalid("A memory must be 1 to 500 characters.".into()));
    }
    if let Some(pid) = persona_id {
        crate::personas::get(pool, account_id, pid).await?;
    }
    let m = Memory {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.into(),
        persona_id: persona_id.map(Into::into),
        text: text.into(),
        source: source.into(),
        chat_id: chat_id.map(Into::into),
        created_at: now(),
    };
    let (model, blob) = match embedding {
        Some((model, v)) => (Some(model.to_string()), Some(to_blob(&normalized(v)))),
        None => (None, None),
    };
    sqlx::query(
        "INSERT INTO memories (id, account_id, persona_id, text, source, chat_id, embedding, embedding_model, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.id)
    .bind(&m.account_id)
    .bind(&m.persona_id)
    .bind(&m.text)
    .bind(&m.source)
    .bind(&m.chat_id)
    .bind(blob)
    .bind(model)
    .bind(m.created_at)
    .execute(pool)
    .await?;
    Ok(m)
}

pub async fn delete(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<()> {
    let done = sqlx::query("DELETE FROM memories WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// Note which message taught an automatic memory. If that message was deleted while
/// the memory was being learned, the memory goes too. Returns whether it was kept.
pub async fn link_message(pool: &SqlitePool, memory_id: &str, message_id: &str) -> CoreResult<bool> {
    sqlx::query("UPDATE memories SET message_id = ? WHERE id = ?")
        .bind(message_id)
        .bind(memory_id)
        .execute(pool)
        .await?;
    let gone = sqlx::query("DELETE FROM memories WHERE id = ? AND NOT EXISTS (SELECT 1 FROM messages WHERE id = ?)")
        .bind(memory_id)
        .bind(message_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(gone == 0)
}

/// Forget what was learned from these messages. Returns how many memories went.
pub async fn forget_messages(pool: &SqlitePool, account_id: &str, message_ids: &[String]) -> CoreResult<u64> {
    let mut n = 0;
    for id in message_ids {
        n += sqlx::query("DELETE FROM memories WHERE account_id = ? AND source = 'auto' AND message_id = ?")
            .bind(account_id)
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected();
    }
    Ok(n)
}

/// Forget what was learned in a chat (before the chat is deleted). Returns how many.
pub async fn forget_chat(pool: &SqlitePool, account_id: &str, chat_id: &str) -> CoreResult<u64> {
    Ok(
        sqlx::query("DELETE FROM memories WHERE account_id = ? AND source = 'auto' AND chat_id = ?")
            .bind(account_id)
            .bind(chat_id)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// Forget everything learned automatically (manual memories stay). Returns how many.
pub async fn clear_auto(pool: &SqlitePool, account_id: &str) -> CoreResult<u64> {
    Ok(
        sqlx::query("DELETE FROM memories WHERE account_id = ? AND source = 'auto'")
            .bind(account_id)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

// ------------------------------------------------------------- automatic memory switch

fn auto_key(account_id: &str) -> String {
    format!("memory.auto.{account_id}")
}

/// Whether Outpost learns memories from this account's chats. On unless turned off.
pub async fn auto_enabled(pool: &SqlitePool, account_id: &str) -> CoreResult<bool> {
    Ok(crate::settings::get(pool, &auto_key(account_id)).await?.as_deref() != Some("off"))
}

pub async fn set_auto_enabled(pool: &SqlitePool, account_id: &str, on: bool) -> CoreResult<()> {
    crate::settings::set_bool(pool, &auto_key(account_id), on).await
}

// ------------------------------------------------------------- recall

/// A memory as stored, with its embedding when it has one.
#[derive(Debug, Clone)]
pub struct Stored {
    pub id: String,
    pub text: String,
    pub source: String,
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
}

/// Every memory a persona can see (account-wide plus its own), newest first.
pub async fn visible(pool: &SqlitePool, account_id: &str, persona_id: Option<&str>) -> CoreResult<Vec<Stored>> {
    type Row = (String, String, String, Option<Vec<u8>>, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, text, source, embedding, embedding_model FROM memories
         WHERE account_id = ? AND (persona_id IS NULL OR persona_id = ?)
         ORDER BY created_at DESC, rowid DESC",
    )
    .bind(account_id)
    .bind(persona_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, text, source, blob, model)| Stored {
            id,
            text,
            source,
            embedding: blob.as_deref().and_then(from_blob),
            embedding_model: model,
        })
        .collect())
}

/// Memories (id, text) with no embedding from `model` yet, oldest first.
pub async fn needing_embedding(
    pool: &SqlitePool,
    account_id: &str,
    model: &str,
    limit: i64,
) -> CoreResult<Vec<(String, String)>> {
    Ok(sqlx::query_as(
        "SELECT id, text FROM memories WHERE account_id = ? AND (embedding IS NULL OR embedding_model IS NOT ?)
         ORDER BY created_at LIMIT ?",
    )
    .bind(account_id)
    .bind(model)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn set_embedding(pool: &SqlitePool, id: &str, model: &str, v: &[f32]) -> CoreResult<()> {
    sqlx::query("UPDATE memories SET embedding = ?, embedding_model = ? WHERE id = ?")
        .bind(to_blob(&normalized(v)))
        .bind(model)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Pick the memory lines for a prompt. Manual memories always go in (newest
/// [`MANUAL_LIMIT`]). Automatic ones are ranked by similarity to `query` when it
/// has an embedding from `model`; otherwise the newest [`AUTO_RECALL`] go in.
pub fn select(memories: &[Stored], query: Option<(&str, &[f32])>) -> Vec<String> {
    let mut out: Vec<String> = memories
        .iter()
        .filter(|m| m.source != "auto")
        .take(MANUAL_LIMIT)
        .map(|m| m.text.clone())
        .collect();
    let auto: Vec<&Stored> = memories.iter().filter(|m| m.source == "auto").collect();
    match query {
        Some((model, q)) => {
            let q = normalized(q);
            let mut scored: Vec<(f32, &Stored)> = auto
                .iter()
                .filter_map(|m| {
                    let e = m.embedding.as_ref()?;
                    (m.embedding_model.as_deref() == Some(model) && e.len() == q.len()).then(|| (dot(e, &q), *m))
                })
                .filter(|(s, _)| *s >= RECALL_THRESHOLD)
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            out.extend(scored.into_iter().take(AUTO_RECALL).map(|(_, m)| m.text.clone()));
        }
        None => out.extend(auto.iter().take(AUTO_RECALL).map(|m| m.text.clone())),
    }
    out
}

/// Is this fact already known? Same words (ignoring case and punctuation), or an
/// embedding at least [`DUPLICATE_THRESHOLD`] similar.
pub fn is_duplicate(memories: &[Stored], text: &str, embedding: Option<(&str, &[f32])>) -> bool {
    let key = fold(text);
    if memories.iter().any(|m| fold(&m.text) == key) {
        return true;
    }
    let Some((model, v)) = embedding else { return false };
    let v = normalized(v);
    memories.iter().any(|m| {
        m.embedding_model.as_deref() == Some(model)
            && m.embedding
                .as_ref()
                .is_some_and(|e| e.len() == v.len() && dot(e, &v) >= DUPLICATE_THRESHOLD)
    })
}

fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Memories for a prompt without any embedding: manual ones plus the newest automatic.
pub async fn for_prompt(pool: &SqlitePool, account_id: &str, persona_id: Option<&str>) -> CoreResult<Vec<String>> {
    Ok(select(&visible(pool, account_id, persona_id).await?, None))
}

/// The system prompt a persona sends: its own prompt plus remembered facts.
pub fn compose_system_prompt(persona_prompt: &str, memories: &[String]) -> String {
    let mut out = persona_prompt.trim().to_string();
    if !memories.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str("Things to remember about the user:\n");
        for m in memories {
            out.push_str("- ");
            out.push_str(m);
            out.push('\n');
        }
    }
    out
}

// ------------------------------------------------------------- vectors

pub fn normalized(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 && n.is_finite() {
        v.iter().map(|x| x / n).collect()
    } else {
        v.to_vec()
    }
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub(crate) fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub(crate) fn from_blob(b: &[u8]) -> Option<Vec<f32>> {
    (!b.is_empty() && b.len() % 4 == 0).then(|| {
        b.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    })
}

#[cfg(test)]
mod message_tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;

    #[tokio::test]
    async fn deleting_a_message_forgets_what_it_taught() {
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
        let chat = crate::chats::create(&pool, &dan, None, None).await.unwrap();
        let said = crate::chats::add_message(&pool, &chat.id, "user", "I keep bees", None, None).await.unwrap();
        crate::chats::add_message(&pool, &chat.id, "assistant", "Lovely!", None, None).await.unwrap();
        let m = add_auto(&pool, &dan, None, Some(&chat.id), "The user keeps bees", None).await.unwrap();
        assert!(link_message(&pool, &m.id, &said.id).await.unwrap());
        add(&pool, &dan, None, "Call me Dan").await.unwrap();

        let removed = crate::chats::remove_message(&pool, &dan, &chat.id, &said.id).await.unwrap();
        assert_eq!(removed.len(), 2, "the reply goes with it");
        assert_eq!(forget_messages(&pool, &dan, &removed).await.unwrap(), 1);
        let left = list(&pool, &dan).await.unwrap();
        assert_eq!(left.len(), 1, "manual memories stay");
        assert!(crate::chats::messages(&pool, &chat.id).await.unwrap().is_empty());

        // Learned from a message deleted meanwhile: not kept.
        let m = add_auto(&pool, &dan, None, Some(&chat.id), "The user likes honey", None).await.unwrap();
        assert!(!link_message(&pool, &m.id, &said.id).await.unwrap());
        assert_eq!(list(&pool, &dan).await.unwrap().len(), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;
    use crate::personas::{self, PersonaInput};

    async fn dan(pool: &SqlitePool) -> String {
        accounts::create(
            pool,
            NewAccount {
                name: "Dan".into(),
                role: Role::Owner,
                secret: Some("4821".into()),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn shared_and_persona_memories() {
        let pool = open_in_memory().await.unwrap();
        let dan = dan(&pool).await;
        let coder = personas::create(
            &pool,
            &dan,
            PersonaInput {
                name: Some("Coder".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let other = personas::create(
            &pool,
            &dan,
            PersonaInput {
                name: Some("Writer".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        add(&pool, &dan, None, "Lives in Sydney").await.unwrap();
        add(&pool, &dan, Some(&coder.id), "Prefers Rust").await.unwrap();

        let for_coder = for_prompt(&pool, &dan, Some(&coder.id)).await.unwrap();
        assert_eq!(for_coder.len(), 2);
        let for_writer = for_prompt(&pool, &dan, Some(&other.id)).await.unwrap();
        assert_eq!(for_writer, vec!["Lives in Sydney".to_string()]);

        let prompt = compose_system_prompt("Be terse.", &for_writer);
        assert!(prompt.starts_with("Be terse."));
        assert!(prompt.contains("- Lives in Sydney"));
        assert_eq!(compose_system_prompt("Be terse.", &[]), "Be terse.");
    }

    #[test]
    fn vectors_round_trip_and_normalise() {
        let v = normalized(&[3.0, 4.0]);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        assert_eq!(from_blob(&to_blob(&v)).unwrap(), v);
        assert!(from_blob(&[1, 2, 3]).is_none());
        assert_eq!(normalized(&[0.0, 0.0]), vec![0.0, 0.0]);
    }

    #[tokio::test]
    async fn auto_memories_recall_by_relevance_and_skip_duplicates() {
        let pool = open_in_memory().await.unwrap();
        let dan = dan(&pool).await;
        let m = "test-embed";
        add(&pool, &dan, None, "Call me Dan").await.unwrap();
        // Toy 3-d "embeddings": x = pets, y = food, z = work.
        add_auto(
            &pool,
            &dan,
            None,
            None,
            "The user has a dog called Rex",
            Some((m, &[1.0, 0.0, 0.1])),
        )
        .await
        .unwrap();
        add_auto(
            &pool,
            &dan,
            None,
            None,
            "The user is vegetarian",
            Some((m, &[0.0, 1.0, 0.0])),
        )
        .await
        .unwrap();
        add_auto(
            &pool,
            &dan,
            None,
            None,
            "The user works night shifts",
            Some((m, &[0.0, 0.1, 1.0])),
        )
        .await
        .unwrap();

        let all = visible(&pool, &dan, None).await.unwrap();
        assert_eq!(all.len(), 4);

        // A question about dinner recalls the food fact, and the manual one always comes along.
        let picked = select(&all, Some((m, &[0.1, 1.0, 0.0])));
        assert_eq!(picked[0], "Call me Dan");
        assert!(picked.contains(&"The user is vegetarian".to_string()));
        assert!(!picked.contains(&"The user has a dog called Rex".to_string()));

        // Embeddings from another model are ignored rather than compared.
        let other = select(&all, Some(("other-model", &[0.1, 1.0, 0.0])));
        assert_eq!(other, vec!["Call me Dan".to_string()]);

        // No query embedding: newest automatic memories.
        assert_eq!(select(&all, None).len(), 4);

        // Duplicates by words or by meaning.
        assert!(is_duplicate(&all, "the user is VEGETARIAN.", None));
        assert!(is_duplicate(
            &all,
            "The user doesn't eat meat",
            Some((m, &[0.05, 1.0, 0.0]))
        ));
        assert!(!is_duplicate(
            &all,
            "The user plays guitar",
            Some((m, &[0.5, -0.5, 0.5]))
        ));

        // Backfill finds what has no embedding yet (the manual memory).
        let todo = needing_embedding(&pool, &dan, m, 10).await.unwrap();
        assert_eq!(todo.len(), 1);
        set_embedding(&pool, &todo[0].0, m, &[0.0, 0.0, 2.0]).await.unwrap();
        assert!(needing_embedding(&pool, &dan, m, 10).await.unwrap().is_empty());

        // Learned in a chat that is then deleted: forgotten with it.
        let chat = crate::chats::create(&pool, &dan, None, None).await.unwrap();
        add_auto(&pool, &dan, None, Some(&chat.id), "The user collects stamps", None)
            .await
            .unwrap();
        assert_eq!(forget_chat(&pool, &dan, &chat.id).await.unwrap(), 1);

        // Switch and clearing.
        assert!(auto_enabled(&pool, &dan).await.unwrap());
        set_auto_enabled(&pool, &dan, false).await.unwrap();
        assert!(!auto_enabled(&pool, &dan).await.unwrap());
        assert_eq!(clear_auto(&pool, &dan).await.unwrap(), 3);
        assert_eq!(list(&pool, &dan).await.unwrap().len(), 1);
    }
}
