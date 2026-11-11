//! Known faces: owners add people (a name and a few photos); a routine that looks at a
//! doorbell or camera, with "Say who it is" on, is told who it most likely sees. Faces are
//! found and described by the voice service (OpenCV's YuNet and SFace, on the CPU) and matched
//! here; nothing leaves this server.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use base64::Engine;
use outpost_core::faces::{self, KnownFace};
use outpost_engines::{EngineId, FacesFetch};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// Largest photo accepted.
const MAX_PHOTO: usize = 20 * 1024 * 1024;

/// A face the voice service found in a picture.
#[derive(Debug, Clone)]
pub struct Found {
    /// x, y, width, height in the picture's pixels.
    pub area: [f32; 4],
    pub feature: Vec<f32>,
    /// The face, straightened (JPEG).
    pub crop: Vec<u8>,
}

/// Every face in a picture (biggest first).
pub async fn find(state: &AppState, picture: Vec<u8>) -> Result<Vec<Found>, String> {
    let r = state
        .llm
        .post(format!("{}/faces", EngineId::Voice.base_url()))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .timeout(Duration::from_secs(60))
        .body(picture)
        .send()
        .await
        .map_err(|_| "Voice isn't running (it finds the faces). Start it in Settings › Engines.".to_string())?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !ok {
        return Err(v["error"].as_str().unwrap_or("Faces couldn't be looked for.").to_string());
    }
    Ok(v["faces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let n = |i: usize| f["box"][i].as_f64().unwrap_or(0.0) as f32;
            let feature: Vec<f32> = f["feature"].as_array()?.iter().filter_map(Value::as_f64).map(|x| x as f32).collect();
            let crop = base64::engine::general_purpose::STANDARD.decode(f["crop"].as_str().unwrap_or("")).ok()?;
            (feature.len() == 128).then_some(Found { area: [n(0), n(1), n(2), n(3)], feature, crop })
        })
        .collect())
}

/// A picture sent as base64 or a `data:` URL.
fn picture(encoded: &str) -> Result<Vec<u8>, ApiError> {
    let raw = match encoded.split_once(";base64,") {
        Some((head, rest)) if head.starts_with("data:") => rest,
        _ => encoded,
    };
    if raw.len() > MAX_PHOTO / 3 * 4 + 8 {
        return Err(ApiError::bad_request("Photos can be up to 20 MB."));
    }
    let cleaned: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(cleaned.as_bytes())
        .map_err(|_| ApiError::bad_request("That photo couldn't be read."))
}

/// What a routine is told about the faces in its pictures: who it most likely is, and how
/// many it doesn't know. Each face is also kept a while (Seen at the door) so an owner can say
/// who it was. `None` when known faces aren't set up, or nobody's been added yet.
pub async fn describe(state: &AppState, routine_id: &str, pictures: &[String]) -> Option<String> {
    if !state.engines.faces().ready {
        return None;
    }
    let known = faces::features(&state.pool).await.ok()?;
    // Each person once, at their best; unknown faces counted per picture (the most in any one).
    let mut seen: Vec<faces::Match> = Vec::new();
    let mut unknown = 0usize;
    // The faces to keep: the same face in several stills is kept once, its biggest.
    let mut keep: Vec<(Found, Option<faces::Match>)> = Vec::new();
    for p in pictures {
        let Ok(bytes) = picture(p) else { continue };
        let found = match find(state, bytes).await {
            Ok(f) => f,
            Err(e) => return Some(format!("[Faces couldn't be checked: {e}]")),
        };
        let mut here_unknown = 0;
        for f in found {
            let m = faces::best_match(&known, &f.feature);
            match &m {
                Some(m) => match seen.iter_mut().find(|s| s.face_id == m.face_id) {
                    Some(s) if m.score > s.score => *s = m.clone(),
                    Some(_) => {}
                    None => seen.push(m.clone()),
                },
                None => here_unknown += 1,
            }
            let size = f.area[2] * f.area[3];
            match keep.iter_mut().find(|(k, _)| same_face(&k.feature, &f.feature)) {
                Some(k) if size > k.0.area[2] * k.0.area[3] => *k = (f, m),
                Some(_) => {}
                None => keep.push((f, m)),
            }
        }
        unknown = unknown.max(here_unknown);
    }
    for (f, m) in &keep {
        let _ = faces::add_seen(&state.pool, Some(routine_id), &f.feature, &f.crop, m.as_ref()).await;
    }
    if known.is_empty() {
        return None;
    }
    if keep.is_empty() {
        return Some("[Known faces: no face could be made out in the pictures. Don't name anyone.]".into());
    }
    let mut parts: Vec<String> = seen
        .iter()
        .map(|m| if m.good { format!("{} (very likely)", m.name) } else { format!("possibly {} (not sure)", m.name) })
        .collect();
    match unknown {
        0 => {}
        1 => parts.push("someone not known".into()),
        n => parts.push(format!("{n} people not known")),
    }
    Some(format!(
        "[Known faces in the pictures: {}. Name only these people, and say \"possibly\" where it says not sure; describe anyone else.]",
        parts.join("; ")
    ))
}

/// Two faces from the same few seconds that are very alike: the same person.
fn same_face(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() > 0.6
}

// ------------------------------------------------------------------ routes

#[derive(Debug, Serialize, ToSchema)]
pub struct Faces {
    pub faces: Vec<KnownFace>,
    /// Known faces are set up (in the voice service), being set up, or why not.
    pub setup: FacesFetch,
    /// The voice service is installed (it's what finds faces).
    pub voice_installed: bool,
}

async fn voice_installed(state: &AppState) -> bool {
    state.engines.status().await.engines.iter().any(|e| {
        e.id == EngineId::Voice
            && e.managed
            && !matches!(e.state, outpost_engines::EngineState::NotInstalled | outpost_engines::EngineState::Installing)
    })
}

/// Everyone Outpost knows by face. (Owners.)
#[utoipa::path(get, path = "/api/faces", tag = "faces", security(("bearer" = [])),
    responses((status = 200, body = Faces), (status = 403, description = "Not an owner", body = ErrorBody)))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Faces>> {
    auth.require_owner()?;
    Ok(Json(Faces {
        faces: faces::list(&state.pool).await?,
        setup: state.engines.faces(),
        voice_installed: voice_installed(&state).await,
    }))
}

/// Set up known faces: OpenCV and two small face models, in the voice service (about 80 MB).
#[utoipa::path(post, path = "/api/faces/setup", tag = "faces", security(("bearer" = [])),
    responses((status = 202, description = "Setting up"), (status = 409, description = "Voice isn't installed, or it's already setting up", body = ErrorBody)))]
pub async fn setup(State(state): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    if !voice_installed(&state).await {
        return Err(ApiError::conflict("Install Voice in Settings › Engines first: it's what finds the faces."));
    }
    state.engines.fetch_faces().map_err(|_| ApiError::conflict("It's already being set up."))?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewFace {
    pub name: String,
    /// Their Outpost account, when they use it.
    pub account_id: Option<String>,
}

/// Add someone (then their photos).
#[utoipa::path(post, path = "/api/faces", tag = "faces", security(("bearer" = [])), request_body = NewFace,
    responses((status = 200, body = KnownFace), (status = 400, description = "No name", body = ErrorBody)))]
pub async fn create(State(state): State<AppState>, auth: Auth, Json(input): Json<NewFace>) -> ApiResult<Json<KnownFace>> {
    auth.require_owner()?;
    Ok(Json(faces::create(&state.pool, &input.name, input.account_id.as_deref(), &auth.account.id).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FaceChanges {
    pub name: Option<String>,
    /// Their Outpost account; "" for none.
    pub account_id: Option<String>,
}

/// Rename someone, or link them to their Outpost account.
#[utoipa::path(patch, path = "/api/faces/{id}", tag = "faces", security(("bearer" = [])), request_body = FaceChanges,
    responses((status = 200, body = KnownFace), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn update(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(c): Json<FaceChanges>) -> ApiResult<Json<KnownFace>> {
    auth.require_owner()?;
    Ok(Json(faces::update(&state.pool, &id, c.name.as_deref(), c.account_id.as_deref()).await?))
}

/// Forget someone: their name and every face of theirs.
#[utoipa::path(delete, path = "/api/faces/{id}", tag = "faces", security(("bearer" = [])),
    responses((status = 204, description = "Forgotten"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    faces::delete(&state.pool, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewPhoto {
    /// A photo of them (JPEG or PNG), as base64 or a `data:` URL, up to 20 MB.
    pub image: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PhotoAdded {
    pub face: KnownFace,
    /// Said when it wasn't straightforward (several faces: the biggest was kept).
    pub note: Option<String>,
}

/// Add a photo of someone: their face in it is kept (the biggest, if there are several).
#[utoipa::path(post, path = "/api/faces/{id}/photos", tag = "faces", security(("bearer" = [])), request_body = NewPhoto,
    responses((status = 200, body = PhotoAdded), (status = 400, description = "No face in it", body = ErrorBody),
        (status = 409, description = "Known faces aren't set up", body = ErrorBody)))]
pub async fn add_photo(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(p): Json<NewPhoto>) -> ApiResult<Json<PhotoAdded>> {
    auth.require_owner()?;
    faces::get(&state.pool, &id).await?;
    if !state.engines.faces().ready {
        return Err(ApiError::conflict("Set up known faces first."));
    }
    let found = find(&state, picture(&p.image)?).await.map_err(ApiError::bad_request)?;
    let Some(face) = found.first() else {
        return Err(ApiError::bad_request("No face could be found in that photo. Try one where their face is clear and facing the camera."));
    };
    // A face much smaller than the picture's biggest isn't a mix-up; two big ones might be.
    let note = (found.len() > 1).then(|| {
        format!("There were {} faces in that photo; the biggest was kept. Check it's theirs below.", found.len())
    });
    faces::add_sample(&state.pool, &id, &face.feature, &face.crop).await?;
    Ok(Json(PhotoAdded { face: faces::get(&state.pool, &id).await?, note }))
}

/// A kept face (JPEG).
#[utoipa::path(get, path = "/api/faces/samples/{id}", tag = "faces", security(("bearer" = [])),
    responses((status = 200, description = "JPEG"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn sample(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    auth.require_owner()?;
    let jpeg = faces::crop(&state.pool, &id).await?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "private, max-age=86400")
        .body(Body::from(jpeg))
        .expect("response"))
}

/// Remove one kept face (a bad photo).
#[utoipa::path(delete, path = "/api/faces/samples/{id}", tag = "faces", security(("bearer" = [])),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_sample(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    faces::delete_sample(&state.pool, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Faces seen at the door in the last 30 days (by routines with Say who it is on), newest first.
#[utoipa::path(get, path = "/api/faces/seen", tag = "faces", security(("bearer" = [])),
    responses((status = 200, body = Vec<faces::SeenFace>)))]
pub async fn seen(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<faces::SeenFace>>> {
    auth.require_owner()?;
    Ok(Json(faces::seen(&state.pool).await?))
}

/// A seen face (JPEG).
#[utoipa::path(get, path = "/api/faces/seen/{id}/picture", tag = "faces", security(("bearer" = [])),
    responses((status = 200, description = "JPEG"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn seen_picture(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    auth.require_owner()?;
    let jpeg = faces::seen_crop(&state.pool, &id).await?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "private, max-age=86400")
        .body(Body::from(jpeg))
        .expect("response"))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WhoItWas {
    /// Someone already in Known faces...
    pub face_id: Option<String>,
    /// ...or someone new, by name.
    pub name: Option<String>,
}

/// Say who a seen face was: it's kept as one of their faces.
#[utoipa::path(post, path = "/api/faces/seen/{id}/save", tag = "faces", security(("bearer" = [])), request_body = WhoItWas,
    responses((status = 200, body = KnownFace), (status = 400, description = "Neither someone nor a name", body = ErrorBody)))]
pub async fn save_seen(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(w): Json<WhoItWas>) -> ApiResult<Json<KnownFace>> {
    auth.require_owner()?;
    let face_id = match (w.face_id.filter(|f| !f.is_empty()), w.name.filter(|n| !n.trim().is_empty())) {
        (Some(f), _) => f,
        (None, Some(n)) => faces::create(&state.pool, &n, None, &auth.account.id).await?.id,
        (None, None) => return Err(ApiError::bad_request("Say who it was: someone already added, or a new name.")),
    };
    faces::save_seen(&state.pool, &id, &face_id).await?;
    Ok(Json(faces::get(&state.pool, &face_id).await?))
}

/// Remove a seen face (a stranger, or too blurry).
#[utoipa::path(delete, path = "/api/faces/seen/{id}", tag = "faces", security(("bearer" = [])),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_seen(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    faces::delete_seen(&state.pool, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
