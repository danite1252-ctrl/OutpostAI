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
    /// The helper PC that runs it, when this server can't (not downloaded here, or too big
    /// for its card) and a helper has it ready: "Runs on Dan's PC".
    pub runs_on: Option<String>,
}

/// The helper PC that would run `pack`, when this server wouldn't run it itself.
pub fn helper_for(state: &AppState, pack: &Pack, hw: &Hardware) -> Option<String> {
    let here = state.assets.status(pack).ready && pack_fit(pack.vram_gb, hw) == Fit::Fits;
    if here {
        return None;
    }
    state.helpers.for_pack(pack).map(|h| h.name)
}

/// Does a pack needing `vram_gb` at its peak run here? ComfyUI can spill to
/// system memory, so between the two it runs, slowly.
pub fn pack_fit(vram_gb: f32, hw: &Hardware) -> Fit {
    packs::fit(vram_gb, hw.vram_mb as f32 / 1024.0, hw.ram_mb as f32 / 1024.0)
}

fn pack_or_404(id: &str) -> Result<Pack, ApiError> {
    packs::find(id).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", format!("No pack called {id}.")))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PacksQuery {
    /// Every pack, including ones too big for this graphics card.
    pub all: Option<bool>,
}

/// Worth listing in Create: it runs within the graphics card's memory, or its models are
/// already downloaded (or downloading). Packs that would only run by spilling into system
/// memory, or not at all, are left out so the choices stay short.
pub fn worth_showing(info: &PackInfo) -> bool {
    info.fit == Fit::Fits || info.models.ready || info.models.setup.is_some() || info.runs_on.is_some()
}

/// The workflow packs that suit this machine (see [`worth_showing`]; `?all=true` for every
/// one), with whether their models are downloaded and how they fit.
#[utoipa::path(get, path = "/api/packs", tag = "create", security(("bearer" = [])),
    params(PacksQuery),
    responses((status = 200, description = "Packs", body = [PackInfo])))]
pub async fn list_packs(State(state): State<AppState>, _auth: Auth, Query(q): Query<PacksQuery>) -> Json<Vec<PackInfo>> {
    let hw = crate::routes::models::measure(&state).await;
    let nodes = state.comfy.node_types().await;
    let mut infos = pack_infos(&state, &hw, nodes.as_ref().map(|n| &n.nodes));
    if !q.all.unwrap_or(false) {
        infos.retain(worth_showing);
    }
    Json(infos)
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
        .map(|pack| {
            let runs_on = helper_for(state, pack, hw);
            PackInfo {
                models: state.assets.status(pack),
                fit: pack_fit(pack.vram_gb, hw),
                task: pack.task(),
                recommended: picks.contains(&pack.id),
                // A helper PC's own ComfyUI is what matters for those.
                missing_nodes: if runs_on.is_some() { Vec::new() } else { missing(pack) },
                runs_on,
                pack: pack.clone(),
            }
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
    /// My trained characters, by the LoRA file each uses.
    pub characters: Vec<outpost_core::training::Character>,
}

/// The LoRAs an account may see and use, like personas: its own (uploaded or trained, in
/// `loras/<account id>/`), plus shared ones at the top of the LoRA folder, except a shared
/// file that is someone else's character.
pub async fn visible_loras(state: &AppState, account_id: &str) -> Vec<String> {
    let everyone = outpost_core::training::characters(&state.pool, None).await.unwrap_or_default();
    let mut out = state.assets.account_loras(account_id);
    out.extend(
        state
            .assets
            .loras()
            .into_iter()
            .filter(|f| !everyone.iter().any(|c| &c.lora_file == f && c.account_id != account_id)),
    );
    out
}

/// LoRAs used to share one folder. At start-up, move each shared LoRA that clearly belongs to
/// someone into their own folder: a character's LoRA goes to the character's owner, and an
/// uploaded one to whoever uploaded it (from the audit log). Ones with no known owner stay
/// shared. Returns how many moved.
pub async fn claim_shared_loras(state: &AppState) -> usize {
    let mut moved = 0;
    for file in state.assets.loras() {
        let owner = match outpost_core::training::character_by_lora(&state.pool, &file).await {
            Ok(Some(c)) => Some(c.account_id),
            _ => sqlx::query_scalar::<_, Option<String>>(
                "SELECT account_id FROM audit_log WHERE action = 'lora_upload' AND detail = ? ORDER BY at DESC LIMIT 1",
            )
            .bind(&file)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            .flatten(),
        };
        let Some(owner) = owner else { continue };
        if outpost_core::accounts::get(&state.pool, &owner).await.is_err() {
            continue;
        }
        let dir = state.assets.account_loras_dir(&owner);
        let (from, to) = (state.assets.loras_dir().join(&file), dir.join(&file));
        if to.exists() || std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        if let Err(e) = std::fs::rename(&from, &to) {
            tracing::warn!(%file, error = %e, "couldn't move a LoRA into its owner's folder");
            continue;
        }
        let rel = format!("{owner}/{file}");
        if let Err(e) = outpost_core::training::move_character_lora(&state.pool, &file, &rel).await {
            // Put it back, so the character keeps working.
            let _ = std::fs::rename(&to, &from);
            tracing::warn!(%file, error = %e, "couldn't update a character's LoRA");
            continue;
        }
        outpost_core::audit::log_quiet(&state.pool, Some(&owner), None, "lora_made_private", Some(&rel)).await;
        tracing::info!(%file, "moved a LoRA into its owner's own folder");
        moved += 1;
    }
    moved
}

/// My LoRAs (and shared ones), with which are my characters.
#[utoipa::path(get, path = "/api/loras", tag = "create", security(("bearer" = [])),
    responses((status = 200, description = "LoRAs", body = LoraList)))]
pub async fn list_loras(State(state): State<AppState>, auth: Auth) -> Json<LoraList> {
    let mut characters = outpost_core::training::characters(&state.pool, Some(auth.account.id.as_str()))
        .await
        .unwrap_or_default();
    for c in &mut characters {
        let _ = outpost_core::training::fill_subject(&state.pool, c).await;
    }
    Json(LoraList {
        loras: visible_loras(&state, &auth.account.id).await,
        folder: state.assets.account_loras_dir(&auth.account.id).to_string_lossy().to_string(),
        characters,
    })
}

/// The largest LoRA Outpost takes (big ones are around 1–2 GB).
pub const MAX_LORA_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Deserialize, IntoParams)]
pub struct LoraUpload {
    /// The file name, e.g. `kat.safetensors`.
    pub name: String,
    /// Overwrite a LoRA with the same name.
    pub replace: Option<bool>,
    /// Also make it one of my characters, with this name (needs `trigger`).
    pub character: Option<String>,
    /// The character's trigger word, as it was trained.
    pub trigger: Option<String>,
    /// What it is, e.g. "woman" or "dog" (helps pictures of it).
    pub subject: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LoraUploaded {
    pub file: String,
    pub bytes: u64,
    pub character: Option<outpost_core::training::Character>,
}

/// `Kat v2.safetensors` → `Kat_v2.safetensors`; refuses anything that isn't a plain
/// `.safetensors` name.
pub fn lora_file_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    let stem = name
        .strip_suffix(".safetensors")
        .or_else(|| name.strip_suffix(".SAFETENSORS"))
        .ok_or_else(|| ApiError::bad_request("A LoRA must be a .safetensors file."))?;
    let stem: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let stem = stem.trim_matches(['.', '_']);
    if stem.is_empty() || stem.len() > 100 {
        return Err(ApiError::bad_request("Give the LoRA a name of up to 100 letters."));
    }
    Ok(format!("{stem}.safetensors"))
}

/// A safetensors file starts with the length of its JSON header, then the header.
fn looks_like_safetensors(start: &[u8], total: u64) -> bool {
    if start.len() < 9 {
        return false;
    }
    let n = u64::from_le_bytes(start[..8].try_into().unwrap_or_default());
    n >= 2 && n < 100 * 1024 * 1024 && 8 + n <= total && start[8] == b'{'
}

/// Upload a LoRA you already have (trained elsewhere, or on another Outpost) into your own
/// LoRA folder (only you see it), and optionally make it one of your characters. The body
/// is the file itself; the reply's `file` (`<account id>/<name>`) is what packs take.
#[utoipa::path(post, path = "/api/loras", tag = "create", security(("bearer" = [])),
    params(LoraUpload),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "The .safetensors file"),
    responses((status = 201, description = "Saved", body = LoraUploaded),
        (status = 400, description = "Not a safetensors file, or a bad name", body = ErrorBody),
        (status = 403, description = "An app key", body = ErrorBody),
        (status = 409, description = "A LoRA with that name exists (send replace=true)", body = ErrorBody),
        (status = 413, description = "Bigger than 4 GB", body = ErrorBody)))]
pub async fn upload_lora(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<LoraUpload>,
    body: Body,
) -> ApiResult<(StatusCode, Json<LoraUploaded>)> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    auth.require_person("Adding a LoRA")?;
    let file = lora_file_name(&q.name)?;
    // Check the character first, so a mistake there doesn't waste a long upload.
    let character = match (q.character.as_deref().map(str::trim).filter(|c| !c.is_empty()), q.trigger.as_deref()) {
        (Some(name), Some(trigger)) => {
            let t = trigger.trim();
            if !(2..=40).contains(&t.chars().count()) || name.chars().count() > 60 {
                return Err(ApiError::bad_request(
                    "Give the character a name of up to 60 letters and a trigger word of 2 to 40.",
                ));
            }
            Some((name.to_string(), t.to_string()))
        }
        (Some(_), None) => return Err(ApiError::bad_request("A character needs its trigger word.")),
        _ => None,
    };
    // Into my own LoRA folder: only I see and use it.
    let dir = state.assets.account_loras_dir(&auth.account.id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::conflict(format!("Couldn't make the LoRA folder: {e}")))?;
    let target = dir.join(&file);
    let name = file.clone();
    let file = format!("{}/{name}", auth.account.id);
    if target.exists() && !q.replace.unwrap_or(false) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "exists",
            format!("You already have a LoRA called {file}. Replace it, or rename yours."),
        ));
    }
    // Into a hidden part file first, so a half-finished upload never shows up as a LoRA.
    let part = dir.join(format!(".upload-{}.part", uuid::Uuid::new_v4()));
    let result: Result<u64, ApiError> = async {
        let mut out = tokio::fs::File::create(&part)
            .await
            .map_err(|e| ApiError::conflict(format!("Couldn't save the LoRA: {e}")))?;
        let mut stream = body.into_data_stream();
        let mut written: u64 = 0;
        let mut start: Vec<u8> = Vec::with_capacity(16);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| ApiError::bad_request("The upload was interrupted."))?;
            written += chunk.len() as u64;
            if written > MAX_LORA_BYTES {
                return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "That's bigger than 4 GB, which no LoRA should be."));
            }
            if start.len() < 16 {
                start.extend(chunk.iter().take(16 - start.len()));
            }
            out.write_all(&chunk)
                .await
                .map_err(|e| ApiError::conflict(format!("Couldn't save the LoRA (is the disk full?): {e}")))?;
        }
        out.flush().await.ok();
        if !looks_like_safetensors(&start, written) {
            return Err(ApiError::bad_request("That isn't a .safetensors file (or it's damaged)."));
        }
        Ok(written)
    }
    .await;
    let bytes = match result {
        Ok(b) => b,
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
    };
    if let Err(e) = tokio::fs::rename(&part, &target).await {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(ApiError::conflict(format!("Couldn't save the LoRA: {e}")));
    }
    tracing::info!(%file, bytes, account = %auth.account.name, "added a LoRA");
    let character = match character {
        Some((name, trigger)) => Some(
            outpost_core::training::save_character(
                &state.pool,
                &auth.account.id,
                outpost_core::training::NewCharacter {
                    name: &name,
                    trigger_word: &trigger,
                    lora_file: &file,
                    training_id: None,
                    step: None,
                    subject: q.subject.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                    style: None,
                },
            )
            .await?,
        ),
        None => None,
    };
    outpost_core::audit::log_quiet(&state.pool, Some(&auth.account.id), Some(&auth.session.id), "lora_upload", Some(&file)).await;
    Ok((StatusCode::CREATED, Json(LoraUploaded { file, bytes, character })))
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
    /// The helper PC running it, if not this server.
    pub on: Option<String>,
    /// Queued for a helper PC that's off or paused: its name.
    pub waiting_for: Option<String>,
}

async fn view(state: &AppState, job: Job) -> ApiResult<JobView> {
    let position = if job.status == JobStatus::Queued {
        Some(jobs::position(&state.pool, &job).await?)
    } else {
        None
    };
    let progress = state.queue.progress_of(&job.id).map(|r| r.progress);
    let on = state.queue.remote_run(&job.id).map(|r| r.helper);
    let waiting_for = if job.status == JobStatus::Queued {
        crate::helpers::waiting_for(state, &crate::helpers::local_packs(state).await, &job.pack_id)
    } else {
        None
    };
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
        on,
        waiting_for,
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
    let hw = crate::routes::models::measure_cached(&state).await;
    let helper = helper_for(&state, &pack, &hw);
    if helper.is_none() && !state.assets.status(&pack).ready {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "models_missing",
            format!("{}'s models aren't downloaded yet.", pack.name),
        ));
    }
    // (A helper PC's own ComfyUI is checked there.)
    if let Some(have) = state.comfy.node_types().await.filter(|_| helper.is_none()) {
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
        // One of my gallery pictures, by id (the picture tools): no need to send it back.
        let from_gallery = inputs.get(&input.key).and_then(|v| v.get("$media")).and_then(Value::as_str).map(str::to_string);
        let image = if let Some(id) = from_gallery {
            match gallery_image(&state, &auth.account.id, &id).await {
                Ok(image) => image,
                Err(e) => {
                    attachments::remove_files(&saved).await;
                    return Err(e);
                }
            }
        } else {
            let Some(encoded) = inputs.get(&input.key).and_then(Value::as_str).filter(|s| !s.is_empty()) else {
                inputs.remove(&input.key);
                continue;
            };
            attachments::decode_image(encoded)?
        };
        let rel = attachments::save_job_input(&state.config.files_dir(), &auth.account.id, &image).await?;
        saved.push(state.config.files_dir().join(&rel));
        uploaded.insert(input.key.clone(), "check.png".to_string());
        inputs.insert(input.key.clone(), serde_json::json!({ "$file": rel }));
    }
    // Validate by building once (the real build happens when the job runs).
    if let Err(e) = pack.build(&inputs, || 0, &uploaded, &visible_loras(&state, &auth.account.id).await) {
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

#[derive(Debug, Deserialize, IntoParams)]
pub struct FinishedQuery {
    /// Jobs that finished at or after this time (Unix seconds).
    pub since: i64,
    /// Wait up to this many seconds (at most 50) for one to finish, rather than answering
    /// "none" straight away: a phone holding this open hears about a job the moment it ends.
    pub wait: Option<u64>,
}

/// A finished job, worded for a notification.
#[derive(Debug, Serialize, ToSchema)]
pub struct FinishedJob {
    pub id: String,
    pub pack_id: String,
    /// `picture`, `video`, `model` or `training`.
    pub kind: String,
    pub status: JobStatus,
    /// "Your 3D model is ready", "Training didn't finish"...
    pub title: String,
    /// The prompt, the character's name, or what went wrong.
    pub body: String,
    /// It took a while (a minute or more) or is a kind that always does: worth a notification.
    pub long: bool,
    pub finished_at: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FinishedJobs {
    pub jobs: Vec<FinishedJob>,
    /// My jobs still waiting or running (so a phone knows to look again soon).
    pub unfinished: i64,
    /// The server's clock now (Unix seconds), for the next `since`.
    pub now: i64,
}

/// My jobs that finished since a time, for "it's ready" notifications. Stopped jobs are left out.
#[utoipa::path(get, path = "/api/jobs/finished", tag = "create", security(("bearer" = [])),
    params(FinishedQuery),
    responses((status = 200, description = "Finished jobs", body = FinishedJobs)))]
pub async fn finished_jobs(State(state): State<AppState>, auth: Auth, Query(q): Query<FinishedQuery>) -> ApiResult<Json<FinishedJobs>> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(q.wait.unwrap_or(0).min(50));
    loop {
        let now = outpost_core::now();
        let found: Vec<Job> = jobs::finished_since(&state.pool, &auth.account.id, q.since, 50)
            .await?
            .into_iter()
            .filter(|j| j.status != JobStatus::Cancelled)
            .collect();
        let unfinished = jobs::unfinished(&state.pool, &auth.account.id).await?;
        // Answer when something finished, nothing's left to wait for, or the wait is up.
        if !found.is_empty() || unfinished == 0 || std::time::Instant::now() >= until {
            let mut out = Vec::new();
            for job in found {
                out.push(finished_view(&state, job).await);
            }
            return Ok(Json(FinishedJobs { jobs: out, unfinished, now }));
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn finished_view(state: &AppState, job: Job) -> FinishedJob {
    let kind = if job.pack_id == crate::train::PACK_ID {
        "training"
    } else {
        match packs::find(&job.pack_id).map(|p| p.output.kind) {
            Some(packs::OutputKind::Model) => "model",
            Some(packs::OutputKind::Video) => "video",
            _ => "picture",
        }
    };
    let thing = match kind {
        "training" => "Training",
        "model" => "Your 3D model",
        "video" => "Your video",
        _ => "Your picture",
    };
    let ok = job.status == JobStatus::Done;
    let title = match (kind, ok) {
        ("training", true) => "Training finished".to_string(),
        (_, true) => format!("{thing} is ready"),
        (_, false) => format!("{thing} didn't finish"),
    };
    let what = if kind == "training" {
        match outpost_core::training::training_for_job(&state.pool, &job.id).await {
            Ok(t) => outpost_core::training::get_dataset(&state.pool, &t.account_id, &t.dataset_id)
                .await
                .map(|d| if ok { format!("{} is ready to try in Train.", d.name) } else { d.name })
                .unwrap_or_default(),
            Err(_) => String::new(),
        }
    } else {
        let values: Value = serde_json::from_str(&job.inputs).unwrap_or_default();
        values.get("prompt").and_then(Value::as_str).unwrap_or_default().to_string()
    };
    let body = if ok {
        what
    } else {
        let why = job.error.clone().unwrap_or_else(|| "It failed.".into());
        if what.is_empty() { why } else { format!("{what}: {why}") }
    };
    let took = job.finished_at.unwrap_or(0) - job.started_at.unwrap_or(job.created_at);
    FinishedJob {
        long: took >= 60 || kind != "picture",
        body: body.chars().take(160).collect(),
        id: job.id,
        pack_id: job.pack_id,
        kind: kind.into(),
        status: job.status,
        title,
        finished_at: job.finished_at.unwrap_or(0),
    }
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
    /// The helper PC running it, if not this server ("Dan's PC").
    pub on: Option<String>,
    /// Queued for a workflow only a helper PC can run, and it's off or paused: its name.
    pub waiting_for: Option<String>,
}

/// The line for the graphics card, everyone's jobs in the order they run. Other
/// people's jobs show only whose they are and how far along, never what they make.
#[utoipa::path(get, path = "/api/queue", tag = "create", security(("bearer" = [])),
    responses((status = 200, description = "The queue", body = [LineView])))]
pub async fn queue(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<LineView>>> {
    let here = crate::helpers::local_packs(&state).await;
    let out = jobs::line(&state.pool)
        .await?
        .into_iter()
        .map(|e| {
            let mine = e.account_id == auth.account.id;
            let live = state.queue.progress_of(&e.job_id);
            let live = live.as_ref();
            let on = state.queue.remote_run(&e.job_id).map(|r| r.helper);
            let waiting_for =
                (e.status == JobStatus::Queued).then(|| crate::helpers::waiting_for(&state, &here, &e.pack_id)).flatten();
            LineView {
                on,
                waiting_for,
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

/// One of an account's gallery pictures, as a job input.
async fn gallery_image(state: &AppState, account_id: &str, id: &str) -> ApiResult<attachments::ImageUpload> {
    let (_, path) = media::get(&state.pool, &state.config.files_dir(), account_id, id)
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That picture isn't in your gallery."))?;
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That picture's file is missing."))?;
    let mime = attachments::sniff_image(&data)
        .ok_or_else(|| ApiError::bad_request("That's a video, not a picture."))?;
    Ok(attachments::ImageUpload { data, mime })
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

/// A 3D model ready for a slicer (Bambu Studio, PrusaSlicer, Cura): a .3mf, upright, flat on
/// the bed. Made when the model was (with "Ready for 3D printing" on), or now, at 100 mm tall.
#[utoipa::path(get, path = "/api/media/{id}/print", tag = "create", security(("bearer" = [])),
    params(("id" = String, Path, description = "Media id")),
    responses((status = 200, description = "The .3mf file"), (status = 400, description = "Not a 3D model", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_media_print(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    let (m, path) = find_media(&state, &auth, &id, true).await?;
    if m.kind != "model" {
        return Err(ApiError::bad_request("Only 3D models can be made ready for printing."));
    }
    let print = crate::print3d::sidecar(&path);
    if !tokio::fs::try_exists(&print).await.unwrap_or(false) {
        let title = m.prompt.clone().unwrap_or_else(|| "Outpost 3D model".into());
        let pack = m.pack_id.as_deref().and_then(packs::find);
        crate::print3d::make(path.clone(), crate::print3d::Options::defaults(pack.as_ref()), title)
            .await
            .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "not_printable", e))?;
    }
    let filaments = tokio::fs::read_to_string(crate::print3d::filaments_file(&path)).await.unwrap_or_default();
    let data = tokio::fs::read(&print)
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That file is missing."))?;
    let short: String = m.id.chars().take(8).collect();
    Response::builder()
        .header(header::CONTENT_TYPE, "model/3mf")
        // Its filament colours, most used first (`#RRGGBB,...`; empty for one colour).
        .header("x-outpost-filaments", filaments.trim())
        .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "x-outpost-filaments")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"outpost-{short}.3mf\""))
        .header(header::CACHE_CONTROL, "private, no-cache")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(data))
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Could not send the file."))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NameMedia {
    /// The new name; empty clears it.
    pub name: String,
}

/// Name one of my gallery items (the owner: anyone's). The name shows in the gallery, is
/// found by search, and is used for the file when it's saved.
#[utoipa::path(patch, path = "/api/media/{id}", tag = "create", security(("bearer" = [])),
    params(("id" = String, Path, description = "Media id")),
    request_body = NameMedia,
    responses((status = 200, description = "Named", body = Media), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn name_media(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(req): Json<NameMedia>) -> ApiResult<Json<Media>> {
    let (mut m, _) = find_media(&state, &auth, &id, false).await?;
    m.name = media::set_name(&state.pool, &id, &req.name).await?;
    Ok(Json(m))
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

/// The largest gallery item brought over from another Outpost (videos included).
pub const MAX_IMPORT_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Debug, Deserialize, IntoParams)]
pub struct MediaImport {
    /// Its file name, for the type (`.png`, `.jpg`, `.webp`, `.gif`, `.mp4`, `.webm`).
    pub name: String,
    pub prompt: Option<String>,
    pub pack_id: Option<String>,
    pub seed: Option<u64>,
    /// When it was made (Unix seconds); kept, so the gallery stays in order.
    pub created_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MediaImported {
    /// False when the same item was copied before (nothing added).
    pub added: bool,
    pub media: Option<Media>,
}

/// Bring a picture or video over from another Outpost into my gallery, with its prompt,
/// seed and date. The body is the file. The same item twice is only kept once.
#[utoipa::path(post, path = "/api/media/import", tag = "create", security(("bearer" = [])),
    params(MediaImport),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "The picture or video"),
    responses((status = 200, description = "Added, or already there", body = MediaImported),
        (status = 400, description = "Not a picture or video the gallery keeps", body = ErrorBody),
        (status = 403, description = "An app key", body = ErrorBody)))]
pub async fn import_media(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<MediaImport>,
    body: Body,
) -> ApiResult<Json<MediaImported>> {
    auth.require_person("Copying pictures in")?;
    if media::kind_of(&q.name).is_none() {
        return Err(ApiError::bad_request(format!("{} isn't a picture or video the gallery keeps.", q.name)));
    }
    let data = axum::body::to_bytes(body, MAX_IMPORT_BYTES)
        .await
        .map_err(|_| ApiError::bad_request("That file is too big (over 1 GB), or the copy was interrupted."))?;
    if data.is_empty() {
        return Err(ApiError::bad_request("That file is empty."));
    }
    let now = outpost_core::now();
    let created_at = q.created_at.filter(|t| (946_684_800..=now + 86_400).contains(t));
    if let Some(at) = created_at {
        if media::has_copy(&state.pool, &auth.account.id, at, data.len() as i64).await? {
            return Ok(Json(MediaImported { added: false, media: None }));
        }
    }
    let tidy = |v: &Option<String>| v.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(|t| t.chars().take(4000).collect::<String>());
    let (prompt, pack_id) = (tidy(&q.prompt), tidy(&q.pack_id));
    let m = media::add(
        &state.pool,
        &state.config.files_dir(),
        media::NewMedia {
            account_id: &auth.account.id,
            job_id: None,
            pack_id: pack_id.as_deref(),
            prompt: prompt.as_deref(),
            seed: q.seed,
            source_name: &q.name,
            data: &data,
            created_at,
        },
    )
    .await?;
    Ok(Json(MediaImported { added: true, media: Some(m) }))
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
    fn lora_names_and_files() {
        assert_eq!(lora_file_name("kat.safetensors").unwrap(), "kat.safetensors");
        assert_eq!(lora_file_name(" Kat v2.safetensors ").unwrap(), "Kat_v2.safetensors");
        assert_eq!(lora_file_name("../../etc/x.safetensors").unwrap(), "etc_x.safetensors");
        assert!(lora_file_name("kat.ckpt").is_err());
        assert!(lora_file_name(".safetensors").is_err());
        let mut good = 2u64.to_le_bytes().to_vec();
        good.extend(b"{}");
        assert!(looks_like_safetensors(&good, 10));
        assert!(!looks_like_safetensors(b"PK\x03\x04 not a lora", 100));
        assert!(!looks_like_safetensors(&good, 5));
    }

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

// ------------------------------------------------------------------ gallery search

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    /// What to look for, e.g. "dog on a beach".
    pub q: String,
    pub limit: Option<usize>,
}

/// A gallery item a search found, with what Outpost sees in it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Found {
    #[serde(flatten)]
    pub media: Media,
    /// The picture's description (written for search), once it has one.
    pub description: Option<String>,
}

/// Search my gallery by what's in the pictures (and their prompts), best match first.
#[utoipa::path(get, path = "/api/gallery/search", tag = "create", security(("bearer" = [])),
    params(SearchQuery),
    responses((status = 200, description = "Matches", body = [Found])))]
pub async fn search_gallery(State(state): State<AppState>, auth: Auth, Query(q): Query<SearchQuery>) -> ApiResult<Json<Vec<Found>>> {
    let query = q.q.trim();
    if query.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let model = crate::memory::embedding_model(&state).await;
    let items = outpost_core::media_index::all(&state.pool, &auth.account.id, model.as_deref()).await?;
    let qv = match &model {
        Some(m) => crate::memory::embed(&state, m, &[query.to_string()]).await.and_then(|mut v| v.pop()),
        None => None,
    };
    let found = outpost_core::media_index::search(&items, query, qv.as_deref(), q.limit.unwrap_or(60).clamp(1, 200));
    Ok(Json(
        found
            .into_iter()
            .map(|(_, it)| Found { media: it.media.clone(), description: it.description.clone() })
            .collect(),
    ))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SearchStatus {
    /// Pictures are being described for search (the owner can turn it off).
    pub enabled: bool,
    /// The vision model that describes them; none installed means search uses prompts only.
    pub describer: Option<String>,
    /// The embedding model that finds by meaning; none means search matches words.
    pub embedder: Option<String>,
    /// Pictures described, of all pictures on this Outpost.
    pub described: i64,
    pub total: i64,
    /// Still to describe.
    pub waiting: i64,
    /// A vision model to get when there's none (with its download, if it's downloading).
    pub get_describer: Option<Suggested>,
    /// The embedding model to get when there's none.
    pub get_embedder: Option<Suggested>,
}

/// A model worth downloading, from the catalogue.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Suggested {
    pub model: String,
    pub name: String,
    pub size_gb: f32,
    /// Downloading now: how far (0-100, once known).
    pub downloading: bool,
    pub percent: Option<f32>,
    /// Why the last download failed.
    pub error: Option<String>,
}

fn suggest(state: &AppState, choices: &[&str], hw: &outpost_engines::hardware::Hardware) -> Option<Suggested> {
    let entry = choices
        .iter()
        .filter_map(|m| outpost_engines::catalog::CATALOG.iter().find(|e| e.model == *m))
        .find(|e| outpost_engines::catalog::fit(e.size_gb, hw) == Fit::Fits)
        .or_else(|| outpost_engines::catalog::CATALOG.iter().find(|e| Some(&e.model) == choices.last()))?;
    let pull = state.models.pull_states().into_iter().find(|p| p.model == entry.model);
    Some(Suggested {
        model: entry.model.into(),
        name: entry.name.into(),
        size_gb: entry.size_gb,
        downloading: pull.as_ref().is_some_and(|p| !p.done),
        percent: pull.as_ref().filter(|p| !p.done).and_then(|p| p.percent),
        error: pull.and_then(|p| p.error),
    })
}

/// How far describing pictures for search has got.
#[utoipa::path(get, path = "/api/gallery/search/status", tag = "create", security(("bearer" = [])),
    responses((status = 200, description = "Status", body = SearchStatus)))]
pub async fn search_status(State(state): State<AppState>, _auth: Auth) -> ApiResult<Json<SearchStatus>> {
    let (described, total, waiting) = outpost_core::media_index::progress(&state.pool).await?;
    let describer = crate::gallery_index::describer(&state).await;
    let embedder = crate::memory::embedding_model(&state).await;
    let hw = crate::routes::models::measure_cached(&state).await;
    Ok(Json(SearchStatus {
        enabled: crate::gallery_index::enabled(&state).await,
        get_describer: if describer.is_none() { suggest(&state, &crate::gallery_index::SUGGESTED_DESCRIBERS, &hw) } else { None },
        get_embedder: if embedder.is_none() { suggest(&state, &[crate::gallery_index::EMBEDDER], &hw) } else { None },
        describer,
        embedder,
        described,
        total,
        waiting,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchSettings {
    pub enabled: bool,
}

/// Turn describing pictures for search on or off (owner).
#[utoipa::path(put, path = "/api/gallery/search/settings", tag = "create", security(("bearer" = [])),
    request_body = SearchSettings,
    responses((status = 200, description = "Saved", body = SearchStatus), (status = 403, description = "Not an owner", body = ErrorBody)))]
pub async fn search_settings(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<SearchSettings>,
) -> ApiResult<Json<SearchStatus>> {
    auth.require_owner()?;
    outpost_core::settings::set_bool(&state.pool, outpost_core::settings::GALLERY_DESCRIBE, req.enabled).await?;
    search_status(State(state), auth).await
}
