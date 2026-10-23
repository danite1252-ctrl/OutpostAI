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

pub async fn messages(pool: &SqlitePool, chat_id: &str) -> CoreResult<Vec<Message>> {
    Ok(sqlx::query_as(
        "SELECT id, chat_id, seq, role, content, thinking, model, created_at FROM messages WHERE chat_id = ? ORDER BY seq",
    )
    .bind(chat_id)
    .fetch_all(pool)
    .await?)
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
    let mut tx = pool.begin().await?;
    let seq: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE chat_id = ?")
        .bind(chat_id)
        .fetch_one(&mut *tx)
        .await?;
    let msg = Message {
        id: uuid::Uuid::new_v4().to_string(),
        chat_id: chat_id.to_string(),
        seq,
        role: role.to_string(),
        content: content.to_string(),
        thinking: thinking.filter(|t| !t.is_empty()).map(Into::into),
        model: model.map(Into::into),
        created_at: t,
    };
    sqlx::query(
        "INSERT INTO messages (id, chat_id, seq, role, content, thinking, model, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&msg.id)
    .bind(&msg.chat_id)
    .bind(msg.seq)
    .bind(&msg.role)
    .bind(&msg.content)
    .bind(&msg.thinking)
    .bind(&msg.model)
    .bind(t)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE chats SET updated_at = ? WHERE id = ?")
        .bind(t)
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
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
