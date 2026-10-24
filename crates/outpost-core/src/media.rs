//! The gallery: images and videos made in Create, private to each account and
//! stored under `<files>/<account_id>/gallery/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Media {
    pub id: String,
    pub account_id: String,
    pub job_id: Option<String>,
    /// `image` or `video`.
    pub kind: String,
    pub mime: String,
    pub bytes: i64,
    pub pack_id: Option<String>,
    pub prompt: Option<String>,
    pub seed: Option<i64>,
    pub created_at: i64,
}

const COLS: &str = "id, account_id, job_id, kind, mime, bytes, pack_id, prompt, seed, created_at";

/// What a new gallery item is.
pub struct NewMedia<'a> {
    pub account_id: &'a str,
    pub job_id: Option<&'a str>,
    pub pack_id: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub seed: Option<u64>,
    /// The file's name as ComfyUI saved it (for its extension).
    pub source_name: &'a str,
    pub data: &'a [u8],
}

/// Image and video types the gallery keeps, by extension.
pub fn kind_of(name: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => ("image", "image/png", "png"),
        "jpg" | "jpeg" => ("image", "image/jpeg", "jpg"),
        "webp" => ("image", "image/webp", "webp"),
        "gif" => ("image", "image/gif", "gif"),
        "mp4" => ("video", "video/mp4", "mp4"),
        "webm" => ("video", "video/webm", "webm"),
        _ => return None,
    })
}

pub async fn add(pool: &SqlitePool, files_dir: &Path, new: NewMedia<'_>) -> CoreResult<Media> {
    let (kind, mime, ext) = kind_of(new.source_name)
        .ok_or_else(|| CoreError::Invalid(format!("{} isn't an image or video.", new.source_name)))?;
    let id = uuid::Uuid::new_v4().to_string();
    let rel = format!("{}/gallery/{id}.{ext}", new.account_id);
    let full = files_dir.join(&rel);
    if let Some(dir) = full.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io)?;
    }
    tokio::fs::write(&full, new.data).await.map_err(io)?;
    let m = Media {
        id,
        account_id: new.account_id.into(),
        job_id: new.job_id.map(Into::into),
        kind: kind.into(),
        mime: mime.into(),
        bytes: new.data.len() as i64,
        pack_id: new.pack_id.map(Into::into),
        prompt: new.prompt.map(Into::into),
        seed: new.seed.map(|s| s as i64),
        created_at: now(),
    };
    let saved = sqlx::query(
        "INSERT INTO media (id, account_id, job_id, kind, mime, bytes, path, pack_id, prompt, seed, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.id)
    .bind(&m.account_id)
    .bind(&m.job_id)
    .bind(&m.kind)
    .bind(&m.mime)
    .bind(m.bytes)
    .bind(&rel)
    .bind(&m.pack_id)
    .bind(&m.prompt)
    .bind(m.seed)
    .bind(m.created_at)
    .execute(pool)
    .await;
    if let Err(e) = saved {
        let _ = tokio::fs::remove_file(&full).await;
        return Err(e.into());
    }
    Ok(m)
}

fn io(e: std::io::Error) -> CoreError {
    CoreError::Invalid(format!("Could not save to the gallery: {e}"))
}

/// My gallery, newest first. `before` (a created_at) pages back.
pub async fn list(pool: &SqlitePool, account_id: &str, limit: i64, before: Option<i64>) -> CoreResult<Vec<Media>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM media WHERE account_id = ? AND created_at < ? ORDER BY created_at DESC, rowid DESC LIMIT ?"
    ))
    .bind(account_id)
    .bind(before.unwrap_or(i64::MAX))
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?)
}

/// The items a job made.
pub async fn for_job(pool: &SqlitePool, account_id: &str, job_id: &str) -> CoreResult<Vec<Media>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM media WHERE account_id = ? AND job_id = ? ORDER BY created_at, rowid"
    ))
    .bind(account_id)
    .bind(job_id)
    .fetch_all(pool)
    .await?)
}

/// One of my items and its file.
pub async fn get(pool: &SqlitePool, files_dir: &Path, account_id: &str, id: &str) -> CoreResult<(Media, PathBuf)> {
    let m: Media = sqlx::query_as(&format!("SELECT {COLS} FROM media WHERE id = ? AND account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)?;
    let rel: String = sqlx::query_scalar("SELECT path FROM media WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok((m, files_dir.join(rel)))
}

/// Delete one of my items and its file.
pub async fn delete(pool: &SqlitePool, files_dir: &Path, account_id: &str, id: &str) -> CoreResult<()> {
    let (_, path) = get(pool, files_dir, account_id, id).await?;
    sqlx::query("DELETE FROM media WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    let _ = tokio::fs::remove_file(path).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;

    #[tokio::test]
    async fn gallery_saves_lists_and_deletes_privately() {
        let pool = open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("outpost-media-{}", uuid::Uuid::new_v4()));
        let mk = |name: &'static str| NewAccount {
            name: name.into(),
            role: Role::Member,
            secret: Some("4821".into()),
        };
        let dan = accounts::create(&pool, mk("Dan")).await.unwrap().id;
        let eve = accounts::create(&pool, mk("Eve")).await.unwrap().id;

        let m = add(
            &pool,
            &dir,
            NewMedia {
                account_id: &dan,
                job_id: None,
                pack_id: Some("z_image_turbo"),
                prompt: Some("a lighthouse"),
                seed: Some(42),
                source_name: "outpost_00001_.png",
                data: b"\x89PNG fake",
            },
        )
        .await
        .unwrap();
        assert_eq!(m.kind, "image");
        assert_eq!(m.seed, Some(42));
        assert_eq!(list(&pool, &dan, 50, None).await.unwrap().len(), 1);
        assert!(list(&pool, &eve, 50, None).await.unwrap().is_empty());
        assert!(matches!(get(&pool, &dir, &eve, &m.id).await, Err(CoreError::NotFound)));
        let (_, path) = get(&pool, &dir, &dan, &m.id).await.unwrap();
        assert!(path.is_file());

        assert!(delete(&pool, &dir, &eve, &m.id).await.is_err());
        delete(&pool, &dir, &dan, &m.id).await.unwrap();
        assert!(!path.exists());
        assert!(list(&pool, &dan, 50, None).await.unwrap().is_empty());

        // Only images and videos.
        let bad = add(
            &pool,
            &dir,
            NewMedia {
                account_id: &dan,
                job_id: None,
                pack_id: None,
                prompt: None,
                seed: None,
                source_name: "notes.txt",
                data: b"x",
            },
        )
        .await;
        assert!(matches!(bad, Err(CoreError::Invalid(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of("a.PNG").unwrap().0, "image");
        assert_eq!(kind_of("clip.mp4").unwrap().1, "video/mp4");
        assert!(kind_of("x.exe").is_none());
        assert!(kind_of("noext").is_none());
    }
}
