//! Chats and their messages. Private to the account that made them.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Chat {
    pub id: String,
    pub account_id: String,
    pub persona_id: Option<String>,
    pub title: String,
    /// Overrides the persona's model when set.
    pub model: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Message {
    pub id: String,
    pub chat_id: String,
    pub seq: i64,
    /// `user`, `assistant` or `system`.
    pub role: String,
    pub content: String,
    /// The model's reasoning, for thinking models.
    pub thinking: Option<String>,
    /// Model that wrote an assistant message.
    pub model: Option<String>,
    pub created_at: i64,
    /// Images attached to this message (fetch each with `GET /api/files/{id}`).
    #[sqlx(skip)]
    #[serde(default)]
    pub attachments: Vec<crate::attachments::Attachment>,
}

const CHAT_COLS: &str = "id, account_id, persona_id, title, model, created_at, updated_at";

/// "Untitled chat", or the start of the first message.
pub fn title_from(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    if title.is_empty() {
        "Untitled chat".into()
    } else {
        title
    }
}

pub async fn list(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Chat>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {CHAT_COLS} FROM chats WHERE account_id = ? ORDER BY updated_at DESC LIMIT 500"
    ))
    .bind(account_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Chat> {
    sqlx::query_as(&format!(
        "SELECT {CHAT_COLS} FROM chats WHERE id = ? AND account_id = ?"
    ))
    .bind(id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?
    .ok_or(CoreError::NotFound)
}

pub async fn create(
    pool: &SqlitePool,
    account_id: &str,
    persona_id: Option<&str>,
    title: Option<&str>,
) -> CoreResult<Chat> {
    if let Some(pid) = persona_id {
        // Only your own personas.
        crate::personas::get(pool, account_id, pid).await?;
    }
    let t = now();
    let chat = Chat {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        persona_id: persona_id.map(Into::into),
        title: title.map(title_from).unwrap_or_else(|| "New chat".into()),
        model: None,
        created_at: t,
        updated_at: t,
    };
    sqlx::query(
        "INSERT INTO chats (id, account_id, persona_id, title, model, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&chat.id)
    .bind(&chat.account_id)
    .bind(&chat.persona_id)
    .bind(&chat.title)
    .bind(&chat.model)
    .bind(t)
    .bind(t)
    .execute(pool)
    .await?;
    Ok(chat)
}

/// Change a chat's title, persona or model override. `Some("")` clears the model override.
pub async fn update(
    pool: &SqlitePool,
    account_id: &str,
    id: &str,
    title: Option<&str>,
    persona_id: Option<&str>,
    model: Option<&str>,
) -> CoreResult<Chat> {
    let mut chat = get(pool, account_id, id).await?;
    if let Some(t) = title {
        let t = t.trim();
        if t.is_empty() || t.chars().count() > 120 {
            return Err(CoreError::Invalid("A title must be 1 to 120 characters.".into()));
        }
        chat.title = t.to_string();
    }
    if let Some(pid) = persona_id {
        crate::personas::get(pool, account_id, pid).await?;
        chat.persona_id = Some(pid.to_string());
    }
    if let Some(m) = model {
        if !m.is_empty() && !crate::personas::valid_model_name(m) {
            return Err(CoreError::Invalid(format!("\"{m}\" is not a valid model name.")));
        }
        chat.model = Some(m.to_string()).filter(|m| !m.is_empty());
    }
    chat.updated_at = now();
    sqlx::query(
        "UPDATE chats SET title = ?, persona_id = ?, model = ?, updated_at = ? WHERE id = ? AND account_id = ?",
    )
    .bind(&chat.title)
    .bind(&chat.persona_id)
    .bind(&chat.model)
    .bind(chat.updated_at)
    .bind(id)
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(chat)
}

pub async fn delete(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<()> {
    let done = sqlx::query("DELETE FROM chats WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// A chat's messages in order, each with its attachments.
pub async fn messages(pool: &SqlitePool, chat_id: &str) -> CoreResult<Vec<Message>> {
    let mut list: Vec<Message> = sqlx::query_as(
        "SELECT id, chat_id, seq, role, content, thinking, model, created_at FROM messages WHERE chat_id = ? ORDER BY seq",
    )
    .bind(chat_id)
    .fetch_all(pool)
    .await?;
    let mut files = crate::attachments::for_chat(pool, chat_id).await?;
    for m in &mut list {
        if let Some(a) = files.remove(&m.id) {
            m.attachments = a;
        }
    }
    Ok(list)
}

/// Remove one message (used to undo a send that failed half-way).
pub async fn delete_message(pool: &SqlitePool, chat_id: &str, message_id: &str) -> CoreResult<()> {
    sqlx::query("DELETE FROM messages WHERE id = ? AND chat_id = ?")
        .bind(message_id)
        .bind(chat_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete one of my messages. A message I sent takes the reply that answered it with
/// it. Returns the ids removed (their attachments' rows go too; the caller removes
/// the files first with [`crate::attachments::paths_for_message`]).
pub async fn remove_message(pool: &SqlitePool, account_id: &str, chat_id: &str, message_id: &str) -> CoreResult<Vec<String>> {
    get(pool, account_id, chat_id).await?;
    let all = messages(pool, chat_id).await?;
    let i = all.iter().position(|m| m.id == message_id).ok_or(CoreError::NotFound)?;
    let mut ids = vec![all[i].id.clone()];
    if all[i].role == "user" {
        if let Some(reply) = all.get(i + 1).filter(|m| m.role == "assistant") {
            ids.push(reply.id.clone());
        }
    }
    for id in &ids {
        sqlx::query("DELETE FROM attachments WHERE message_id = ?").bind(id).execute(pool).await?;
        delete_message(pool, chat_id, id).await?;
    }
    Ok(ids)
}

/// Append a message and bump the chat to the top of the list.
pub async fn add_message(
    pool: &SqlitePool,
    chat_id: &str,
    role: &str,
    content: &str,
    thinking: Option<&str>,
    model: Option<&str>,
) -> CoreResult<Message> {
    if !matches!(role, "user" | "assistant" | "system") {
        return Err(CoreError::Invalid(format!("Unknown role {role}.")));
    }
    let t = now();
    let mut msg = Message {
        id: uuid::Uuid::new_v4().to_string(),
        chat_id: chat_id.to_string(),
        seq: 0,
        role: role.to_string(),
        content: content.to_string(),
        thinking: thinking.filter(|t| !t.is_empty()).map(Into::into),
        model: model.map(Into::into),
        created_at: t,
        attachments: Vec::new(),
    };
    // One statement, so it waits its turn if something else is writing (a read-then-
    // write transaction would fail instead, e.g. while the job queue starts a job).
    msg.seq = sqlx::query_scalar(
        "INSERT INTO messages (id, chat_id, seq, role, content, thinking, model, created_at)
         SELECT ?, ?, COALESCE(MAX(seq), 0) + 1, ?, ?, ?, ?, ? FROM messages WHERE chat_id = ?
         RETURNING seq",
    )
    .bind(&msg.id)
    .bind(&msg.chat_id)
    .bind(&msg.role)
    .bind(&msg.content)
    .bind(&msg.thinking)
    .bind(&msg.model)
    .bind(t)
    .bind(chat_id)
    .fetch_one(pool)
    .await?;
    sqlx::query("UPDATE chats SET updated_at = ? WHERE id = ?")
        .bind(t)
        .bind(chat_id)
        .execute(pool)
        .await?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;

    #[test]
    fn titles() {
        assert_eq!(title_from("\n  Hello there\nsecond line"), "Hello there");
        assert_eq!(title_from(""), "Untitled chat");
        assert!(title_from(&"x".repeat(100)).ends_with('…'));
    }

    #[tokio::test]
    async fn messages_keep_order_and_privacy() {
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
        let sarah = accounts::create(
            &pool,
            NewAccount {
                name: "Sarah".into(),
                role: Role::Member,
                secret: Some("hunter22".into()),
            },
        )
        .await
        .unwrap();
        let chat = create(&pool, &dan.id, None, Some("Plan a trip")).await.unwrap();
        add_message(&pool, &chat.id, "user", "Hi", None, None).await.unwrap();
        add_message(
            &pool,
            &chat.id,
            "assistant",
            "Hello!",
            Some("greet back"),
            Some("qwen3.5:4b"),
        )
        .await
        .unwrap();
        let msgs = messages(&pool, &chat.id).await.unwrap();
        assert_eq!(msgs.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(msgs[1].thinking.as_deref(), Some("greet back"));

        assert!(matches!(
            get(&pool, &sarah.id, &chat.id).await,
            Err(CoreError::NotFound)
        ));
        assert!(matches!(
            delete(&pool, &sarah.id, &chat.id).await,
            Err(CoreError::NotFound)
        ));
        assert_eq!(list(&pool, &dan.id).await.unwrap().len(), 1);
        delete(&pool, &dan.id, &chat.id).await.unwrap();
        assert!(messages(&pool, &chat.id).await.unwrap().is_empty());
    }
}

/// A chat's rolling summary of its older messages, and the last message seq it covers.
pub async fn summary(pool: &SqlitePool, chat_id: &str) -> CoreResult<(Option<String>, i64)> {
    Ok(sqlx::query_as::<_, (Option<String>, i64)>("SELECT summary, summary_upto FROM chats WHERE id = ?")
        .bind(chat_id)
        .fetch_optional(pool)
        .await?
        .unwrap_or((None, 0)))
}

pub async fn set_summary(pool: &SqlitePool, chat_id: &str, summary: &str, upto: i64) -> CoreResult<()> {
    sqlx::query("UPDATE chats SET summary = ?, summary_upto = ? WHERE id = ?")
        .bind(summary)
        .bind(upto)
        .bind(chat_id)
        .execute(pool)
        .await?;
    Ok(())
}
