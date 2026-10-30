//! Gallery search: a description of each picture (written by a vision model, in the
//! background) and an embedding of its prompt and description, so pictures can be found by
//! what's in them. Searches only ever look in one account's own gallery.

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use crate::documents::{query_words, word_score};
use crate::media::Media;
use crate::memory::{dot, from_blob, normalized, to_blob};
use crate::CoreResult;

/// Tries before a picture is left undescribed.
pub const MAX_TRIES: i64 = 3;

const MEDIA_COLS: &str =
    "m.id, m.account_id, m.job_id, m.kind, m.mime, m.bytes, m.pack_id, m.prompt, m.seed, m.created_at, m.shared_at";

/// The next picture to describe (newest first), and its file.
pub async fn next_to_describe(pool: &SqlitePool, files_dir: &Path) -> CoreResult<Option<(Media, PathBuf)>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT m.path FROM media m LEFT JOIN media_index i ON i.media_id = m.id
         WHERE m.kind = 'image' AND (i.media_id IS NULL OR (i.description IS NULL AND i.tries < ?))
         ORDER BY m.created_at DESC LIMIT 1",
    )
    .bind(MAX_TRIES)
    .fetch_optional(pool)
    .await?;
    let Some((rel,)) = row else { return Ok(None) };
    let m: Media = sqlx::query_as(&format!("SELECT {MEDIA_COLS} FROM media m WHERE m.path = ?"))
        .bind(&rel)
        .fetch_one(pool)
        .await?;
    Ok(Some((m, files_dir.join(rel))))
}

pub async fn set_description(pool: &SqlitePool, media_id: &str, model: &str, text: &str) -> CoreResult<()> {
    sqlx::query(
        "INSERT INTO media_index (media_id, description, model, described_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(media_id) DO UPDATE SET description = excluded.description, model = excluded.model,
           described_at = excluded.described_at, error = NULL, embedding = NULL, embedding_model = NULL",
    )
    .bind(media_id)
    .bind(text)
    .bind(model)
    .bind(crate::now())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_failed(pool: &SqlitePool, media_id: &str, error: &str) -> CoreResult<()> {
    sqlx::query(
        "INSERT INTO media_index (media_id, tries, error) VALUES (?, 1, ?)
         ON CONFLICT(media_id) DO UPDATE SET tries = tries + 1, error = excluded.error",
    )
    .bind(media_id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// The text an item is found by: its prompt and what the vision model saw.
pub fn search_text(prompt: Option<&str>, description: Option<&str>) -> String {
    [prompt, description].into_iter().flatten().filter(|t| !t.trim().is_empty()).collect::<Vec<_>>().join("\n")
}

/// Items (any account) with text but no embedding from `model` yet: (id, text).
pub async fn needing_embedding(pool: &SqlitePool, model: &str, limit: i64) -> CoreResult<Vec<(String, String)>> {
    let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT m.id, m.prompt, i.description FROM media m LEFT JOIN media_index i ON i.media_id = m.id
         WHERE (i.embedding_model IS NULL OR i.embedding_model != ?)
           AND (m.prompt IS NOT NULL OR i.description IS NOT NULL)
           AND (m.kind = 'video' OR i.description IS NOT NULL OR COALESCE(i.tries, 0) >= ?)
         ORDER BY m.created_at DESC LIMIT ?",
    )
    .bind(model)
    .bind(MAX_TRIES)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, p, d)| (id, search_text(p.as_deref(), d.as_deref())))
        .filter(|(_, t)| !t.is_empty())
        .collect())
}

pub async fn set_embedding(pool: &SqlitePool, media_id: &str, model: &str, v: &[f32]) -> CoreResult<()> {
    sqlx::query(
        "INSERT INTO media_index (media_id, embedding, embedding_model) VALUES (?, ?, ?)
         ON CONFLICT(media_id) DO UPDATE SET embedding = excluded.embedding, embedding_model = excluded.embedding_model",
    )
    .bind(media_id)
    .bind(to_blob(&normalized(v)))
    .bind(model)
    .execute(pool)
    .await?;
    Ok(())
}

/// A gallery item as search sees it.
#[derive(Debug, Clone)]
pub struct Indexed {
    pub media: Media,
    pub description: Option<String>,
    pub embedding: Option<Vec<f32>>,
}

/// Everything in one account's gallery, with descriptions and embeddings from `model`.
pub async fn all(pool: &SqlitePool, account_id: &str, model: Option<&str>) -> CoreResult<Vec<Indexed>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        media: Media,
        description: Option<String>,
        embedding: Option<Vec<u8>>,
        embedding_model: Option<String>,
    }
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {MEDIA_COLS}, i.description, i.embedding, i.embedding_model
         FROM media m LEFT JOIN media_index i ON i.media_id = m.id
         WHERE m.account_id = ? ORDER BY m.created_at DESC"
    ))
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Indexed {
            embedding: match (model, r.embedding_model.as_deref(), r.embedding) {
                (Some(m), Some(em), Some(b)) if m == em => from_blob(&b),
                _ => None,
            },
            description: r.description,
            media: r.media,
        })
        .collect())
}

/// The best matches for `query`, best first: by meaning where there are embeddings, by
/// words otherwise. Weak matches are left out.
pub fn search<'a>(items: &'a [Indexed], query: &str, query_embedding: Option<&[f32]>, limit: usize) -> Vec<(f32, &'a Indexed)> {
    let words = query_words(query);
    let q = query_embedding.map(normalized);
    let mut scored: Vec<(f32, &Indexed)> = items
        .iter()
        .map(|it| {
            let text = search_text(it.media.prompt.as_deref(), it.description.as_deref());
            let by_words = word_score(&words, &text, items.len());
            let score = match (&q, &it.embedding) {
                (Some(q), Some(e)) => dot(q, e) + 0.03 * by_words.min(4.0),
                // No embedding (yet): words only, scaled to sit among meaning scores.
                _ if by_words > 0.0 => 0.25 + 0.08 * by_words.min(5.0),
                _ => 0.0,
            };
            (score, it)
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let best = scored.first().map(|(s, _)| *s).unwrap_or(0.0);
    // Keep what's close to the best match, and nothing that's barely related.
    let floor = if q.is_some() { (best - 0.15).max(0.3) } else { 0.0 };
    scored.into_iter().filter(|(s, _)| *s >= floor).take(limit).collect()
}

/// (pictures described, pictures in all galleries, pictures still to try).
pub async fn progress(pool: &SqlitePool) -> CoreResult<(i64, i64, i64)> {
    let (described, total, waiting): (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(i.description IS NOT NULL), 0), COUNT(*),
                COALESCE(SUM(i.description IS NULL AND COALESCE(i.tries, 0) < ?), 0)
         FROM media m LEFT JOIN media_index i ON i.media_id = m.id WHERE m.kind = 'image'",
    )
    .bind(MAX_TRIES)
    .fetch_one(pool)
    .await?;
    Ok((described, total, waiting))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, prompt: Option<&str>, description: Option<&str>, e: Option<Vec<f32>>) -> Indexed {
        Indexed {
            media: Media {
                id: id.into(),
                account_id: "a".into(),
                job_id: None,
                kind: "image".into(),
                mime: "image/png".into(),
                bytes: 1,
                pack_id: None,
                prompt: prompt.map(Into::into),
                seed: None,
                created_at: 0,
                shared_at: None,
            },
            description: description.map(Into::into),
            embedding: e.map(|v| normalized(&v)),
        }
    }

    #[test]
    fn words_find_by_prompt_and_description() {
        let items = vec![
            item("a", Some("golden retriever on a shoreline"), Some("A dog running along a sandy beach."), None),
            item("b", Some("city at night"), None, None),
            item("c", None, Some("A red bicycle against a brick wall."), None),
        ];
        let found = search(&items, "dog on the beach", None, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.media.id, "a");
        assert_eq!(search(&items, "bicycle", None, 10)[0].1.media.id, "c");
        assert!(search(&items, "spaceship", None, 10).is_empty());
    }

    #[test]
    fn meaning_ranks_and_weak_matches_drop() {
        let items = vec![
            item("dog", None, Some("dog"), Some(vec![1.0, 0.0, 0.0])),
            item("puppy", None, Some("puppy"), Some(vec![0.9, 0.3, 0.0])),
            item("car", None, Some("car"), Some(vec![0.0, 0.0, 1.0])),
        ];
        let found: Vec<&str> = search(&items, "hound", Some(&[1.0, 0.1, 0.0]), 10).iter().map(|(_, i)| i.media.id.as_str()).collect();
        assert_eq!(found, vec!["dog", "puppy"]);
    }
}
