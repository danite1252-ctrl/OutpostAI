//! Documents to chat with: PDFs, Word files and text, private to the account that added
//! them. Each one's text is kept in passages ("chunks"), with an embedding each, so a chat
//! can find the passages relevant to a question. A document is used by the chats and
//! personas it's attached to (a shared persona's documents go with it).
//!
//! Reading the file and making the passages happen in the gateway; this module stores them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::memory::{dot, from_blob, normalized, to_blob};
use crate::{now, CoreError, CoreResult};

/// Largest document file.
pub const MAX_BYTES: usize = 50 * 1024 * 1024;
/// Most text kept from one document (about a 1,000-page book).
pub const MAX_CHARS: usize = 3_000_000;
/// Most documents one chat or persona uses.
pub const MAX_ATTACHED: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Document {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub mime: String,
    pub bytes: i64,
    /// `reading` (being read and indexed), `ready` or `failed`.
    pub status: String,
    pub error: Option<String>,
    /// Pages, for a PDF.
    pub pages: Option<i64>,
    /// Characters of text found.
    pub chars: i64,
    pub created_at: i64,
}

const COLS: &str = "id, account_id, name, mime, bytes, status, error, pages, chars, created_at";

/// A passage a reply used, numbered as the reply cites it ("[1]").
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct Source {
    pub n: u32,
    pub document_id: String,
    pub name: String,
    pub page: Option<i64>,
}

/// One passage of a document, with its embedding if it has one from `model`.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub document_id: String,
    pub document_name: String,
    pub idx: i64,
    pub page: Option<i64>,
    pub text: String,
    pub embedding: Option<Vec<f32>>,
}

/// A passage to store.
#[derive(Debug, Clone, PartialEq)]
pub struct NewChunk {
    pub page: Option<u32>,
    pub text: String,
}

/// The kinds of document Outpost reads, by file name: (mime, extension).
pub fn kind_of(name: &str) -> Option<(&'static str, &'static str)> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "pdf" => ("application/pdf", "pdf"),
        "docx" => ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "docx"),
        "txt" | "text" | "log" => ("text/plain", "txt"),
        "md" | "markdown" => ("text/markdown", "md"),
        "csv" => ("text/csv", "csv"),
        "html" | "htm" => ("text/html", "html"),
        _ => return None,
    })
}

fn io(e: std::io::Error) -> CoreError {
    CoreError::Invalid(format!("Could not store the document: {e}"))
}

/// Keep a new document (not read yet: status `reading`). The same file added again by the
/// same account returns the one already there.
pub async fn add(pool: &SqlitePool, files_dir: &Path, account_id: &str, name: &str, data: &[u8]) -> CoreResult<(Document, bool)> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(200).collect();
    let (mime, ext) = kind_of(&name).ok_or_else(|| {
        CoreError::Invalid("Outpost reads PDF, Word (.docx), text, Markdown, CSV and HTML files.".into())
    })?;
    if data.is_empty() {
        return Err(CoreError::Invalid("That file is empty.".into()));
    }
    if data.len() > MAX_BYTES {
        return Err(CoreError::Invalid(format!("Documents can be up to {} MB.", MAX_BYTES / 1024 / 1024)));
    }
    let sha = hex(&Sha256::digest(data));
    let same: Option<Document> =
        sqlx::query_as(&format!("SELECT {COLS} FROM documents WHERE account_id = ? AND sha256 = ? AND status != 'failed'"))
            .bind(account_id)
            .bind(&sha)
            .fetch_optional(pool)
            .await?;
    if let Some(d) = same {
        return Ok((d, false));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let rel = format!("{account_id}/documents/{id}.{ext}");
    let full = files_dir.join(&rel);
    if let Some(dir) = full.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io)?;
    }
    tokio::fs::write(&full, data).await.map_err(io)?;
    let d = Document {
        id,
        account_id: account_id.into(),
        name,
        mime: mime.into(),
        bytes: data.len() as i64,
        status: "reading".into(),
        error: None,
        pages: None,
        chars: 0,
        created_at: now(),
    };
    let saved = sqlx::query(
        "INSERT INTO documents (id, account_id, name, mime, bytes, path, sha256, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'reading', ?)",
    )
    .bind(&d.id)
    .bind(&d.account_id)
    .bind(&d.name)
    .bind(&d.mime)
    .bind(d.bytes)
    .bind(&rel)
    .bind(&sha)
    .bind(d.created_at)
    .execute(pool)
    .await;
    if let Err(e) = saved {
        let _ = tokio::fs::remove_file(&full).await;
        return Err(e.into());
    }
    Ok((d, true))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// My documents, newest first.
pub async fn list(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Document>> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM documents WHERE account_id = ? ORDER BY created_at DESC, name"))
        .bind(account_id)
        .fetch_all(pool)
        .await?)
}

/// One of my documents, and its file.
pub async fn get(pool: &SqlitePool, files_dir: &Path, account_id: &str, id: &str) -> CoreResult<(Document, PathBuf)> {
    let row: Option<(String,)> = sqlx::query_as("SELECT path FROM documents WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?;
    let (rel,) = row.ok_or(CoreError::NotFound)?;
    let d: Document = sqlx::query_as(&format!("SELECT {COLS} FROM documents WHERE id = ?"))
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok((d, files_dir.join(rel)))
}

/// Delete one of my documents: its file, passages and attachments.
pub async fn delete(pool: &SqlitePool, files_dir: &Path, account_id: &str, id: &str) -> CoreResult<()> {
    let (_, path) = get(pool, files_dir, account_id, id).await?;
    sqlx::query("DELETE FROM documents WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    let _ = tokio::fs::remove_file(&path).await;
    Ok(())
}

/// Store a document's passages and mark it ready (replacing any it had).
pub async fn set_chunks(pool: &SqlitePool, id: &str, pages: Option<u32>, chunks: &[NewChunk]) -> CoreResult<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM document_chunks WHERE document_id = ?").bind(id).execute(&mut *tx).await?;
    let mut chars = 0i64;
    for (i, c) in chunks.iter().enumerate() {
        chars += c.text.chars().count() as i64;
        sqlx::query("INSERT INTO document_chunks (document_id, idx, page, text) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind(i as i64)
            .bind(c.page.map(i64::from))
            .bind(&c.text)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE documents SET status = 'ready', error = NULL, pages = ?, chars = ? WHERE id = ?")
        .bind(pages.map(i64::from))
        .bind(chars)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_failed(pool: &SqlitePool, id: &str, error: &str) -> CoreResult<()> {
    sqlx::query("UPDATE documents SET status = 'failed', error = ? WHERE id = ?")
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// At start-up: documents left `reading` by a restart, to read again.
pub async fn unread(pool: &SqlitePool) -> CoreResult<Vec<(String, PathBuf)>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, path FROM documents WHERE status = 'reading'")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id, p)| (id, PathBuf::from(p))).collect())
}

/// The file of a document (any account), for reading it.
pub async fn file_of(pool: &SqlitePool, files_dir: &Path, id: &str) -> CoreResult<(Document, PathBuf)> {
    let row: Option<(String, String)> = sqlx::query_as("SELECT account_id, path FROM documents WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let (account, _) = row.ok_or(CoreError::NotFound)?;
    get(pool, files_dir, &account, id).await
}

// ------------------------------------------------------------------ attachments

/// Documents attached to a chat (mine only).
pub async fn for_chat(pool: &SqlitePool, chat_id: &str) -> CoreResult<Vec<Document>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {} FROM documents d JOIN chat_documents c ON c.document_id = d.id
         WHERE c.chat_id = ? ORDER BY d.name",
        prefixed()
    ))
    .bind(chat_id)
    .fetch_all(pool)
    .await?)
}

/// Documents attached to a persona (its owner's).
pub async fn for_persona(pool: &SqlitePool, persona_id: &str) -> CoreResult<Vec<Document>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {} FROM documents d JOIN persona_documents p ON p.document_id = d.id
         WHERE p.persona_id = ? ORDER BY d.name",
        prefixed()
    ))
    .bind(persona_id)
    .fetch_all(pool)
    .await?)
}

fn prefixed() -> String {
    COLS.split(", ").map(|c| format!("d.{c}")).collect::<Vec<_>>().join(", ")
}

/// Check the documents are `account_id`'s own, and not too many.
async fn check_own(pool: &SqlitePool, account_id: &str, ids: &[String]) -> CoreResult<()> {
    if ids.len() > MAX_ATTACHED {
        return Err(CoreError::Invalid(format!("Attach up to {MAX_ATTACHED} documents.")));
    }
    for id in ids {
        let own: Option<(String,)> = sqlx::query_as("SELECT id FROM documents WHERE id = ? AND account_id = ?")
            .bind(id)
            .bind(account_id)
            .fetch_optional(pool)
            .await?;
        if own.is_none() {
            return Err(CoreError::NotFound);
        }
    }
    Ok(())
}

/// Set which of my documents a chat uses (the chat must be mine; checked by the caller).
pub async fn set_for_chat(pool: &SqlitePool, account_id: &str, chat_id: &str, ids: &[String]) -> CoreResult<()> {
    check_own(pool, account_id, ids).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM chat_documents WHERE chat_id = ?").bind(chat_id).execute(&mut *tx).await?;
    for id in ids {
        sqlx::query("INSERT OR IGNORE INTO chat_documents (chat_id, document_id) VALUES (?, ?)")
            .bind(chat_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Set which of my documents a persona uses (the persona must be mine; checked by the caller).
pub async fn set_for_persona(pool: &SqlitePool, account_id: &str, persona_id: &str, ids: &[String]) -> CoreResult<()> {
    check_own(pool, account_id, ids).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM persona_documents WHERE persona_id = ?").bind(persona_id).execute(&mut *tx).await?;
    for id in ids {
        sqlx::query("INSERT OR IGNORE INTO persona_documents (persona_id, document_id) VALUES (?, ?)")
            .bind(persona_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ------------------------------------------------------------------ passages

/// Every passage of these (ready) documents, in order, with embeddings from `model`.
pub async fn chunks(pool: &SqlitePool, document_ids: &[String], model: Option<&str>) -> CoreResult<Vec<Chunk>> {
    let mut out = Vec::new();
    for id in document_ids {
        let rows: Vec<(String, i64, Option<i64>, String, Option<Vec<u8>>, Option<String>)> = sqlx::query_as(
            "SELECT d.name, c.idx, c.page, c.text, c.embedding, c.embedding_model
             FROM document_chunks c JOIN documents d ON d.id = c.document_id
             WHERE c.document_id = ? AND d.status = 'ready' ORDER BY c.idx",
        )
        .bind(id)
        .fetch_all(pool)
        .await?;
        for (name, idx, page, text, emb, emb_model) in rows {
            let embedding = match (model, emb_model.as_deref(), emb) {
                (Some(m), Some(em), Some(b)) if m == em => from_blob(&b),
                _ => None,
            };
            out.push(Chunk { document_id: id.clone(), document_name: name, idx, page, text, embedding });
        }
    }
    Ok(out)
}

pub async fn set_chunk_embedding(pool: &SqlitePool, document_id: &str, idx: i64, model: &str, v: &[f32]) -> CoreResult<()> {
    sqlx::query("UPDATE document_chunks SET embedding = ?, embedding_model = ? WHERE document_id = ? AND idx = ?")
        .bind(to_blob(&normalized(v)))
        .bind(model)
        .bind(document_id)
        .bind(idx)
        .execute(pool)
        .await?;
    Ok(())
}

/// The best `k` passages for a question: by embedding when the question and passages have
/// one, else by the question's words. In document order within each document.
pub fn best<'a>(chunks: &'a [Chunk], query: &str, query_embedding: Option<&[f32]>, k: usize) -> Vec<&'a Chunk> {
    let words = query_words(query);
    let mut scored: Vec<(f32, &Chunk)> = chunks
        .iter()
        .map(|c| {
            let by_words = word_score(&words, &c.text, chunks.len());
            let score = match (query_embedding, c.embedding.as_deref()) {
                // Mostly meaning, a little exact wording (names, numbers).
                (Some(q), Some(e)) => dot(&normalized(q), e) + 0.05 * by_words.min(4.0),
                _ => by_words,
            };
            (score, c)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut picked: Vec<&Chunk> = scored.into_iter().take(k).map(|(_, c)| c).collect();
    picked.sort_by(|a, b| a.document_name.cmp(&b.document_name).then(a.idx.cmp(&b.idx)));
    picked
}

const STOP: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "her", "was", "one", "our", "out", "has",
    "his", "how", "its", "may", "who", "did", "get", "use", "what", "when", "where", "which", "with", "this", "that",
    "from", "they", "have", "does", "about", "would", "there", "their", "into", "your", "tell", "please", "could", "should",
];

pub(crate) fn query_words(q: &str) -> Vec<String> {
    let mut w: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(&w.as_str()))
        .collect();
    w.sort();
    w.dedup();
    w
}

pub(crate) fn word_score(words: &[String], text: &str, _n: usize) -> f32 {
    if words.is_empty() {
        return 0.0;
    }
    let lower = text.to_lowercase();
    words
        .iter()
        .map(|w| {
            let hits = lower.matches(w.as_str()).count() as f32;
            // Diminishing returns, so one word repeated doesn't win alone.
            (1.0 + hits).ln()
        })
        .sum()
}

/// Save which passages a reply used.
pub async fn set_message_sources(pool: &SqlitePool, message_id: &str, sources: &[Source]) -> CoreResult<()> {
    if sources.is_empty() {
        return Ok(());
    }
    let json = serde_json::to_string(sources).map_err(|e| CoreError::Invalid(e.to_string()))?;
    sqlx::query("UPDATE messages SET sources = ? WHERE id = ?")
        .bind(json)
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The sources of a chat's messages, by message id.
pub async fn sources_for_chat(pool: &SqlitePool, chat_id: &str) -> CoreResult<std::collections::HashMap<String, Vec<Source>>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, sources FROM messages WHERE chat_id = ? AND sources IS NOT NULL")
            .bind(chat_id)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, s)| serde_json::from_str::<Vec<Source>>(&s).ok().map(|v| (id, v)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(name: &str, idx: i64, text: &str, e: Option<Vec<f32>>) -> Chunk {
        Chunk {
            document_id: name.into(),
            document_name: name.into(),
            idx,
            page: Some(idx as i64 + 1),
            text: text.into(),
            embedding: e.map(|v| normalized(&v)),
        }
    }

    #[test]
    fn kinds_by_name() {
        assert_eq!(kind_of("Handbook.PDF").unwrap().1, "pdf");
        assert_eq!(kind_of("notes.md").unwrap().1, "md");
        assert_eq!(kind_of("letter.docx").unwrap().1, "docx");
        assert!(kind_of("old.doc").is_none());
        assert!(kind_of("noext").is_none());
    }

    #[test]
    fn words_find_the_passage_without_embeddings() {
        let c = vec![
            chunk("a", 0, "The warranty covers parts for two years.", None),
            chunk("a", 1, "Returns are accepted within 30 days with a receipt.", None),
            chunk("a", 2, "Cleaning: wipe with a damp cloth.", None),
        ];
        let got = best(&c, "How long is the warranty?", None, 1);
        assert_eq!(got[0].idx, 0);
        let got = best(&c, "can I return it without a receipt", None, 1);
        assert_eq!(got[0].idx, 1);
    }

    #[test]
    fn embeddings_win_and_results_keep_document_order() {
        let c = vec![
            chunk("a", 0, "alpha", Some(vec![1.0, 0.0])),
            chunk("a", 1, "beta", Some(vec![0.0, 1.0])),
            chunk("a", 2, "gamma", Some(vec![0.7, 0.7])),
        ];
        let got = best(&c, "nothing in common", Some(&[0.0, 1.0]), 2);
        assert_eq!(got.iter().map(|c| c.idx).collect::<Vec<_>>(), vec![1, 2], "best two, in order");
    }
}
