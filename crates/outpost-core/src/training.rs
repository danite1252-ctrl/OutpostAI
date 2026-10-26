//! Training Studio records: datasets of pictures with captions, the trainings that
//! turn them into LoRAs, and the characters people keep from them.
//!
//! Files live under `<data>/training` (the `root` passed in here):
//! `datasets/<dataset_id>/<image_id>.<ext>` and `runs/<training_id>/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::attachments::sniff_image;
use crate::{now, CoreError, CoreResult};

/// Largest picture accepted into a dataset (phone photos run large).
pub const MAX_DATASET_IMAGE_BYTES: usize = 40 * 1024 * 1024;
/// Most pictures in one dataset. 15 to 40 is plenty for a character.
pub const MAX_DATASET_IMAGES: i64 = 200;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Dataset {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub trigger_word: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub image_count: i64,
    /// Pictures with a caption written.
    pub captioned_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct DatasetImage {
    pub id: String,
    pub dataset_id: String,
    #[serde(skip)]
    pub file: String,
    pub mime: String,
    pub bytes: i64,
    /// What's in it, without the trigger word.
    pub caption: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Training {
    pub id: String,
    pub account_id: String,
    pub dataset_id: String,
    pub job_id: String,
    pub trigger_word: String,
    pub steps: i64,
    pub image_count: i64,
    pub created_at: i64,
    /// The test prompts, as JSON; read them with [`Training::prompts`].
    #[serde(skip)]
    pub sample_prompts: Option<String>,
}

impl Training {
    /// The test prompts this training draws at each checkpoint (empty for old ones).
    pub fn prompts(&self) -> Vec<String> {
        self.sample_prompts
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Character {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub trigger_word: String,
    /// File name in the LoRA folder.
    pub lora_file: String,
    pub training_id: Option<String>,
    pub step: Option<i64>,
    pub created_at: i64,
    /// What it is, from the captions: "woman", "dog", "fox character".
    pub subject: Option<String>,
    /// How it's drawn, from the captions: "realistic photography", "cartoon style".
    pub style: Option<String>,
}

const DATASET_COLS: &str = "d.id, d.account_id, d.name, d.trigger_word, d.created_at, d.updated_at,
    (SELECT COUNT(*) FROM dataset_images i WHERE i.dataset_id = d.id) AS image_count,
    (SELECT COUNT(*) FROM dataset_images i WHERE i.dataset_id = d.id AND i.caption != '') AS captioned_count";

fn io(e: std::io::Error) -> CoreError {
    CoreError::Invalid(format!("Could not store the file: {e}"))
}

fn clean_name(name: &str) -> CoreResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(CoreError::Invalid("Give the character a name of up to 60 characters.".into()));
    }
    Ok(name.to_string())
}

/// Trigger words must be plain: they become a file name and go into every prompt.
/// (Whether the word is *wise* is [`crate::trigger::check`]'s job; that only warns.)
fn clean_trigger(word: &str) -> CoreResult<String> {
    let w = word.trim().to_lowercase();
    if w.len() < 3 || w.len() > 24 || !w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return Err(CoreError::Invalid(
            "The trigger word must be 3 to 24 letters and numbers, no spaces.".into(),
        ));
    }
    Ok(w)
}

/// A LoRA brought in from elsewhere was trained with whatever word its maker chose:
/// letters, numbers, `_` and spaces are fine (it only has to appear in prompts).
fn clean_any_trigger(word: &str) -> CoreResult<String> {
    let w = word.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let ok = (2..=40).contains(&w.chars().count())
        && w.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ' ' || c == '-');
    if !ok {
        return Err(CoreError::Invalid(
            "The trigger word must be 2 to 40 letters, numbers, spaces, - or _.".into(),
        ));
    }
    Ok(w)
}

pub fn dataset_dir(root: &Path, dataset_id: &str) -> PathBuf {
    root.join("datasets").join(dataset_id)
}

pub fn run_dir(root: &Path, training_id: &str) -> PathBuf {
    root.join("runs").join(training_id)
}

// ---------------------------------------------------------------- datasets

pub async fn create_dataset(pool: &SqlitePool, account_id: &str, name: &str, trigger: &str) -> CoreResult<Dataset> {
    let name = clean_name(name)?;
    let trigger = clean_trigger(trigger)?;
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    sqlx::query(
        "INSERT INTO datasets (id, account_id, name, trigger_word, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(account_id)
    .bind(&name)
    .bind(&trigger)
    .bind(t)
    .bind(t)
    .execute(pool)
    .await?;
    get_dataset(pool, account_id, &id).await
}

pub async fn list_datasets(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Dataset>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {DATASET_COLS} FROM datasets d WHERE d.account_id = ? ORDER BY d.updated_at DESC, d.rowid DESC"
    ))
    .bind(account_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_dataset(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Dataset> {
    sqlx::query_as(&format!("SELECT {DATASET_COLS} FROM datasets d WHERE d.id = ? AND d.account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

pub async fn update_dataset(
    pool: &SqlitePool,
    account_id: &str,
    id: &str,
    name: Option<&str>,
    trigger: Option<&str>,
) -> CoreResult<Dataset> {
    let d = get_dataset(pool, account_id, id).await?;
    let name = match name {
        Some(n) => clean_name(n)?,
        None => d.name,
    };
    let trigger = match trigger {
        Some(t) => clean_trigger(t)?,
        None => d.trigger_word,
    };
    sqlx::query("UPDATE datasets SET name = ?, trigger_word = ?, updated_at = ? WHERE id = ?")
        .bind(&name)
        .bind(&trigger)
        .bind(now())
        .bind(id)
        .execute(pool)
        .await?;
    get_dataset(pool, account_id, id).await
}

async fn touch(pool: &SqlitePool, id: &str) {
    let _ = sqlx::query("UPDATE datasets SET updated_at = ? WHERE id = ?")
        .bind(now())
        .bind(id)
        .execute(pool)
        .await;
}

/// Delete a dataset, its pictures and its training runs (characters made from it stay).
pub async fn delete_dataset(pool: &SqlitePool, root: &Path, account_id: &str, id: &str) -> CoreResult<()> {
    get_dataset(pool, account_id, id).await?;
    let runs: Vec<String> = sqlx::query_scalar("SELECT id FROM trainings WHERE dataset_id = ?")
        .bind(id)
        .fetch_all(pool)
        .await?;
    sqlx::query("DELETE FROM datasets WHERE id = ?").bind(id).execute(pool).await?;
    let _ = tokio::fs::remove_dir_all(dataset_dir(root, id)).await;
    for r in runs {
        let _ = tokio::fs::remove_dir_all(run_dir(root, &r)).await;
    }
    Ok(())
}

// ---------------------------------------------------------------- pictures

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "gif",
    }
}

/// Add a picture (its bytes are checked to be PNG, JPEG or WebP).
pub async fn add_image(
    pool: &SqlitePool,
    root: &Path,
    account_id: &str,
    dataset_id: &str,
    data: &[u8],
    caption: &str,
) -> CoreResult<DatasetImage> {
    let d = get_dataset(pool, account_id, dataset_id).await?;
    if d.image_count >= MAX_DATASET_IMAGES {
        return Err(CoreError::Conflict(format!(
            "A dataset can hold up to {MAX_DATASET_IMAGES} pictures."
        )));
    }
    if data.len() > MAX_DATASET_IMAGE_BYTES {
        return Err(CoreError::Invalid(format!(
            "Pictures can be up to {} MB each.",
            MAX_DATASET_IMAGE_BYTES / 1024 / 1024
        )));
    }
    let mime = sniff_image(data)
        .filter(|m| *m != "image/gif")
        .ok_or_else(|| CoreError::Invalid("Only PNG, JPEG and WebP pictures can be used for training.".into()))?;
    let id = uuid::Uuid::new_v4().to_string();
    let file = format!("{id}.{}", extension(mime));
    let dir = dataset_dir(root, dataset_id);
    tokio::fs::create_dir_all(&dir).await.map_err(io)?;
    tokio::fs::write(dir.join(&file), data).await.map_err(io)?;
    let img = DatasetImage {
        id,
        dataset_id: dataset_id.into(),
        file,
        mime: mime.into(),
        bytes: data.len() as i64,
        caption: tidy_caption(caption, &d.trigger_word),
        created_at: now(),
    };
    let saved = sqlx::query(
        "INSERT INTO dataset_images (id, dataset_id, file, mime, bytes, caption, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&img.id)
    .bind(&img.dataset_id)
    .bind(&img.file)
    .bind(&img.mime)
    .bind(img.bytes)
    .bind(&img.caption)
    .bind(img.created_at)
    .execute(pool)
    .await;
    if let Err(e) = saved {
        let _ = tokio::fs::remove_file(dir.join(&img.file)).await;
        return Err(e.into());
    }
    touch(pool, dataset_id).await;
    Ok(img)
}

/// One line, no trigger word at the front (it's added when training), no full stop.
pub fn tidy_caption(caption: &str, trigger: &str) -> String {
    let line = caption
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_matches(['"', '\''])
        .trim_end_matches('.')
        .trim();
    let lower = line.to_lowercase();
    let rest = if lower == trigger {
        ""
    } else if lower.starts_with(&format!("{trigger},")) || lower.starts_with(&format!("{trigger} ")) {
        line.get(trigger.len()..).unwrap_or(line).trim_start_matches([',', ' '])
    } else {
        line
    };
    rest.chars().take(1000).collect()
}

pub async fn images(pool: &SqlitePool, account_id: &str, dataset_id: &str) -> CoreResult<Vec<DatasetImage>> {
    get_dataset(pool, account_id, dataset_id).await?;
    Ok(sqlx::query_as(
        "SELECT id, dataset_id, file, mime, bytes, caption, created_at FROM dataset_images
         WHERE dataset_id = ? ORDER BY created_at, rowid",
    )
    .bind(dataset_id)
    .fetch_all(pool)
    .await?)
}

/// One picture and where its file is.
pub async fn image(
    pool: &SqlitePool,
    root: &Path,
    account_id: &str,
    dataset_id: &str,
    image_id: &str,
) -> CoreResult<(DatasetImage, PathBuf)> {
    get_dataset(pool, account_id, dataset_id).await?;
    let img: DatasetImage = sqlx::query_as(
        "SELECT id, dataset_id, file, mime, bytes, caption, created_at FROM dataset_images WHERE id = ? AND dataset_id = ?",
    )
    .bind(image_id)
    .bind(dataset_id)
    .fetch_optional(pool)
    .await?
    .ok_or(CoreError::NotFound)?;
    let path = dataset_dir(root, dataset_id).join(&img.file);
    Ok((img, path))
}

pub async fn set_caption(
    pool: &SqlitePool,
    account_id: &str,
    dataset_id: &str,
    image_id: &str,
    caption: &str,
) -> CoreResult<DatasetImage> {
    let d = get_dataset(pool, account_id, dataset_id).await?;
    let caption = tidy_caption(caption, &d.trigger_word);
    let changed = sqlx::query("UPDATE dataset_images SET caption = ? WHERE id = ? AND dataset_id = ?")
        .bind(&caption)
        .bind(image_id)
        .bind(dataset_id)
        .execute(pool)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(CoreError::NotFound);
    }
    touch(pool, dataset_id).await;
    Ok(sqlx::query_as("SELECT id, dataset_id, file, mime, bytes, caption, created_at FROM dataset_images WHERE id = ?")
        .bind(image_id)
        .fetch_one(pool)
        .await?)
}

pub async fn delete_image(pool: &SqlitePool, root: &Path, account_id: &str, dataset_id: &str, image_id: &str) -> CoreResult<()> {
    let (img, path) = image(pool, root, account_id, dataset_id, image_id).await?;
    sqlx::query("DELETE FROM dataset_images WHERE id = ?").bind(&img.id).execute(pool).await?;
    let _ = tokio::fs::remove_file(path).await;
    touch(pool, dataset_id).await;
    Ok(())
}

/// Copy every picture in a folder on this PC into the dataset, with the caption from
/// a `.txt` of the same name when there is one. Returns how many were added and
/// which files were skipped (and why).
pub async fn import_folder(
    pool: &SqlitePool,
    root: &Path,
    account_id: &str,
    dataset_id: &str,
    folder: &Path,
) -> CoreResult<(usize, Vec<String>)> {
    get_dataset(pool, account_id, dataset_id).await?;
    let mut entries = tokio::fs::read_dir(folder)
        .await
        .map_err(|e| CoreError::Invalid(format!("Couldn't open {}: {e}", folder.display())))?;
    let mut files = Vec::new();
    while let Ok(Some(e)) = entries.next_entry().await {
        let path = e.path();
        let ext = path
            .extension()
            .and_then(|x| x.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if path.is_file() && ["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(CoreError::Invalid(format!(
            "There are no PNG, JPEG or WebP pictures in {}.",
            folder.display()
        )));
    }
    let mut added = 0;
    let mut skipped = Vec::new();
    for path in files {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let data = match tokio::fs::read(&path).await {
            Ok(d) => d,
            Err(e) => {
                skipped.push(format!("{name}: {e}"));
                continue;
            }
        };
        let caption = tokio::fs::read_to_string(path.with_extension("txt")).await.unwrap_or_default();
        match add_image(pool, root, account_id, dataset_id, &data, &caption).await {
            Ok(_) => added += 1,
            Err(CoreError::Conflict(m)) => {
                skipped.push(format!("{name}: {m}"));
                break;
            }
            Err(e) => skipped.push(format!("{name}: {e}")),
        }
    }
    Ok((added, skipped))
}

/// Write `<trigger>, <caption>` beside every picture, the way the trainer reads them.
pub async fn write_captions(pool: &SqlitePool, root: &Path, account_id: &str, dataset_id: &str) -> CoreResult<usize> {
    let d = get_dataset(pool, account_id, dataset_id).await?;
    let dir = dataset_dir(root, dataset_id);
    let list = images(pool, account_id, dataset_id).await?;
    for img in &list {
        let text = if img.caption.is_empty() {
            d.trigger_word.clone()
        } else {
            format!("{}, {}", d.trigger_word, img.caption)
        };
        let txt = dir.join(&img.file).with_extension("txt");
        tokio::fs::write(txt, text).await.map_err(io)?;
    }
    Ok(list.len())
}

// ---------------------------------------------------------------- trainings

pub async fn add_training(
    pool: &SqlitePool,
    account_id: &str,
    dataset: &Dataset,
    job_id: &str,
    steps: i64,
    prompts: &[String],
) -> CoreResult<Training> {
    let t = Training {
        sample_prompts: Some(serde_json::to_string(prompts).unwrap_or_else(|_| "[]".into())),
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.into(),
        dataset_id: dataset.id.clone(),
        job_id: job_id.into(),
        trigger_word: dataset.trigger_word.clone(),
        steps,
        image_count: dataset.image_count,
        created_at: now(),
    };
    sqlx::query(
        "INSERT INTO trainings (id, account_id, dataset_id, job_id, trigger_word, steps, image_count, created_at, sample_prompts)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&t.id)
    .bind(&t.account_id)
    .bind(&t.dataset_id)
    .bind(&t.job_id)
    .bind(&t.trigger_word)
    .bind(t.steps)
    .bind(t.image_count)
    .bind(t.created_at)
    .bind(&t.sample_prompts)
    .execute(pool)
    .await?;
    Ok(t)
}

const TRAINING_COLS: &str =
    "id, account_id, dataset_id, job_id, trigger_word, steps, image_count, created_at, sample_prompts";

/// Words that say what kind of picture it is. A caption keyword containing one of
/// these is the dataset's style, kept in every test prompt.
const STYLE_WORDS: &[&str] = &[
    "photo", "photograph", "realistic", "anime", "manga", "cartoon", "illustration", "drawing", "painting",
    "watercolor", "watercolour", "sketch", "3d", "render", "cgi", "pixel art", "digital art", "comic", "chibi",
    "cel shaded", "oil painting", "concept art", "clay", "lineart", "line art", "vector",
];

/// Most common item, ties to the one seen first.
fn most_common(items: impl Iterator<Item = String>) -> Option<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for item in items {
        match counts.iter_mut().find(|(k, _)| *k == item) {
            Some((_, n)) => *n += 1,
            None => counts.push((item, 1)),
        }
    }
    let best = counts.iter().map(|(_, n)| *n).max()?;
    counts.into_iter().find(|(_, n)| *n == best).map(|(k, _)| k)
}

/// Test prompts that suit the dataset, from its captions: what the subject is (the
/// usual first keyword: "woman", "dog", "anime girl") and how it's drawn ("realistic
/// photo", "cartoon style"), in three new scenes. Captions are keyword lists, most
/// important first, as Outpost's captioner writes them.
pub fn suggest_prompts(trigger: &str, captions: &[String]) -> Vec<String> {
    let (subject, style) = subject_and_style(trigger, captions);
    let style = style.unwrap_or_else(|| "photo".into());
    let who = match &subject {
        Some(s) => format!("{trigger}, {s}"),
        None => trigger.to_string(),
    };
    vec![
        format!("{who}, close-up portrait, looking at the camera, soft window light, {style}"),
        format!("{who}, on a snowy city street at dusk, full body, {style}"),
        format!("{who}, on a beach at sunset, relaxed, {style}"),
    ]
}

/// What the pictures show (the usual first keyword: "woman", "dog") and how they're
/// drawn ("realistic photography", "cartoon style"), from the captions.
pub fn subject_and_style(trigger: &str, captions: &[String]) -> (Option<String>, Option<String>) {
    let lists: Vec<Vec<String>> = captions
        .iter()
        .map(|c| {
            c.split(',')
                .map(|k| k.trim().trim_end_matches('.').to_lowercase())
                .filter(|k| !k.is_empty() && k != trigger && k.chars().count() <= 40)
                .collect::<Vec<_>>()
        })
        .filter(|l| !l.is_empty())
        .collect();
    let is_style = |k: &str| STYLE_WORDS.iter().any(|w| k.contains(w));
    let subject = most_common(lists.iter().filter_map(|l| l.iter().find(|k| !is_style(k)).cloned()));
    let style = most_common(lists.iter().flat_map(|l| l.iter().filter(|k| is_style(k)).cloned()));
    (subject, style)
}

/// Tidy test prompts someone wrote: 1 to 4, one line each, each starting with the
/// trigger word (added if it's missing).
pub fn clean_prompts(trigger: &str, prompts: &[String]) -> CoreResult<Vec<String>> {
    let out: Vec<String> = prompts
        .iter()
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .map(|p| {
            let has = p.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| w == trigger);
            if has { p } else { format!("{trigger}, {p}") }
        })
        .collect();
    if out.is_empty() || out.len() > 4 || out.iter().any(|p| p.chars().count() > 500) {
        return Err(CoreError::Invalid("Give 1 to 4 test prompts of up to 500 characters each.".into()));
    }
    Ok(out)
}

/// A dataset's trainings, newest first.
pub async fn trainings(pool: &SqlitePool, account_id: &str, dataset_id: &str) -> CoreResult<Vec<Training>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {TRAINING_COLS} FROM trainings WHERE account_id = ? AND dataset_id = ? ORDER BY created_at DESC, rowid DESC"
    ))
    .bind(account_id)
    .bind(dataset_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_training(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Training> {
    sqlx::query_as(&format!("SELECT {TRAINING_COLS} FROM trainings WHERE id = ? AND account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

/// The training a queued job runs, for the worker.
pub async fn training_for_job(pool: &SqlitePool, job_id: &str) -> CoreResult<Training> {
    sqlx::query_as(&format!("SELECT {TRAINING_COLS} FROM trainings WHERE job_id = ?"))
        .bind(job_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

// ---------------------------------------------------------------- characters

const CHARACTER_COLS: &str = "id, account_id, name, trigger_word, lora_file, training_id, step, created_at, subject, style";

/// My characters (the owner can ask for everyone's).
pub async fn characters(pool: &SqlitePool, account_id: Option<&str>) -> CoreResult<Vec<Character>> {
    let q = match account_id {
        Some(_) => format!("SELECT {CHARACTER_COLS} FROM characters WHERE account_id = ? ORDER BY name COLLATE NOCASE"),
        None => format!("SELECT {CHARACTER_COLS} FROM characters ORDER BY name COLLATE NOCASE"),
    };
    let mut query = sqlx::query_as(&q);
    if let Some(a) = account_id {
        query = query.bind(a);
    }
    Ok(query.fetch_all(pool).await?)
}

/// The character using this LoRA file, whoever's it is.
pub async fn character_by_lora(pool: &SqlitePool, lora_file: &str) -> CoreResult<Option<Character>> {
    Ok(sqlx::query_as(&format!("SELECT {CHARACTER_COLS} FROM characters WHERE lora_file = ?"))
        .bind(lora_file)
        .fetch_optional(pool)
        .await?)
}

pub struct NewCharacter<'a> {
    pub name: &'a str,
    pub trigger_word: &'a str,
    pub lora_file: &'a str,
    pub training_id: Option<&'a str>,
    pub step: Option<i64>,
    pub subject: Option<&'a str>,
    pub style: Option<&'a str>,
}

/// Save a character, or update mine that already uses this LoRA file. Someone else's
/// file is refused.
pub async fn save_character(pool: &SqlitePool, account_id: &str, c: NewCharacter<'_>) -> CoreResult<Character> {
    let name = clean_name(c.name)?;
    let trigger = clean_any_trigger(c.trigger_word)?;
    if let Some(existing) = character_by_lora(pool, c.lora_file).await? {
        if existing.account_id != account_id {
            return Err(CoreError::Conflict(format!(
                "{} is already someone else's character. Pick a different trigger word.",
                c.lora_file
            )));
        }
        sqlx::query(
            "UPDATE characters SET name = ?, trigger_word = ?, training_id = ?, step = ?, subject = ?, style = ? WHERE id = ?",
        )
            .bind(&name)
            .bind(&trigger)
            .bind(c.training_id)
            .bind(c.step)
            .bind(c.subject)
            .bind(c.style)
            .bind(&existing.id)
            .execute(pool)
            .await?;
        return get_character(pool, account_id, &existing.id).await;
    }
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO characters (id, account_id, name, trigger_word, lora_file, training_id, step, created_at, subject, style)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(account_id)
    .bind(&name)
    .bind(&trigger)
    .bind(c.lora_file)
    .bind(c.training_id)
    .bind(c.step)
    .bind(now())
    .bind(c.subject)
    .bind(c.style)
    .execute(pool)
    .await?;
    get_character(pool, account_id, &id).await
}

pub async fn get_character(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Character> {
    sqlx::query_as(&format!("SELECT {CHARACTER_COLS} FROM characters WHERE id = ? AND account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

/// Rename a character (its trigger word and LoRA stay).
pub async fn rename_character(pool: &SqlitePool, account_id: &str, id: &str, name: &str) -> CoreResult<Character> {
    let name = clean_name(name)?;
    get_character(pool, account_id, id).await?;
    sqlx::query("UPDATE characters SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(id)
        .execute(pool)
        .await?;
    get_character(pool, account_id, id).await
}

/// Forget a character. Returns it so the caller can remove its LoRA file.
pub async fn delete_character(pool: &SqlitePool, account_id: Option<&str>, id: &str) -> CoreResult<Character> {
    let c: Character = sqlx::query_as(&format!("SELECT {CHARACTER_COLS} FROM characters WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)?;
    if account_id.is_some_and(|a| a != c.account_id) {
        return Err(CoreError::NotFound);
    }
    sqlx::query("DELETE FROM characters WHERE id = ?").bind(id).execute(pool).await?;
    Ok(c)
}

/// The character a chat message is about: its trigger word or its name appears as a
/// whole word. Longer names win ("Kat Smith" over "Kat").
pub fn mentioned<'a>(text: &str, characters: &'a [Character]) -> Option<&'a Character> {
    let lower = text.to_lowercase();
    let has_word = |needle: &str| {
        let needle = needle.to_lowercase();
        if needle.is_empty() {
            return false;
        }
        lower.match_indices(&needle).any(|(i, _)| {
            let before = lower[..i].chars().next_back();
            let after = lower[i + needle.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    };
    let mut best: Option<(&Character, usize)> = None;
    for c in characters {
        for key in [&c.trigger_word, &c.name] {
            if has_word(key) && best.map_or(true, |(_, len)| key.len() > len) {
                best = Some((c, key.len()));
            }
        }
    }
    best.map(|(c, _)| c)
}

/// The prompt for a character from a chat message: its name becomes its trigger word
/// and what it is ("Kat at the beach" → "zk4t woman at the beach, realistic
/// photography"). A trigger word alone means nothing to the model, which then guesses
/// ("zk4t" came out as a car), so the subject goes beside it and the style at the end,
/// unless the message already says them.
pub fn with_trigger(text: &str, c: &Character) -> String {
    let lower = text.to_lowercase();
    let subject = c.subject.as_deref().filter(|s| !s.is_empty() && !lower.contains(&s.to_lowercase()));
    let who = match subject {
        Some(s) => format!("{} {s}", c.trigger_word),
        None => c.trigger_word.clone(),
    };
    let mut out = None;
    // The trigger word or the name, as a whole word: swap it for trigger + subject.
    for key in [c.trigger_word.as_str(), c.name.as_str()] {
        let k = key.to_lowercase();
        if k.is_empty() {
            continue;
        }
        let found = lower.match_indices(&k).find(|(i, _)| {
            let before = lower[..*i].chars().next_back();
            let after = lower[i + k.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        });
        if let Some((i, _)) = found {
            // Lower-casing kept the length (plain names); otherwise fall back below.
            if text.is_char_boundary(i) && text.is_char_boundary(i + k.len()) && text[i..i + k.len()].to_lowercase() == k {
                out = Some(format!("{}{}{}", &text[..i], who, &text[i + k.len()..]));
                break;
            }
        }
    }
    let mut out = out.unwrap_or_else(|| format!("{who}, {text}"));
    if let Some(style) = c.style.as_deref().filter(|s| !s.is_empty() && !lower.contains(&s.to_lowercase())) {
        out = format!("{}, {style}", out.trim_end_matches(['.', ' ', ',']));
    }
    out
}

/// Fill in a character's subject and style from its training's captions, for ones
/// made before Outpost kept them. Saves them so it's done once.
pub async fn fill_subject(pool: &SqlitePool, c: &mut Character) -> CoreResult<()> {
    if c.subject.is_some() || c.style.is_some() {
        return Ok(());
    }
    let Some(training_id) = c.training_id.clone() else { return Ok(()) };
    let Ok(t) = get_training(pool, &c.account_id, &training_id).await else { return Ok(()) };
    let captions: Vec<String> = sqlx::query_scalar("SELECT caption FROM dataset_images WHERE dataset_id = ?")
        .bind(&t.dataset_id)
        .fetch_all(pool)
        .await?;
    let (subject, style) = subject_and_style(&c.trigger_word, &captions);
    sqlx::query("UPDATE characters SET subject = ?, style = ? WHERE id = ?")
        .bind(&subject)
        .bind(&style)
        .bind(&c.id)
        .execute(pool)
        .await?;
    c.subject = subject;
    c.style = style;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;

    async fn account(pool: &SqlitePool, name: &str) -> String {
        accounts::create(
            pool,
            NewAccount {
                name: name.into(),
                role: Role::Member,
                secret: Some("4821".into()),
            },
        )
        .await
        .unwrap()
        .id
    }

    /// A fresh empty folder under the system temp folder.
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("outpost-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];

    #[tokio::test]
    async fn datasets_hold_pictures_and_captions() {
        let pool = open_in_memory().await.unwrap();
        let root = temp();
        let dan = account(&pool, "Dan").await;
        let eve = account(&pool, "Eve").await;
        assert!(create_dataset(&pool, &dan, "Kat", "has space").await.is_err());
        let d = create_dataset(&pool, &dan, "Kat", "ZK4T").await.unwrap();
        assert_eq!(d.trigger_word, "zk4t");
        let img = add_image(&pool, &root, &dan, &d.id, PNG, "zk4t, a woman smiling.").await.unwrap();
        assert_eq!(img.caption, "a woman smiling");
        assert!(add_image(&pool, &root, &dan, &d.id, b"not a picture", "").await.is_err());
        assert!(add_image(&pool, &root, &eve, &d.id, PNG, "").await.is_err(), "not Eve's");
        let two = add_image(&pool, &root, &dan, &d.id, PNG, "").await.unwrap();
        let d = get_dataset(&pool, &dan, &d.id).await.unwrap();
        assert_eq!((d.image_count, d.captioned_count), (2, 1));
        set_caption(&pool, &dan, &d.id, &two.id, "a woman in a red coat").await.unwrap();
        assert_eq!(write_captions(&pool, &root, &dan, &d.id).await.unwrap(), 2);
        let txt = std::fs::read_to_string(dataset_dir(&root, &d.id).join(&two.file).with_extension("txt")).unwrap();
        assert_eq!(txt, "zk4t, a woman in a red coat");
        delete_image(&pool, &root, &dan, &d.id, &two.id).await.unwrap();
        assert_eq!(images(&pool, &dan, &d.id).await.unwrap().len(), 1);
        delete_dataset(&pool, &root, &dan, &d.id).await.unwrap();
        assert!(!dataset_dir(&root, &d.id).exists());
        assert!(list_datasets(&pool, &dan).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn import_takes_pictures_and_their_captions() {
        let pool = open_in_memory().await.unwrap();
        let root = temp();
        let src = temp();
        std::fs::write(&src.join("a.png"), PNG).unwrap();
        std::fs::write(&src.join("a.txt"), "kat, sitting on a bench\n").unwrap();
        std::fs::write(&src.join("b.jpg"), b"broken").unwrap();
        std::fs::write(&src.join("notes.md"), b"ignored").unwrap();
        let dan = account(&pool, "Dan").await;
        let d = create_dataset(&pool, &dan, "Kat", "kat").await.unwrap();
        let (added, skipped) = import_folder(&pool, &root, &dan, &d.id, &src).await.unwrap();
        assert_eq!(added, 1);
        assert_eq!(skipped.len(), 1);
        assert_eq!(images(&pool, &dan, &d.id).await.unwrap()[0].caption, "sitting on a bench");
    }

    #[test]
    fn test_prompts_follow_the_captions() {
        let people = vec![
            "woman, brown hair, smiling, green cardigan, indoor, realistic photography".to_string(),
            "woman, bob haircut, navy hoodie, crouching, realistic photography".to_string(),
            "young woman, sundress, beach, realistic photo".to_string(),
        ];
        let p = suggest_prompts("zk4t", &people);
        assert_eq!(p.len(), 3);
        assert!(p[0].starts_with("zk4t, woman, close-up"), "{}", p[0]);
        assert!(p[0].ends_with("realistic photography"), "{}", p[0]);

        let dog = vec![
            "zr3xd0g, dog, golden fur, lying on grass, photo".to_string(),
            "dog, golden retriever, running, park, photo".to_string(),
        ];
        assert!(suggest_prompts("zr3xd0g", &dog)[1].starts_with("zr3xd0g, dog, on a snowy"));

        let toon = vec![
            "cartoon style, fox character, orange fur, big eyes".to_string(),
            "fox character, waving, cartoon style, flat colours".to_string(),
        ];
        let t = suggest_prompts("zf0x", &toon);
        assert!(t[2].starts_with("zf0x, fox character,"), "{}", t[2]);
        assert!(t[2].ends_with("cartoon style"), "{}", t[2]);

        // No captions at all: still three prompts with the trigger.
        assert!(suggest_prompts("zk4t", &[]).iter().all(|p| p.starts_with("zk4t, close-up") || p.starts_with("zk4t, on")));

        let mine = clean_prompts("zk4t", &["  a knight   in armour ".into(), "zk4t reading".into(), "".into()]).unwrap();
        assert_eq!(mine, vec!["zk4t, a knight in armour", "zk4t reading"]);
        assert!(clean_prompts("zk4t", &[]).is_err());
    }

    #[tokio::test]
    async fn characters_are_found_by_name_or_trigger() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        let eve = account(&pool, "Eve").await;
        let new = |name, trigger, file| NewCharacter {
            name,
            trigger_word: trigger,
            lora_file: file,
            training_id: None,
            step: Some(1500),
            subject: None,
            style: None,
        };
        let kat = save_character(&pool, &dan, new("Kat", "zk4t", "zk4t.safetensors")).await.unwrap();
        save_character(&pool, &dan, new("Kat Smith", "zk4ts", "zk4ts.safetensors")).await.unwrap();
        assert!(save_character(&pool, &eve, new("Mine", "zk4t", "zk4t.safetensors")).await.is_err());
        // Same file again: updated, not doubled.
        save_character(&pool, &dan, new("Kat", "zk4t", "zk4t.safetensors")).await.unwrap();
        let mine = characters(&pool, Some(&dan)).await.unwrap();
        assert_eq!(mine.len(), 2);
        assert!(characters(&pool, Some(&eve)).await.unwrap().is_empty());

        assert_eq!(mentioned("draw zk4t at the beach", &mine).unwrap().id, kat.id);
        assert_eq!(mentioned("draw kat at the beach", &mine).unwrap().id, kat.id);
        assert_eq!(mentioned("Kat Smith on a bike", &mine).unwrap().name, "Kat Smith");
        assert!(mentioned("a katana on a table", &mine).is_none());
        assert_eq!(with_trigger("Kat at the beach", &kat), "zk4t at the beach");
        assert_eq!(with_trigger("zk4t at the beach", &kat), "zk4t at the beach");
        assert_eq!(with_trigger("a portrait", &kat), "zk4t, a portrait");
        // With what she is and how she's drawn (from the captions), the model knows.
        let mut kat2 = kat.clone();
        kat2.subject = Some("woman".into());
        kat2.style = Some("realistic photography".into());
        assert_eq!(with_trigger("Kat at the beach", &kat2), "zk4t woman at the beach, realistic photography");
        assert_eq!(with_trigger("zk4t at the beach.", &kat2), "zk4t woman at the beach, realistic photography");
        assert_eq!(
            with_trigger("kat, a woman reading, realistic photography", &kat2),
            "zk4t, a woman reading, realistic photography"
        );
        assert_eq!(with_trigger("on a bike", &kat2), "zk4t woman, on a bike, realistic photography");
        delete_character(&pool, Some(&eve), &kat.id).await.unwrap_err();
        delete_character(&pool, Some(&dan), &kat.id).await.unwrap();
    }
}
