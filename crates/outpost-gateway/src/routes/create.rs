//! Create: workflow packs, their model downloads, the GPU job queue and the gallery.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use outpost_core::attachments;
use outpost_core::jobs::{self, Job, JobStatus};
use outpost_core::media::{self, Media};
use outpost_engines::assets::PackModels;
use outpost_engines::catalog::Fit;
use outpost_engines::hardware::Hardware;
use outpost_engines::packs::{self, InputKind, Pack};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PackInfo {
    #[serde(flatten)]
    pub pack: Pack,
    pub models: PackModels,
    /// How the pack's peak video memory compares with this PC.
    pub fit: Fit,
    /// The Create tab it belongs to.
    pub task: packs::Task,
    /// Outpost's pick for its task on this PC.
    pub recommended: bool,
    /// ComfyUI node types this pack needs that the installed ComfyUI lacks: it needs
    /// a newer ComfyUI. Empty when all are there, or ComfyUI hasn't been seen yet.
    pub missing_nodes: Vec<String>,
}

/// Does a pack needing `vram_gb` at its peak run here? ComfyUI can spill to
/// system memory, so between the two it runs, slowly.
pub fn pack_fit(vram_gb: f32, hw: &Hardware) -> Fit {
    packs::fit(vram_gb, hw.vram_mb as f32 / 1024.0, hw.ram_mb as f32 / 1024.0)
}

fn pack_or_404(id: &str) -> Result<Pack, ApiError> {
    packs::find(id).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", format!("No pack called {id}.")))
}

/// The workflow packs, with whether their models are downloaded and how they fit this PC.
#[utoipa::path(get, path = "/api/packs", tag = "create", security(("bearer" = [])),
    responses((status = 200, description = "Packs", body = [PackInfo])))]
pub async fn list_packs(State(state): State<AppState>, _auth: Auth) -> Json<Vec<PackInfo>> {
    let hw = crate::routes::models::measure(&state).await;
    let nodes = state.comfy.node_types().await;
    Json(pack_infos(&state, &hw, nodes.as_ref().map(|n| &n.nodes)))
}

fn pack_infos(
    state: &AppState,
    hw: &Hardware,
    comfy_nodes: Option<&std::collections::HashSet<String>>,
) -> Vec<PackInfo> {
    let all = packs::built_in();
    let missing = |p: &Pack| comfy_nodes.map(|have| p.missing_nodes(have)).unwrap_or_default();
    // A pack this ComfyUI can't run is never the pick.
    let picks: Vec<String> = packs::Task::ALL
        .iter()
        .filter_map(|t| {
            packs::pick(&all, *t, |p| {
                if missing(p).is_empty() {
                    pack_fit(p.vram_gb, hw)
                } else {
                    Fit::TooLarge
                }
            })
        })
        .map(|p| p.id.clone())
        .collect();
    all.iter()
        .map(|pack| PackInfo {
            models: state.assets.status(pack),
            fit: pack_fit(pack.vram_gb, hw),
            task: pack.task(),
            recommended: picks.contains(&pack.id),
            missing_nodes: missing(pack),
            pack: pack.clone(),
        })
        .collect()
}

/// Download a pack's models (owner only). Runs in the background; poll `GET /api/packs`.
#[utoipa::path(post, path = "/api/packs/{id}/install", tag = "create", security(("bearer" = [])),
    params(("id" = String, Path, description = "Pack id")),
    responses((status = 202, description = "Downloading"), (status = 403, description = "Not allowed", body = ErrorBody), (status = 409, description = "Already downloading or no space", body = ErrorBody)))]
pub async fn install_pack(
    State(state): State<AppState>,
    auth: Auth,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    auth.require_fresh_pin(&state, &headers).await?;
    let pack = pack_or_404(&id)?;
    state
        .assets
        .install(&pack)
        .map_err(|e| ApiError::new(StatusCode::CONFLICT, "conflict", e.to_string()))?;
    outpost_core::audit::log_quiet(
        &state.pool,
        Some(&auth.account.id),
        Some(&auth.session.id),
        "pack_install",
        Some(&id),
    )
    .await;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LoraList {
    /// LoRA files available to packs.
    pub loras: Vec<String>,
    /// The folder to put new ones in.
    pub folder: String,
}

/// LoRA files in Outpost's LoRA folder.
#[utoipa::path(get, path = "/api/loras", tag = "create", security(("bearer" = [])),
    responses((status = 200, description = "LoRAs", body = LoraList)))]
pub async fn list_loras(State(state): State<AppState>, _auth: Auth) -> Json<LoraList> {
    Json(LoraList {
        loras: state.assets.loras(),
        folder: state.assets.loras_dir().to_string_lossy().to_string(),
    })
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NewJob {
    pub pack: String,
    /// Values for the pack's inputs, by key. Image inputs are base64 or
    /// `data:image/...;base64,` (PNG, JPEG, WebP or GIF, up to 10 MB).
    #[schema(value_type = Object)]
    pub inputs: serde_json::Map<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct JobView {
    #[serde(flatten)]
    pub job: Job,
    /// Jobs ahead of this one (queued jobs only).
    pub position: Option<i64>,
    /// While running.
    pub progress: Option<outpost_engines::comfy::JobProgress>,
    /// What it made (finished jobs).
    pub media: Vec<Media>,
}

async fn view(state: &AppState, job: Job) -> ApiResult<JobView> {
    let position = if job.status == JobStatus::Queued {
        Some(jobs::position(&state.pool, &job).await?)
    } else {
        None
    };
    let progress = state.queue.running().filter(|r| r.job_id == job.id).map(|r| r.progress);
    let media = if job.status == JobStatus::Done {
        media::for_job(&state.pool, &job.account_id, &job.id).await?
    } else {
        Vec::new()
    };
    Ok(JobView {
        job,
        position,
        progress,
        media,
    })
}

/// Queue a job. It is checked now (so mistakes show straight away) and runs when
/// the GPU is free.
#[utoipa::path(post, path = "/api/jobs", tag = "create", security(("bearer" = [])),
    request_body = NewJob,
    responses((status = 201, description = "Queued", body = JobView), (status = 400, description = "Not accepted", body = ErrorBody), (status = 409, description = "Models missing or too many jobs", body = ErrorBody)))]
pub async fn create_job(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<NewJob>,
) -> ApiResult<(StatusCode, Json<JobView>)> {
    let pack = pack_or_404(&req.pack)?;
    if !state.assets.status(&pack).ready {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "models_missing",
            format!("{}'s models aren't downloaded yet.", pack.name),
        ));
    }
    if let Some(have) = state.comfy.node_types().await {
        let missing = pack.missing_nodes(&have.nodes);
        if !missing.is_empty() {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "comfyui_too_old",
                format!(
                    "{} needs a newer ComfyUI (it has no {}). Update ComfyUI in Settings › Engines.",
                    pack.name,
                    missing.join(", ")
                ),
            ));
        }
    }
    // Only the pack's own inputs are kept.
    let mut inputs: serde_json::Map<String, Value> = req
        .inputs
        .into_iter()
        .filter(|(k, _)| pack.inputs.iter().any(|i| &i.key == k))
        .collect();
    jobs::check_room(&state.pool, &auth.account.id).await?;
    // Images arrive as base64; check each, keep it in the account's folder, and
    // store its path in the job instead.
    let mut uploaded = HashMap::new();
    let mut saved = Vec::new();
    for input in pack.inputs.iter().filter(|i| i.kind == InputKind::Image) {
        let Some(encoded) = inputs.get(&input.key).and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            inputs.remove(&input.key);
            continue;
        };
        let image = attachments::decode_image(encoded)?;
        let rel = attachments::save_job_input(&state.config.files_dir(), &auth.account.id, &image).await?;
        saved.push(state.config.files_dir().join(&rel));
        uploaded.insert(input.key.clone(), "check.png".to_string());
        inputs.insert(input.key.clone(), serde_json::json!({ "$file": rel }));
    }
    // Validate by building once (the real build happens when the job runs).
    if let Err(e) = pack.build(&inputs, || 0, &uploaded, &state.assets.loras()) {
        attachments::remove_files(&saved).await;
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", e.to_string()));
    }
    let priority = jobs::priority_for(auth.session.kind);
    let job = match jobs::create(&state.pool, &auth.account.id, &pack.id, &Value::Object(inputs), priority).await {
        Ok(job) => job,
        Err(e) => {
            attachments::remove_files(&saved).await;
            return Err(e.into());
        }
    };
    state.queue.wake.notify_one();
    Ok((StatusCode::CREATED, Json(view(&state, job).await?)))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListQuery {
    /// How many (default 30).
    pub limit: Option<i64>,
}

/// My recent jobs, newest first, with queue positions and live progress.
#[utoipa::path(get, path = "/api/jobs", tag = "create", security(("bearer" = [])),
    params(ListQuery),
    responses((status = 200, description = "Jobs", body = [JobView])))]
pub async fn list_jobs(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<JobView>>> {
    let mut out = Vec::new();
    for job in jobs::list(&state.pool, &auth.account.id, q.limit.unwrap_or(30)).await? {
        out.push(view(&state, job).await?);
    }
    Ok(Json(out))
}

/// One of my jobs.
#[utoipa::path(get, path = "/api/jobs/{id}", tag = "create", security(("bearer" = [])),
    params(("id" = String, Path, description = "Job id")),
    responses((status = 200, description = "Job", body = JobView), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_job(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<JobView>> {
    let job = jobs::get(&state.pool, &auth.account.id, &id).await?;
    Ok(Json(view(&state, job).await?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LineView {
    /// Yours: you can see what it is and cancel it.
    pub mine: bool,
    /// Your job's id and workflow; left out for other people's.
    pub job_id: Option<String>,
    pub pack_id: Option<String>,
    /// Who it belongs to.
    pub by_name: String,
    pub by_color: String,
    pub status: JobStatus,
    /// The running job's progress (anyone's: "Sam is creating, 60%").
    pub progress: Option<outpost_engines::comfy::JobProgress>,
    /// Seconds left in the running job's current pass, once it can be told.
    pub eta_secs: Option<u64>,
}

/// The line for the graphics card, everyone's jobs in the order they run. Other
/// people's jobs show only whose they are and how far along, never what they make.
#[utoipa::path(get, path = "/api/queue", tag = "create", security(("bearer" = [])),
    responses((status = 200, description = "The queue", body = [LineView])))]
pub async fn queue(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<LineView>>> {
    let running = state.queue.running();
    let out = jobs::line(&state.pool)
        .await?
        .into_iter()
        .map(|e| {
            let mine = e.account_id == auth.account.id;
            let live = running.as_ref().filter(|r| r.job_id == e.job_id);
            LineView {
                mine,
                job_id: mine.then(|| e.job_id.clone()),
                pack_id: mine.then(|| e.pack_id.clone()),
                by_name: e.account_name,
                by_color: e.avatar_color,
                status: e.status,
                progress: live.map(|r| r.progress.clone()),
                eta_secs: live.and_then(|r| r.eta_secs()),
            }
        })
        .collect();
    Ok(Json(out))
}

/// Cancel one of my jobs: a queued one is removed, a running one is stopped.
#[utoipa::path(delete, path = "/api/jobs/{id}", tag = "create", security(("bearer" = [])),
    params(("id" = String, Path, description = "Job id")),
    responses((status = 204, description = "Cancelled"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn cancel_job(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let was = jobs::cancel(&state.pool, &auth.account.id, &id).await?;
    if was == JobStatus::Running {
        // Stop it in ComfyUI if it got there; the worker also keeps asking until it
        // stops, and records the cancel.
        if let Some(prompt_id) = state.queue.cancel(&id) {
            state.comfy.interrupt(&prompt_id).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct GalleryQuery {
    /// Someone else's gallery (owner only).
    pub account_id: Option<String>,
    pub limit: Option<i64>,
    /// Show items made up to this time (Unix seconds), for paging. Items from that
    /// same second come again; drop the ones you already have.
    pub before: Option<i64>,
}

/// My gallery, newest first. The owner can ask for any account's.
#[utoipa::path(get, path = "/api/gallery", tag = "create", security(("bearer" = [])),
    params(GalleryQuery),
    responses((status = 200, description = "Gallery", body = [Media])))]
pub async fn gallery(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<GalleryQuery>,
) -> ApiResult<Json<Vec<Media>>> {
    let account_id = match q.account_id {
        Some(id) if id != auth.account.id => {
            auth.require_owner()?;
            id
        }
        _ => auth.account.id.clone(),
    };
    Ok(Json(
        media::list(&state.pool, &account_id, q.limit.unwrap_or(60), q.before).await?,
    ))
}

/// One of my gallery items (the owner: anyone's), as the image or video itself.
#[utoipa::path(get, path = "/api/media/{id}", tag = "create", security(("bearer" = [])),
    params(("id" = String, Path, description = "Media id")),
    responses((status = 200, description = "The file"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_media(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    let (m, path) = find_media(&state, &auth, &id, true).await?;
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That file is missing."))?;
    Response::builder()
        .header(header::CONTENT_TYPE, m.mime)
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(data))
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Could not send the file.",
            )
        })
}

/// Mine, for the owner anyone's; for viewing, also anything in the shared gallery.
async fn find_media(state: &AppState, auth: &Auth, id: &str, viewing: bool) -> ApiResult<(Media, std::path::PathBuf)> {
    let dir = state.config.files_dir();
    if auth.require_owner().is_ok() {
        return Ok(media::get_any(&state.pool, &dir, id).await?);
    }
    match media::get(&state.pool, &dir, &auth.account.id, id).await {
        Err(outpost_core::CoreError::NotFound) if viewing => Ok(media::get_shared(&state.pool, &dir, id).await?),
        other => Ok(other?),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SharedQuery {
    pub limit: Option<i64>,
    /// Show items shared up to this time (Unix seconds), for paging; that second repeats.
    pub before: Option<i64>,
}

/// The shared gallery: pictures and videos anyone chose to share with everyone here.
#[utoipa::path(get, path = "/api/gallery/shared", tag = "create", security(("bearer" = [])),
    params(SharedQuery),
    responses((status = 200, description = "Shared gallery", body = [outpost_core::media::SharedMedia])))]
pub async fn shared_gallery(
    State(state): State<AppState>,
    _auth: Auth,
    Query(q): Query<SharedQuery>,
) -> ApiResult<Json<Vec<outpost_core::media::SharedMedia>>> {
    Ok(Json(media::list_shared(&state.pool, q.limit.unwrap_or(60), q.before).await?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ShareRequest {
    /// `true` to put it in the shared gallery, `false` to take it out.
    pub shared: bool,
}

/// Share one of my items with everyone, or stop sharing it. The owner can take
/// anyone's out of the shared gallery.
#[utoipa::path(post, path = "/api/media/{id}/share", tag = "create", security(("bearer" = [])),
    params(("id" = String, Path, description = "Media id")),
    request_body = ShareRequest,
    responses((status = 200, description = "Updated", body = Media), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn share_media(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<ShareRequest>,
) -> ApiResult<Json<Media>> {
    let owner = auth.require_owner().is_ok();
    // An owner may unshare anyone's, but shares only their own.
    let m = media::set_shared(&state.pool, &auth.account.id, owner && !req.shared, &id, req.shared).await?;
    Ok(Json(m))
}

/// Delete one of my gallery items (the owner can delete anyone's).
#[utoipa::path(delete, path = "/api/media/{id}", tag = "create", security(("bearer" = [])),
    params(("id" = String, Path, description = "Media id")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_media(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let (m, _) = find_media(&state, &auth, &id, false).await?;
    media::delete(&state.pool, &state.config.files_dir(), &m.account_id, &id).await?;
    if m.account_id != auth.account.id {
        outpost_core::audit::log_quiet(
            &state.pool,
            Some(&auth.account.id),
            Some(&auth.session.id),
            "media_delete_other",
            Some(&id),
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outpost_engines::hardware::Tier;

    #[test]
    fn packs_fit_by_peak_vram() {
        let hw = |v: u64, r: u64| Hardware {
            gpus: Vec::new(),
            vram_mb: v * 1024,
            ram_mb: r * 1024,
            cpu_threads: 8,
            disk_free_mb: 1,
            tier: Tier::from_vram_mb(v * 1024),
        };
        assert_eq!(pack_fit(14.0, &hw(16, 64)), Fit::Fits);
        assert_eq!(pack_fit(14.0, &hw(8, 32)), Fit::Partial);
        assert_eq!(pack_fit(40.0, &hw(8, 16)), Fit::TooLarge);
    }
}
