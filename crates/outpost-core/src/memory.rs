//! Memory: short facts an account wants every chat to know ("I live in Sydney",
//! "call me Dan"). Shared by all personas unless pinned to one. Automatic memory
//! with embeddings builds on this later.

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
    pub created_at: i64,
}

/// At most this many memories go into a prompt (newest first).
pub const PROMPT_LIMIT: i64 = 50;

pub async fn list(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Memory>> {
    Ok(sqlx::query_as(
        "SELECT id, account_id, persona_id, text, created_at FROM memories WHERE account_id = ? ORDER BY created_at DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?)
}

pub async fn add(pool: &SqlitePool, account_id: &str, persona_id: Option<&str>, text: &str) -> CoreResult<Memory> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 500 {
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
        created_at: now(),
    };
    sqlx::query("INSERT INTO memories (id, account_id, persona_id, text, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&m.id)
        .bind(&m.account_id)
        .bind(&m.persona_id)
        .bind(&m.text)
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

/// Memories for a prompt: the account-wide ones plus the persona's own.
pub async fn for_prompt(pool: &SqlitePool, account_id: &str, persona_id: Option<&str>) -> CoreResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT text FROM memories WHERE account_id = ? AND (persona_id IS NULL OR persona_id = ?)
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(account_id)
    .bind(persona_id)
    .bind(PROMPT_LIMIT)
    .fetch_all(pool)
    .await?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;
    use crate::personas::{self, PersonaInput};

    #[tokio::test]
    async fn shared_and_persona_memories() {
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
        .unwrap()
        .id;
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
}
