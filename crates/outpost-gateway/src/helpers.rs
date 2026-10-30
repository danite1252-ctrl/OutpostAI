//! Helper PCs: a desktop app using this server can lend its PC's graphics card for Create
//! jobs (ComfyUI only) that are too big for the server's own card.
//!
//! - The helper checks in every few seconds (`POST /api/helpers/poll`) with its card's memory,
//!   the workflows it has downloaded, and whether it's paused (gaming). That's the registry.
//! - The server keeps every job it can run itself ([`local_packs`]: downloaded here and fitting
//!   this card). A job for a workflow only a helper can run waits in the queue until a helper
//!   that has it ready asks for work; it's then marked running and handed over.
//! - The helper fetches the job's pictures (and LoRA) from the server, runs the job in its own
//!   ComfyUI, reports progress (and learns of a cancel), uploads each result into the owner's
//!   gallery, and says when it's done. A helper that goes quiet mid-job fails the job.
//!
//! Only an owner's paired device can be a helper (jobs from everyone may run on it).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use outpost_core::accounts::Role;
use outpost_core::attachments;
use outpost_core::jobs::{self, JobStatus};
use outpost_core::media::{self, NewMedia};
use outpost_engines::catalog::Fit;
use outpost_engines::comfy::JobProgress;
use outpost_engines::packs::{self, InputKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::create::{RemoteRun, Running};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// A helper that hasn't checked in for this long is offline.
const ONLINE_FOR: Duration = Duration::from_secs(20);
/// A job on a helper that hasn't reported for this long has failed.
const SILENT_FOR: Duration = Duration::from_secs(120);

/// One helper PC, as it last checked in.
#[derive(Debug, Clone)]
pub struct Helper {
    pub session_id: String,
    /// Its paired-device name, e.g. "Dan's gaming PC".
    pub name: String,
    pub vram_mb: u64,
    pub ram_mb: u64,
    /// Workflows downloaded there (and runnable by its ComfyUI).
    pub ready: Vec<String>,
    pub paused: bool,
    pub last_seen: Instant,
}

impl Helper {
    pub fn online(&self) -> bool {
        self.last_seen.elapsed() < ONLINE_FOR
    }

    /// Can it run this pack: downloaded there, and within its card's memory.
    pub fn can_run(&self, pack: &packs::Pack) -> bool {
        self.ready.iter().any(|p| p == &pack.id)
            && packs::fit(pack.vram_gb, self.vram_mb as f32 / 1024.0, self.ram_mb as f32 / 1024.0) == Fit::Fits
    }
}

/// Every helper seen since the server started, by its sign-in.
#[derive(Clone, Default)]
pub struct Helpers {
    inner: Arc<Mutex<HashMap<String, Helper>>>,
}

impl Helpers {
    pub fn all(&self) -> Vec<Helper> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).values().cloned().collect()
    }

    fn put(&self, h: Helper) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(h.session_id.clone(), h);
    }

    /// The helper that would run `pack` (an online, unpaused one first), if any can.
    pub fn for_pack(&self, pack: &packs::Pack) -> Option<Helper> {
        let mut able: Vec<Helper> = self.all().into_iter().filter(|h| h.can_run(pack)).collect();
        able.sort_by_key(|h| (!(h.online() && !h.paused), std::cmp::Reverse(h.vram_mb)));
        able.into_iter().next()
    }
}

/// Packs this server runs itself: downloaded here and within its card (plus training).
/// Others wait for a helper, unless no helper can run them either, in which case they're
/// tried here as before (slowly, spilling into system memory).
pub async fn local_packs(state: &AppState) -> Vec<String> {
    let hw = crate::routes::models::measure_cached(state).await;
    let mut out = vec![crate::train::PACK_ID.to_string()];
    for p in packs::built_in() {
        let ready_here = state.assets.status(&p).ready;
        let fits_here = crate::routes::create::pack_fit(p.vram_gb, &hw) == Fit::Fits;
        if ready_here && (fits_here || state.helpers.for_pack(&p).is_none()) {
            out.push(p.id);
        }
    }
    out
}

/// The helper a queued job waits for, when only a helper can run it and that one is off or
/// paused (`here` is [`local_packs`]).
pub fn waiting_for(state: &AppState, here: &[String], pack_id: &str) -> Option<String> {
    if here.iter().any(|p| p == pack_id) {
        return None;
    }
    let pack = packs::find(pack_id)?;
    state.helpers.for_pack(&pack).filter(|h| !h.online() || h.paused).map(|h| h.name)
}

fn require_helper(auth: &Auth) -> Result<(), ApiError> {
    if auth.account.role != Role::Owner {
        return Err(ApiError::forbidden("Only an owner's PC can be a helper."));
    }
    auth.require_person("Being a helper")
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PollRequest {
    /// The helper's largest graphics card, in MiB.
    pub vram_mb: u64,
    #[serde(default)]
    pub ram_mb: u64,
    /// Workflows downloaded on the helper.
    #[serde(default)]
    pub ready_packs: Vec<String>,
    /// Paused (e.g. while gaming): check in, but take no jobs.
    #[serde(default)]
    pub paused: bool,
    /// Busy with a job already: don't hand it another.
    #[serde(default)]
    pub busy: bool,
}

/// A job for a helper to run.
#[derive(Debug, Serialize, ToSchema)]
pub struct HelperJob {
    pub job_id: String,
    pub pack_id: String,
    /// The pack's input values (pictures as `{"$file": ...}`: fetch each with `input/{key}`).
    #[schema(value_type = Object)]
    pub inputs: Value,
    /// Input keys that are pictures to fetch.
    pub images: Vec<String>,
    /// The LoRA it uses, if any: its name as the pack sees it (`<account>/<file>`); fetch
    /// it with `lora`.
    pub lora: Option<String>,
    pub lora_bytes: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PollResponse {
    /// A job to run now, if there's one for this helper.
    pub job: Option<HelperJob>,
}

/// A helper PC checks in (and asks for work). Owner's paired device only.
#[utoipa::path(post, path = "/api/helpers/poll", tag = "helpers", security(("bearer" = [])),
    request_body = PollRequest,
    responses((status = 200, description = "Maybe a job", body = PollResponse), (status = 403, description = "Not an owner", body = ErrorBody)))]
pub async fn poll(State(state): State<AppState>, auth: Auth, Json(req): Json<PollRequest>) -> ApiResult<Json<PollResponse>> {
    require_helper(&auth)?;
    let helper = Helper {
        session_id: auth.session.id.clone(),
        name: auth.session.label.clone(),
        vram_mb: req.vram_mb,
        ram_mb: req.ram_mb,
        ready: req.ready_packs.into_iter().filter(|p| packs::find(p).is_some()).collect(),
        paused: req.paused,
        last_seen: Instant::now(),
    };
    state.helpers.put(helper.clone());
    if req.paused || req.busy {
        return Ok(Json(PollResponse { job: None }));
    }
    // What only a helper should run, and this one can.
    let here = local_packs(&state).await;
    let mine: Vec<String> = packs::built_in()
        .into_iter()
        .filter(|p| !here.contains(&p.id) && helper.can_run(p))
        .map(|p| p.id)
        .collect();
    let Some(job) = jobs::take_next_of(&state.pool, &mine).await? else {
        return Ok(Json(PollResponse { job: None }));
    };
    let pack = packs::find(&job.pack_id).ok_or_else(|| ApiError::conflict("Unknown pack."))?;
    let inputs: Value = serde_json::from_str(&job.inputs).unwrap_or(Value::Object(Default::default()));
    let images: Vec<String> = pack
        .inputs
        .iter()
        .filter(|i| i.kind == InputKind::Image && inputs.get(&i.key).and_then(|v| v.get("$file")).is_some())
        .map(|i| i.key.clone())
        .collect();
    let lora = pack
        .inputs
        .iter()
        .filter(|i| i.kind == InputKind::Lora)
        .find_map(|i| inputs.get(&i.key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string));
    let lora_bytes = lora
        .as_deref()
        .and_then(|l| state.assets.lora_path(l))
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len());
    state.queue.remote_start(RemoteRun {
        run: Running {
            job_id: job.id.clone(),
            pack_id: job.pack_id.clone(),
            prompt_id: None,
            progress: JobProgress { phase: format!("Starting on {}", helper.name), ..Default::default() },
            cancelled: false,
            sampling: None,
        },
        helper: helper.name.clone(),
        session_id: helper.session_id.clone(),
        last_heard: Instant::now(),
    });
    tracing::info!(job = %job.id, pack = %job.pack_id, helper = %helper.name, "a helper PC took a job");
    Ok(Json(PollResponse {
        job: Some(HelperJob { job_id: job.id, pack_id: job.pack_id, inputs, images, lora, lora_bytes }),
    }))
}

/// The helper's own running job, or not found (so nobody else can see others' jobs).
fn my_run(state: &AppState, auth: &Auth, job_id: &str) -> Result<RemoteRun, ApiError> {
    state
        .queue
        .remote_run(job_id)
        .filter(|r| r.session_id == auth.session.id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That job isn't running on this helper."))
}

fn bytes(data: Vec<u8>, kind: &'static str) -> ApiResult<Response> {
    Response::builder()
        .header(header::CONTENT_TYPE, kind)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(data))
        .map_err(|_| ApiError::conflict("Couldn't send the file."))
}

/// One of the job's input pictures (to the helper running it).
#[utoipa::path(get, path = "/api/helpers/jobs/{id}/input/{key}", tag = "helpers", security(("bearer" = [])),
    params(("id" = String, Path, description = "Job id"), ("key" = String, Path, description = "Input key")),
    responses((status = 200, description = "The picture"), (status = 404, description = "Not this helper's job", body = ErrorBody)))]
pub async fn input(State(state): State<AppState>, auth: Auth, Path((id, key)): Path<(String, String)>) -> ApiResult<Response> {
    require_helper(&auth)?;
    my_run(&state, &auth, &id)?;
    let job = jobs::get_any(&state.pool, &id).await?;
    let values: Value = serde_json::from_str(&job.inputs).unwrap_or_default();
    let rel = values
        .get(&key)
        .and_then(|v| v.get("$file"))
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "No such picture."))?;
    let data = attachments::read_job_input(&state.config.files_dir(), &job.account_id, rel).await?;
    bytes(data, "application/octet-stream")
}

/// The job's LoRA file (to the helper running it).
#[utoipa::path(get, path = "/api/helpers/jobs/{id}/lora", tag = "helpers", security(("bearer" = [])),
    params(("id" = String, Path, description = "Job id")),
    responses((status = 200, description = "The LoRA"), (status = 404, description = "Not this helper's job", body = ErrorBody)))]
pub async fn lora(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    require_helper(&auth)?;
    my_run(&state, &auth, &id)?;
    let job = jobs::get_any(&state.pool, &id).await?;
    let pack = packs::find(&job.pack_id).ok_or_else(|| ApiError::conflict("Unknown pack."))?;
    let values: Value = serde_json::from_str(&job.inputs).unwrap_or_default();
    let name = pack
        .inputs
        .iter()
        .filter(|i| i.kind == InputKind::Lora)
        .find_map(|i| values.get(&i.key).and_then(Value::as_str))
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "This job uses no LoRA."))?;
    // Only one the job's owner may use.
    if !crate::routes::create::visible_loras(&state, &job.account_id).await.iter().any(|l| l == name) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "That LoRA is gone."));
    }
    let path = state.assets.lora_path(name).ok_or_else(|| ApiError::conflict("Bad LoRA name."))?;
    let data = tokio::fs::read(&path).await.map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That LoRA is gone."))?;
    bytes(data, "application/octet-stream")
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProgressReply {
    /// The owner cancelled: stop the job.
    pub cancel: bool,
}

/// How far the job has got; the reply says if it was cancelled.
#[utoipa::path(post, path = "/api/helpers/jobs/{id}/progress", tag = "helpers", security(("bearer" = [])),
    params(("id" = String, Path, description = "Job id")),
    request_body = JobProgress,
    responses((status = 200, description = "Carry on, or stop", body = ProgressReply)))]
pub async fn progress(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(p): Json<JobProgress>,
) -> ApiResult<Json<ProgressReply>> {
    require_helper(&auth)?;
    my_run(&state, &auth, &id)?;
    let cancel = state.queue.remote_update(&id, p).unwrap_or(true);
    Ok(Json(ProgressReply { cancel }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ResultQuery {
    /// The file's name, for its type (`.png`, `.mp4`...).
    pub name: String,
    /// The seed it used.
    pub seed: Option<u64>,
}

/// One result of the job, into its owner's gallery.
#[utoipa::path(post, path = "/api/helpers/jobs/{id}/result", tag = "helpers", security(("bearer" = [])),
    params(("id" = String, Path, description = "Job id"), ResultQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses((status = 201, description = "Saved"), (status = 404, description = "Not this helper's job", body = ErrorBody)))]
pub async fn result(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<ResultQuery>,
    body: Body,
) -> ApiResult<StatusCode> {
    require_helper(&auth)?;
    my_run(&state, &auth, &id)?;
    let job = jobs::get_any(&state.pool, &id).await?;
    let data = axum::body::to_bytes(body, crate::routes::create::MAX_IMPORT_BYTES)
        .await
        .map_err(|_| ApiError::bad_request("The result was too big, or the upload was cut off."))?;
    if let Some(seed) = q.seed {
        let _ = jobs::set_seed(&state.pool, &id, seed).await;
    }
    let values: Value = serde_json::from_str(&job.inputs).unwrap_or_default();
    media::add(
        &state.pool,
        &state.config.files_dir(),
        NewMedia {
            account_id: &job.account_id,
            job_id: Some(&job.id),
            pack_id: Some(&job.pack_id),
            prompt: values.get("prompt").and_then(Value::as_str),
            seed: q.seed,
            source_name: &q.name,
            data: &data[..],
            created_at: None,
        },
    )
    .await?;
    Ok(StatusCode::CREATED)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FinishRequest {
    /// It worked (results were sent).
    pub ok: bool,
    /// Stopped because it was cancelled.
    #[serde(default)]
    pub cancelled: bool,
    pub error: Option<String>,
}

/// The helper is done with the job.
#[utoipa::path(post, path = "/api/helpers/jobs/{id}/finish", tag = "helpers", security(("bearer" = [])),
    params(("id" = String, Path, description = "Job id")),
    request_body = FinishRequest,
    responses((status = 204, description = "Recorded")))]
pub async fn finish(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<FinishRequest>,
) -> ApiResult<StatusCode> {
    require_helper(&auth)?;
    let run = my_run(&state, &auth, &id)?;
    let (status, error) = if req.cancelled || run.run.cancelled {
        (JobStatus::Cancelled, None)
    } else if req.ok {
        (JobStatus::Done, None)
    } else {
        let e = req.error.unwrap_or_else(|| "It failed.".into());
        (JobStatus::Failed, Some(format!("On {}: {e}", run.helper)))
    };
    end(&state, &id, status, error.as_deref()).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn end(state: &AppState, id: &str, status: JobStatus, error: Option<&str>) {
    state.queue.remote_end(id);
    if let Err(e) = jobs::finish(&state.pool, id, status, error).await {
        tracing::warn!(error = %e, "couldn't record a helper's job");
    }
    crate::webhooks::job_finished(state, id).await;
    if let Ok(job) = jobs::get_any(&state.pool, id).await {
        crate::create::remove_inputs(state, &job).await;
    }
}

/// Fail jobs whose helper went quiet (switched off mid-job, lost its connection).
pub async fn watchdog(state: AppState) {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;
        for r in state.queue.remote_runs() {
            if r.last_heard.elapsed() > SILENT_FOR {
                tracing::warn!(job = %r.run.job_id, helper = %r.helper, "a helper stopped answering mid-job");
                let msg = format!("{} stopped answering while making this (switched off, or lost its connection).", r.helper);
                end(&state, &r.run.job_id, JobStatus::Failed, Some(&msg)).await;
            }
        }
    }
}

/// Helpers as the apps see them.
#[derive(Debug, Serialize, ToSchema)]
pub struct HelperView {
    pub name: String,
    pub online: bool,
    pub paused: bool,
    pub vram_mb: u64,
    /// Workflows it has ready.
    pub ready: Vec<String>,
    /// What it's making now (the job's pack), if anything.
    pub busy_with: Option<String>,
}

/// The helper PCs (everyone may see them: "Waiting for Dan's PC").
#[utoipa::path(get, path = "/api/helpers", tag = "helpers", security(("bearer" = [])),
    responses((status = 200, description = "Helpers", body = [HelperView])))]
pub async fn list(State(state): State<AppState>, _auth: Auth) -> Json<Vec<HelperView>> {
    let runs = state.queue.remote_runs();
    let mut out: Vec<HelperView> = state
        .helpers
        .all()
        .into_iter()
        .map(|h| HelperView {
            busy_with: runs.iter().find(|r| r.session_id == h.session_id).map(|r| r.run.pack_id.clone()),
            online: h.online(),
            paused: h.paused,
            vram_mb: h.vram_mb,
            ready: h.ready,
            name: h.name,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Json(out)
}
