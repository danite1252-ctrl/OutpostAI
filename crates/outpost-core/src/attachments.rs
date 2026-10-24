//! Images attached to chat messages. Each file is stored in its account's own folder
//! (`<files>/<account_id>/images/<id>.<ext>`) and recorded in the database. Only
//! real PNG, JPEG, WebP and GIF files are accepted, checked by their contents.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

/// Largest image accepted, after decoding.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Most images on one message.
pub const MAX_IMAGES_PER_MESSAGE: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Attachment {
    pub id: String,
    pub message_id: String,
    /// Always `image` for now.
    pub kind: String,
    pub mime: String,
    pub bytes: i64,
    pub created_at: i64,
}

/// An image checked and decoded, ready to save once its message exists.
#[derive(Debug, Clone)]
pub struct ImageUpload {
    pub data: Vec<u8>,
    pub mime: &'static str,
}

/// What kind of image this is, from its first bytes (never from its name).
pub fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "gif",
    }
}

/// Decode an image sent as base64 or a `data:image/...;base64,` URL and check it.
pub fn decode_image(encoded: &str) -> CoreResult<ImageUpload> {
    let raw = match encoded.split_once(";base64,") {
        Some((head, rest)) if head.starts_with("data:") => rest,
        _ => encoded,
    };
    let cleaned: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    // Base64 is 4/3 the size; refuse oversized input before decoding it.
    if cleaned.len() > MAX_IMAGE_BYTES / 3 * 4 + 8 {
        return Err(too_big());
    }
    let data = STANDARD
        .decode(cleaned.as_bytes())
        .map_err(|_| CoreError::Invalid("That image couldn't be read.".into()))?;
    if data.len() > MAX_IMAGE_BYTES {
        return Err(too_big());
    }
    let mime = sniff_image(&data)
        .ok_or_else(|| CoreError::Invalid("Only PNG, JPEG, WebP and GIF images can be attached.".into()))?;
    Ok(ImageUpload { data, mime })
}

fn too_big() -> CoreError {
    CoreError::Invalid(format!(
        "Images can be up to {} MB each.",
        MAX_IMAGE_BYTES / 1024 / 1024
    ))
}

/// Save a checked image for a message. `files_dir` is `<data>/files`.
pub async fn save_image(
    pool: &SqlitePool,
    files_dir: &Path,
    account_id: &str,
    message_id: &str,
    image: &ImageUpload,
) -> CoreResult<Attachment> {
    let id = uuid::Uuid::new_v4().to_string();
    let rel = format!("{account_id}/images/{id}.{}", extension(image.mime));
    let full = files_dir.join(&rel);
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io)?;
    }
    tokio::fs::write(&full, &image.data).await.map_err(io)?;
    let a = Attachment {
        id,
        message_id: message_id.to_string(),
        kind: "image".into(),
        mime: image.mime.to_string(),
        bytes: image.data.len() as i64,
        created_at: now(),
    };
    let saved = sqlx::query(
        "INSERT INTO attachments (id, account_id, message_id, kind, mime, bytes, path, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&a.id)
    .bind(account_id)
    .bind(&a.message_id)
    .bind(&a.kind)
    .bind(&a.mime)
    .bind(a.bytes)
    .bind(&rel)
    .bind(a.created_at)
    .execute(pool)
    .await;
    if let Err(e) = saved {
        let _ = tokio::fs::remove_file(&full).await;
        return Err(e.into());
    }
    Ok(a)
}

fn io(e: std::io::Error) -> CoreError {
    CoreError::Invalid(format!("Could not store the image: {e}"))
}

/// Attachments for every message in a chat, keyed by message id.
pub async fn for_chat(pool: &SqlitePool, chat_id: &str) -> CoreResult<HashMap<String, Vec<Attachment>>> {
    let rows: Vec<Attachment> = sqlx::query_as(
        "SELECT a.id, a.message_id, a.kind, a.mime, a.bytes, a.created_at
         FROM attachments a JOIN messages m ON m.id = a.message_id
         WHERE m.chat_id = ? ORDER BY a.created_at, a.id",
    )
    .bind(chat_id)
    .fetch_all(pool)
    .await?;
    let mut map: HashMap<String, Vec<Attachment>> = HashMap::new();
    for a in rows {
        map.entry(a.message_id.clone()).or_default().push(a);
    }
    Ok(map)
}

/// One of my attachments and where its file is.
pub async fn get(pool: &SqlitePool, files_dir: &Path, account_id: &str, id: &str) -> CoreResult<(Attachment, PathBuf)> {
    let row: Option<(String, String, String, String, i64, i64, String)> = sqlx::query_as(
        "SELECT id, message_id, kind, mime, bytes, created_at, path FROM attachments WHERE id = ? AND account_id = ?",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    let (id, message_id, kind, mime, bytes, created_at, path) = row.ok_or(CoreError::NotFound)?;
    Ok((
        Attachment {
            id,
            message_id,
            kind,
            mime,
            bytes,
            created_at,
        },
        files_dir.join(path),
    ))
}

/// Read an attachment's file.
pub async fn read(path: &Path) -> CoreResult<Vec<u8>> {
    tokio::fs::read(path).await.map_err(|_| CoreError::NotFound)
}

/// File paths for a chat's attachments, to delete after the chat is gone.
pub async fn paths_for_chat(pool: &SqlitePool, files_dir: &Path, chat_id: &str) -> CoreResult<Vec<PathBuf>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT a.path FROM attachments a JOIN messages m ON m.id = a.message_id WHERE m.chat_id = ?")
            .bind(chat_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(p,)| files_dir.join(p)).collect())
}

/// File paths for one message's attachments.
pub async fn paths_for_message(pool: &SqlitePool, files_dir: &Path, message_id: &str) -> CoreResult<Vec<PathBuf>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM attachments WHERE message_id = ?")
        .bind(message_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(p,)| files_dir.join(p)).collect())
}

/// Keep an image given to a Create job (e.g. the picture to edit) until the job runs.
/// Returns its path relative to `files_dir`: `<account_id>/inputs/<id>.<ext>`.
pub async fn save_job_input(files_dir: &Path, account_id: &str, image: &ImageUpload) -> CoreResult<String> {
    let rel = format!("{account_id}/inputs/{}.{}", uuid::Uuid::new_v4(), extension(image.mime));
    let full = files_dir.join(&rel);
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io)?;
    }
    tokio::fs::write(&full, &image.data).await.map_err(io)?;
    Ok(rel)
}

/// Read back a job input saved by [`save_job_input`], only from that account's inputs.
pub async fn read_job_input(files_dir: &Path, account_id: &str, rel: &str) -> CoreResult<Vec<u8>> {
    let prefix = format!("{account_id}/inputs/");
    let name = rel.strip_prefix(&prefix).ok_or(CoreError::NotFound)?;
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(CoreError::NotFound);
    }
    tokio::fs::read(files_dir.join(rel))
        .await
        .map_err(|_| CoreError::NotFound)
}

/// Delete files; missing ones are fine.
pub async fn remove_files(paths: &[PathBuf]) {
    for p in paths {
        if let Err(e) = tokio::fs::remove_file(p).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %p.display(), error = %e, "could not delete attachment file");
            }
        }
    }
}

/// Delete an account's whole files folder (after the account itself is deleted).
pub async fn remove_account_files(files_dir: &Path, account_id: &str) {
    // Account ids are UUIDs; never let an odd id point outside the files folder.
    if account_id.is_empty() || !account_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return;
    }
    let dir = files_dir.join(account_id);
    if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %dir.display(), error = %e, "could not delete account files");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::chats;
    use crate::db::open_in_memory;

    /// A real 1×1 PNG.
    pub const TINY_PNG: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    #[test]
    fn decodes_and_sniffs() {
        let img = decode_image(TINY_PNG).unwrap();
        assert_eq!(img.mime, "image/png");
        let url = decode_image(&format!("data:image/png;base64,{TINY_PNG}")).unwrap();
        assert_eq!(url.data, img.data);
        // Not an image, whatever it claims.
        let text = STANDARD.encode(b"hello, this is not a picture");
        assert!(matches!(decode_image(&text), Err(CoreError::Invalid(_))));
        assert!(matches!(decode_image("%%%not base64%%%"), Err(CoreError::Invalid(_))));
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    }

    #[tokio::test]
    async fn job_inputs_stay_in_their_account() {
        let dir = std::env::temp_dir().join(format!("outpost-inputs-{}", uuid::Uuid::new_v4()));
        let img = decode_image(TINY_PNG).unwrap();
        let rel = save_job_input(&dir, "dan", &img).await.unwrap();
        assert!(rel.starts_with("dan/inputs/") && rel.ends_with(".png"));
        assert_eq!(read_job_input(&dir, "dan", &rel).await.unwrap(), img.data);
        assert!(read_job_input(&dir, "eve", &rel).await.is_err());
        assert!(read_job_input(&dir, "dan", "dan/inputs/../../etc/passwd")
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_huge_images() {
        let big = "A".repeat(MAX_IMAGE_BYTES / 3 * 4 + 100);
        assert!(matches!(decode_image(&big), Err(CoreError::Invalid(_))));
    }

    #[tokio::test]
    async fn save_list_get_and_clean_up() {
        let pool = open_in_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("outpost-attach-{}", uuid::Uuid::new_v4()));
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
        let eve = accounts::create(
            &pool,
            NewAccount {
                name: "Eve".into(),
                role: Role::Member,
                secret: Some("1357".into()),
            },
        )
        .await
        .unwrap()
        .id;
        let chat = chats::create(&pool, &dan, None, None).await.unwrap();
        let msg = chats::add_message(&pool, &chat.id, "user", "What is this?", None, None)
            .await
            .unwrap();

        let img = decode_image(TINY_PNG).unwrap();
        let a = save_image(&pool, &dir, &dan, &msg.id, &img).await.unwrap();
        assert_eq!(a.mime, "image/png");

        let map = for_chat(&pool, &chat.id).await.unwrap();
        assert_eq!(map[&msg.id].len(), 1);
        let listed = chats::messages(&pool, &chat.id).await.unwrap();
        assert_eq!(listed[0].attachments.len(), 1);

        let (_, path) = get(&pool, &dir, &dan, &a.id).await.unwrap();
        assert_eq!(read(&path).await.unwrap(), img.data);
        // Someone else's image is "not found".
        assert!(matches!(get(&pool, &dir, &eve, &a.id).await, Err(CoreError::NotFound)));

        let paths = paths_for_chat(&pool, &dir, &chat.id).await.unwrap();
        chats::delete(&pool, &dan, &chat.id).await.unwrap();
        remove_files(&paths).await;
        assert!(!path.exists());
        assert!(for_chat(&pool, &chat.id).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
