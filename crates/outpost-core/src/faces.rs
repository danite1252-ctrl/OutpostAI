//! Known faces: people Outpost can name at the door. Each is a name (and, if they use
//! Outpost, their account) with a few photos' faces, kept as 128 numbers each plus the face
//! itself for the People page. Everything stays on this server; owners manage them.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

/// The most faces kept for one person (the oldest go first).
pub const MAX_SAMPLES: i64 = 40;
/// At least this alike (cosine, of unit-length features): very likely the same person.
pub const GOOD_MATCH: f32 = 0.45;
/// At least this alike: SFace's own line for "the same person", said with less certainty.
pub const POSSIBLE_MATCH: f32 = 0.363;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FaceSample {
    pub id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KnownFace {
    pub id: String,
    pub name: String,
    /// Their Outpost account, when they use it.
    pub account_id: Option<String>,
    pub added_by: String,
    pub created_at: i64,
    /// Their faces, newest first.
    pub samples: Vec<FaceSample>,
}

fn clean_name(name: &str) -> CoreResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 60 {
        return Err(CoreError::Invalid("Give them a name (up to 60 letters).".into()));
    }
    Ok(n.to_string())
}

pub async fn list(pool: &SqlitePool) -> CoreResult<Vec<KnownFace>> {
    let faces: Vec<(String, String, Option<String>, String, i64)> =
        sqlx::query_as("SELECT id, name, account_id, added_by, created_at FROM known_faces ORDER BY name COLLATE NOCASE")
            .fetch_all(pool)
            .await?;
    let samples: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT id, face_id, created_at FROM face_samples ORDER BY created_at DESC").fetch_all(pool).await?;
    Ok(faces
        .into_iter()
        .map(|(id, name, account_id, added_by, created_at)| KnownFace {
            samples: samples
                .iter()
                .filter(|(_, f, _)| *f == id)
                .map(|(sid, _, at)| FaceSample { id: sid.clone(), created_at: *at })
                .collect(),
            id,
            name,
            account_id,
            added_by,
            created_at,
        })
        .collect())
}

pub async fn get(pool: &SqlitePool, id: &str) -> CoreResult<KnownFace> {
    list(pool).await?.into_iter().find(|f| f.id == id).ok_or(CoreError::NotFound)
}

pub async fn create(pool: &SqlitePool, name: &str, account_id: Option<&str>, added_by: &str) -> CoreResult<KnownFace> {
    let name = clean_name(name)?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO known_faces (id, name, account_id, added_by, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(&name)
        .bind(account_id.filter(|a| !a.is_empty()))
        .bind(added_by)
        .bind(now())
        .execute(pool)
        .await?;
    get(pool, &id).await
}

/// Rename, or link to (or unlink from) an Outpost account (`Some("")` unlinks).
pub async fn update(pool: &SqlitePool, id: &str, name: Option<&str>, account_id: Option<&str>) -> CoreResult<KnownFace> {
    let f = get(pool, id).await?;
    let name = match name {
        Some(n) => clean_name(n)?,
        None => f.name,
    };
    let account = match account_id {
        Some(a) => Some(a).filter(|a| !a.is_empty()).map(str::to_string),
        None => f.account_id,
    };
    sqlx::query("UPDATE known_faces SET name = ?, account_id = ? WHERE id = ?")
        .bind(&name)
        .bind(&account)
        .bind(id)
        .execute(pool)
        .await?;
    get(pool, id).await
}

/// Forget someone: their name and every face of theirs.
pub async fn delete(pool: &SqlitePool, id: &str) -> CoreResult<()> {
    let done = sqlx::query("DELETE FROM known_faces WHERE id = ?").bind(id).execute(pool).await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

fn to_blob(feature: &[f32]) -> Vec<u8> {
    feature.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn from_blob(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Keep one more face for someone (the oldest go past [`MAX_SAMPLES`]).
pub async fn add_sample(pool: &SqlitePool, face_id: &str, feature: &[f32], crop: &[u8]) -> CoreResult<FaceSample> {
    if feature.len() != 128 {
        return Err(CoreError::Invalid("That face couldn't be read.".into()));
    }
    get(pool, face_id).await?;
    let s = FaceSample { id: uuid::Uuid::new_v4().to_string(), created_at: now() };
    sqlx::query("INSERT INTO face_samples (id, face_id, feature, crop, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&s.id)
        .bind(face_id)
        .bind(to_blob(feature))
        .bind(crop)
        .bind(s.created_at)
        .execute(pool)
        .await?;
    sqlx::query(
        "DELETE FROM face_samples WHERE face_id = ? AND id NOT IN \
         (SELECT id FROM face_samples WHERE face_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?)",
    )
    .bind(face_id)
    .bind(face_id)
    .bind(MAX_SAMPLES)
    .execute(pool)
    .await?;
    Ok(s)
}

/// A kept face, as JPEG.
pub async fn crop(pool: &SqlitePool, sample_id: &str) -> CoreResult<Vec<u8>> {
    sqlx::query_scalar("SELECT crop FROM face_samples WHERE id = ?")
        .bind(sample_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

pub async fn delete_sample(pool: &SqlitePool, sample_id: &str) -> CoreResult<()> {
    let done = sqlx::query("DELETE FROM face_samples WHERE id = ?").bind(sample_id).execute(pool).await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// Everyone's faces, for matching: (face id, name, feature).
pub async fn features(pool: &SqlitePool) -> CoreResult<Vec<(String, String, Vec<f32>)>> {
    let rows: Vec<(String, String, Vec<u8>)> = sqlx::query_as(
        "SELECT k.id, k.name, s.feature FROM face_samples s JOIN known_faces k ON k.id = s.face_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id, name, f)| (id, name, from_blob(&f))).collect())
}

/// Who a face most looks like.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct Match {
    pub face_id: String,
    pub name: String,
    pub score: f32,
    /// Very likely them (`false`: possibly them).
    pub good: bool,
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// The known person a face is most like, if they're alike enough. A person counts by their
/// closest face; when two people are both close and within 0.05 of each other, it's too close
/// to call.
pub fn best_match(known: &[(String, String, Vec<f32>)], probe: &[f32]) -> Option<Match> {
    let mut best: Vec<(String, String, f32)> = Vec::new();
    for (id, name, f) in known {
        let s = cosine(f, probe);
        match best.iter_mut().find(|(b, _, _)| b == id) {
            Some(b) if s > b.2 => b.2 = s,
            Some(_) => {}
            None => best.push((id.clone(), name.clone(), s)),
        }
    }
    best.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let (id, name, score) = best.first()?.clone();
    if score < POSSIBLE_MATCH {
        return None;
    }
    if let Some(second) = best.get(1) {
        if second.2 >= POSSIBLE_MATCH && score - second.2 < 0.05 {
            return None;
        }
    }
    Some(Match { face_id: id, name, score, good: score >= GOOD_MATCH })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let mut f = vec![0.0; 128];
        f[..v.len()].copy_from_slice(v);
        let n: f32 = f.iter().map(|x| x * x).sum::<f32>().sqrt();
        f.iter().map(|x| x / n).collect()
    }

    #[test]
    fn a_face_is_named_only_when_it_is_alike_enough() {
        let known = vec![
            ("sam".to_string(), "Sam".to_string(), unit(&[1.0, 0.0, 0.0])),
            ("sam".to_string(), "Sam".to_string(), unit(&[0.9, 0.3, 0.0])),
            ("mia".to_string(), "Mia".to_string(), unit(&[0.0, 1.0, 0.0])),
        ];
        let m = best_match(&known, &unit(&[0.95, 0.1, 0.0])).unwrap();
        assert_eq!((m.name.as_str(), m.good), ("Sam", true));
        // Someone not added.
        assert_eq!(best_match(&known, &unit(&[0.0, 0.0, 1.0])), None);
        // Halfway between two people: too close to call.
        assert_eq!(best_match(&known, &unit(&[0.949, 1.316, 0.0])), None);
        // Somewhat like Sam only: possibly Sam.
        let m = best_match(&known, &unit(&[0.4, 0.0, 0.9])).unwrap();
        assert_eq!((m.name.as_str(), m.good), ("Sam", false));
    }

    #[tokio::test]
    async fn people_and_their_faces_are_kept_and_forgotten() {
        let pool = crate::db::open_in_memory().await.unwrap();
        let f = create(&pool, " Sam ", None, "owner").await.unwrap();
        assert_eq!(f.name, "Sam");
        assert!(create(&pool, "  ", None, "owner").await.is_err());
        let s = add_sample(&pool, &f.id, &unit(&[1.0]), b"jpeg").await.unwrap();
        assert!(add_sample(&pool, &f.id, &[0.5; 3], b"x").await.is_err());
        assert_eq!(crop(&pool, &s.id).await.unwrap(), b"jpeg");
        assert_eq!(features(&pool).await.unwrap()[0].2, unit(&[1.0]));
        let f = update(&pool, &f.id, Some("Samuel"), None).await.unwrap();
        assert_eq!((f.name.as_str(), f.samples.len()), ("Samuel", 1));
        delete(&pool, &f.id).await.unwrap();
        assert!(crop(&pool, &s.id).await.is_err());
        assert!(list(&pool).await.unwrap().is_empty());
    }
}
