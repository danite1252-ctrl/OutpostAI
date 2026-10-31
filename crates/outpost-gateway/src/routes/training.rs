//! Training Studio: datasets and their captions, LoRA trainings in the GPU queue,
//! picking the best checkpoint, and the characters that come out of it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use base64::Engine;
use outpost_core::jobs::{self, JobStatus};
use outpost_core::sessions::SessionKind;
use outpost_core::training::{self, Character, Dataset, DatasetImage, NewCharacter, Training};
use outpost_core::trigger::{self, TriggerCheck};
use outpost_engines::catalog::Badge;
use outpost_engines::trainer::{self, Checkpoint};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{IntoParams, ToSchema};

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::train::{self as train_job, PACK_ID};
use crate::AppState;

/// What the captioner is doing for one dataset.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct CaptionRun {
    pub running: bool,
    pub done: u32,
    pub total: u32,
    pub failed: u32,
    pub model: String,
    /// Why it stopped early, if it did.
    pub error: Option<String>,
}

/// Captioning in progress, by dataset id.
pub type Captions = Arc<Mutex<HashMap<String, CaptionRun>>>;

fn captions_of(state: &AppState, dataset_id: &str) -> Option<CaptionRun> {
    state.captions.lock().unwrap_or_else(|p| p.into_inner()).get(dataset_id).cloned()
}

fn set_caption_run(state: &AppState, dataset_id: &str, f: impl FnOnce(&mut CaptionRun)) {
    let mut map = state.captions.lock().unwrap_or_else(|p| p.into_inner());
    f(map.entry(dataset_id.to_string()).or_default());
}

/// Training changes files and takes the GPU for hours: people only, not app keys.
fn require_person(auth: &Auth) -> Result<(), ApiError> {
    if auth.session.kind == SessionKind::App {
        return Err(ApiError::forbidden(
            "App keys can't use Training Studio. Use the desktop app or a paired phone.",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------- status

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TrainingStatus {
    /// The ai-toolkit folder in use.
    pub toolkit_dir: String,
    /// Why training can't run yet; `None` when ai-toolkit is ready.
    pub toolkit_problem: Option<String>,
    /// Installed models that can see pictures, for captions.
    pub caption_models: Vec<String>,
    /// Z-Image Turbo's models (for using a trained character) are downloaded.
    pub character_pack_ready: bool,
    /// Video memory the training needs, and this PC's.
    pub vram_needed_gb: f32,
    pub vram_gb: f32,
    /// Outpost's own install of ai-toolkit (the Install button).
    pub install: outpost_engines::TrainerInstall,
    /// The helper PC trainings run on, when this server can't train and one can.
    pub trains_on: Option<String>,
    /// Its card's memory, in GB.
    pub trains_on_vram_gb: Option<f32>,
}

/// Is this PC ready to train, and which models can write captions?
#[utoipa::path(get, path = "/api/training", tag = "training", security(("bearer" = [])),
    responses((status = 200, description = "Status", body = TrainingStatus)))]
pub async fn status(State(state): State<AppState>, _auth: Auth) -> Json<TrainingStatus> {
    let dir = train_job::toolkit_dir(&state).await;
    let caption_models = vision_models(&state).await;
    let hw = crate::routes::models::measure(&state).await;
    let character_pack_ready = outpost_engines::packs::built_in()
        .iter()
        .any(|p| p.task() == outpost_engines::packs::Task::Character && state.assets.status(p).ready);
    let helper = crate::helpers::trains_on(&state).await;
    Json(TrainingStatus {
        trains_on_vram_gb: helper.as_ref().map(|h| h.vram_mb as f32 / 1024.0),
        trains_on: helper.map(|h| h.name),
        toolkit_problem: trainer::check_install(&dir).err(),
        toolkit_dir: dir.to_string_lossy().to_string(),
        caption_models,
        character_pack_ready,
        vram_needed_gb: train_job::VRAM_GB,
        vram_gb: hw.vram_mb as f32 / 1024.0,
        install: state.engines.trainer_install(),
    })
}

/// Install ai-toolkit for training (owner; a few GB, in the background; follow it with
/// `GET /api/training`). Installing again updates it to the version Outpost pins.
#[utoipa::path(post, path = "/api/training/install", tag = "training", security(("bearer" = [])),
    responses((status = 202, description = "Installing"), (status = 403, description = "Not allowed", body = ErrorBody), (status = 409, description = "Already installing or training", body = ErrorBody)))]
pub async fn install_toolkit(
    State(state): State<AppState>,
    auth: Auth,
    headers: axum::http::HeaderMap,
) -> ApiResult<StatusCode> {
    auth.require_owner()?;
    auth.require_fresh_pin(&state, &headers).await?;
    if state.queue.training() {
        return Err(ApiError::conflict("A training is running; install after it finishes."));
    }
    state
        .engines
        .install_trainer()
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    outpost_core::audit::log_quiet(&state.pool, Some(&auth.account.id), Some(&auth.session.id), "toolkit_install", None).await;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) async fn vision_models(state: &AppState) -> Vec<String> {
    state
        .models
        .installed()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.badges.contains(&Badge::Vision) && !m.badges.contains(&Badge::Embedding))
        .map(|m| m.model)
        .collect()
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TrainingSettings {
    /// The folder ai-toolkit is installed in (holds `run.py` and `venv`).
    pub toolkit_dir: String,
}

/// Point Outpost at ai-toolkit (owner, desktop app).
#[utoipa::path(put, path = "/api/training", tag = "training", security(("bearer" = [])),
    request_body = TrainingSettings,
    responses((status = 200, description = "Saved", body = TrainingStatus), (status = 403, description = "Not allowed", body = ErrorBody)))]
pub async fn set_settings(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<TrainingSettings>,
) -> ApiResult<Json<TrainingStatus>> {
    auth.require_owner()?;
    auth.require_desktop("Choosing the ai-toolkit folder")?;
    outpost_core::settings::set(&state.pool, outpost_core::settings::AI_TOOLKIT_DIR, req.toolkit_dir.trim()).await?;
    Ok(status(State(state), auth).await)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct WordQuery {
    pub word: String,
}

/// Is this a safe trigger word? If not, why, and a safer one.
#[utoipa::path(get, path = "/api/training/trigger-check", tag = "training", security(("bearer" = [])),
    params(WordQuery),
    responses((status = 200, description = "Checked", body = TriggerCheck)))]
pub async fn trigger_check(_auth: Auth, Query(q): Query<WordQuery>) -> Json<TriggerCheck> {
    Json(trigger::check(&q.word))
}

// ---------------------------------------------------------------- datasets

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NewDataset {
    /// The character's name, e.g. "Kat".
    pub name: String,
    /// Leave out for a suggestion made from the name.
    pub trigger_word: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DatasetView {
    #[serde(flatten)]
    pub dataset: Dataset,
    pub trigger_check: TriggerCheck,
    pub images: Vec<DatasetImage>,
    pub captioning: Option<CaptionRun>,
    /// Test prompts that suit these pictures, made from their captions.
    pub suggested_prompts: Vec<String>,
    /// Newest first.
    pub trainings: Vec<TrainingView>,
}

async fn dataset_view(state: &AppState, auth: &Auth, id: &str) -> ApiResult<DatasetView> {
    let dataset = training::get_dataset(&state.pool, &auth.account.id, id).await?;
    let images = training::images(&state.pool, &auth.account.id, id).await?;
    let mut trainings = Vec::new();
    for t in training::trainings(&state.pool, &auth.account.id, id).await? {
        trainings.push(training_view(state, t).await?);
    }
    let captions: Vec<String> = images.iter().map(|i| i.caption.clone()).collect();
    Ok(DatasetView {
        suggested_prompts: training::suggest_prompts(&dataset.trigger_word, &captions),
        trigger_check: trigger::check(&dataset.trigger_word),
        captioning: captions_of(state, id),
        dataset,
        images,
        trainings,
    })
}

/// My datasets.
#[utoipa::path(get, path = "/api/datasets", tag = "training", security(("bearer" = [])),
    responses((status = 200, description = "Datasets", body = [Dataset])))]
pub async fn list_datasets(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<Dataset>>> {
    Ok(Json(training::list_datasets(&state.pool, &auth.account.id).await?))
}

/// Start a dataset for a new character.
#[utoipa::path(post, path = "/api/datasets", tag = "training", security(("bearer" = [])),
    request_body = NewDataset,
    responses((status = 201, description = "Made", body = DatasetView), (status = 400, description = "Not accepted", body = ErrorBody)))]
pub async fn create_dataset(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<NewDataset>,
) -> ApiResult<(StatusCode, Json<DatasetView>)> {
    require_person(&auth)?;
    let word = req
        .trigger_word
        .filter(|w| !w.trim().is_empty())
        .unwrap_or_else(|| trigger::from_name(&req.name));
    let d = training::create_dataset(&state.pool, &auth.account.id, &req.name, &word).await?;
    Ok((StatusCode::CREATED, Json(dataset_view(&state, &auth, &d.id).await?)))
}

/// A dataset with its pictures, captions and trainings.
#[utoipa::path(get, path = "/api/datasets/{id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id")),
    responses((status = 200, description = "Dataset", body = DatasetView), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_dataset(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<DatasetView>> {
    Ok(Json(dataset_view(&state, &auth, &id).await?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DatasetUpdate {
    pub name: Option<String>,
    pub trigger_word: Option<String>,
}

/// Rename a dataset or change its trigger word.
#[utoipa::path(patch, path = "/api/datasets/{id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id")),
    request_body = DatasetUpdate,
    responses((status = 200, description = "Updated", body = DatasetView), (status = 400, description = "Not accepted", body = ErrorBody)))]
pub async fn update_dataset(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<DatasetUpdate>,
) -> ApiResult<Json<DatasetView>> {
    require_person(&auth)?;
    training::update_dataset(&state.pool, &auth.account.id, &id, req.name.as_deref(), req.trigger_word.as_deref()).await?;
    Ok(Json(dataset_view(&state, &auth, &id).await?))
}

/// Delete a dataset, its pictures and its training runs. Characters made from it stay.
#[utoipa::path(delete, path = "/api/datasets/{id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id")),
    responses((status = 204, description = "Deleted"), (status = 409, description = "Training now", body = ErrorBody)))]
pub async fn delete_dataset(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    require_person(&auth)?;
    for t in training::trainings(&state.pool, &auth.account.id, &id).await? {
        let job = jobs::get_any(&state.pool, &t.job_id).await?;
        if !job.status.finished() {
            return Err(ApiError::conflict("It's training. Cancel the training first."));
        }
    }
    training::delete_dataset(&state.pool, &train_job::root(&state), &auth.account.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NewImage {
    /// The picture as base64 or a `data:image/...;base64,` URL (PNG, JPEG or WebP, up to 40 MB).
    pub image: String,
    #[serde(default)]
    pub caption: String,
}

fn decode(encoded: &str) -> Result<Vec<u8>, ApiError> {
    let raw = match encoded.split_once(";base64,") {
        Some((head, rest)) if head.starts_with("data:") => rest,
        _ => encoded,
    };
    if raw.len() > training::MAX_DATASET_IMAGE_BYTES / 3 * 4 + 8 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", "Pictures can be up to 40 MB each."));
    }
    let cleaned: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(cleaned.as_bytes())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid", "That picture couldn't be read."))
}

/// Add a picture to a dataset.
#[utoipa::path(post, path = "/api/datasets/{id}/images", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id")),
    request_body = NewImage,
    responses((status = 201, description = "Added", body = DatasetImage), (status = 400, description = "Not accepted", body = ErrorBody)))]
pub async fn add_image(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<NewImage>,
) -> ApiResult<(StatusCode, Json<DatasetImage>)> {
    require_person(&auth)?;
    let data = decode(&req.image)?;
    let img = training::add_image(&state.pool, &train_job::root(&state), &auth.account.id, &id, &data, &req.caption).await?;
    Ok((StatusCode::CREATED, Json(img)))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ImportRequest {
    /// A folder on the Outpost PC, e.g. `C:\dev\dataset\kat`.
    pub folder: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ImportResult {
    pub added: usize,
    /// Files left out, and why.
    pub skipped: Vec<String>,
}

/// Copy the pictures (and any `.txt` captions) from a folder on this PC (desktop app).
#[utoipa::path(post, path = "/api/datasets/{id}/import", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id")),
    request_body = ImportRequest,
    responses((status = 200, description = "Imported", body = ImportResult), (status = 400, description = "Not accepted", body = ErrorBody)))]
pub async fn import_folder(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<ImportRequest>,
) -> ApiResult<Json<ImportResult>> {
    auth.require_desktop("Adding pictures from a folder")?;
    let folder = PathBuf::from(req.folder.trim().trim_matches('"'));
    let (added, skipped) =
        training::import_folder(&state.pool, &train_job::root(&state), &auth.account.id, &id, &folder).await?;
    Ok(Json(ImportResult { added, skipped }))
}

/// A dataset picture.
#[utoipa::path(get, path = "/api/datasets/{id}/images/{image_id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id"), ("image_id" = String, Path, description = "Picture id")),
    responses((status = 200, description = "The picture"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_image(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, image_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (img, path) = training::image(&state.pool, &train_job::root(&state), &auth.account.id, &id, &image_id).await?;
    send_file(&path, &img.mime).await
}

async fn send_file(path: &std::path::Path, mime: &str) -> ApiResult<Response> {
    let data = tokio::fs::read(path)
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That file is missing."))?;
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(data))
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Could not send the file."))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CaptionUpdate {
    pub caption: String,
}

/// Edit a picture's caption.
#[utoipa::path(patch, path = "/api/datasets/{id}/images/{image_id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id"), ("image_id" = String, Path, description = "Picture id")),
    request_body = CaptionUpdate,
    responses((status = 200, description = "Saved", body = DatasetImage), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn set_caption(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, image_id)): Path<(String, String)>,
    Json(req): Json<CaptionUpdate>,
) -> ApiResult<Json<DatasetImage>> {
    require_person(&auth)?;
    Ok(Json(training::set_caption(&state.pool, &auth.account.id, &id, &image_id, &req.caption).await?))
}

/// Take a picture out of a dataset.
#[utoipa::path(delete, path = "/api/datasets/{id}/images/{image_id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id"), ("image_id" = String, Path, description = "Picture id")),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_image(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, image_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    require_person(&auth)?;
    training::delete_image(&state.pool, &train_job::root(&state), &auth.account.id, &id, &image_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- captions

/// What the vision model is asked for each picture. For a character, a caption must
/// describe only what changes from picture to picture. Whatever it names (say "fair
/// skin" or "brown hair") the model ties to those words instead of to the trigger
/// word, so the character comes out without it unless the prompt says it again. What
/// is left out (face, hair, skin, build) is what the trigger word learns.
pub const CAPTION_PROMPT: &str = "This picture is for teaching an image model one particular person, animal or character.
Describe ONLY what changes from picture to picture, as ONE line of comma-separated keywords, most important first:
what kind of subject it is in one or two words (e.g. woman, man, girl, dog, cartoon fox), clothing and accessories,
pose and expression, what they are doing, setting and background, lighting, camera angle and framing, picture type
(photo, selfie, cartoon, painting, 3D render).
Do NOT describe what makes them look like themselves: no face, eyes, nose, hair colour or hairstyle, skin tone or
complexion, age, body shape or build.
No full sentences, no names, no preamble, no quotes.";

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct CaptionRequest {
    /// A model with vision; leave out for the first one installed.
    pub model: Option<String>,
    /// Write new captions for pictures that already have one too.
    #[serde(default)]
    pub overwrite: bool,
}

/// Caption the dataset's pictures with a vision model, in the background. Follow it
/// in `GET /api/datasets/{id}` (`captioning`).
#[utoipa::path(post, path = "/api/datasets/{id}/caption", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id")),
    request_body = CaptionRequest,
    responses((status = 202, description = "Captioning", body = CaptionRun), (status = 400, description = "No vision model", body = ErrorBody), (status = 409, description = "Busy", body = ErrorBody)))]
pub async fn caption(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<CaptionRequest>,
) -> ApiResult<(StatusCode, Json<CaptionRun>)> {
    require_person(&auth)?;
    if state.queue.training() {
        return Err(ApiError::conflict("The graphics card is training right now. Caption once it's done."));
    }
    let dataset = training::get_dataset(&state.pool, &auth.account.id, &id).await?;
    let models = vision_models(&state).await;
    let model = match req.model.filter(|m| !m.is_empty()) {
        Some(m) if models.contains(&m) => m,
        Some(m) => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "model_no_vision",
                format!("{m} isn't installed or can't see pictures."),
            ))
        }
        None => models.into_iter().next().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "no_vision_model",
                "No model that can see pictures is installed. Get one with the eye badge in Models.",
            )
        })?,
    };
    if captions_of(&state, &id).is_some_and(|c| c.running) {
        return Err(ApiError::conflict("Already writing captions for this dataset."));
    }
    let todo: Vec<DatasetImage> = training::images(&state.pool, &auth.account.id, &id)
        .await?
        .into_iter()
        .filter(|i| req.overwrite || i.caption.is_empty())
        .collect();
    let run = CaptionRun {
        running: !todo.is_empty(),
        done: 0,
        total: todo.len() as u32,
        failed: 0,
        model: model.clone(),
        error: None,
    };
    set_caption_run(&state, &id, |c| *c = run.clone());
    if !todo.is_empty() {
        tokio::spawn(caption_all(state.clone(), auth.account.id.clone(), dataset, model, todo));
    }
    Ok((StatusCode::ACCEPTED, Json(run)))
}

pub(crate) async fn caption_all(state: AppState, account_id: String, dataset: Dataset, model: String, todo: Vec<DatasetImage>) {
    let root = train_job::root(&state);
    let url = format!("{}/api/chat", state.config.ollama_url.trim_end_matches('/'));
    let can_think = state
        .models
        .installed()
        .await
        .unwrap_or_default()
        .iter()
        .any(|m| m.model == model && m.badges.contains(&Badge::Thinking));
    let mut failures_in_a_row = 0;
    for img in todo {
        let path = training::dataset_dir(&root, &dataset.id).join(&img.file);
        let result = async {
            let data = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
            let mut body = json!({
                "model": model,
                "stream": false,
                "messages": [{ "role": "user", "content": CAPTION_PROMPT, "images": [b64] }],
                "options": { "temperature": 0.2 },
            });
            if can_think {
                body["think"] = json!(false);
            }
            let resp = tokio::time::timeout(
                std::time::Duration::from_secs(300),
                state.llm.post(&url).json(&body).send(),
            )
            .await
            .map_err(|_| "the model took too long".to_string())?
            .map_err(|_| "Ollama isn't running".to_string())?;
            if !resp.status().is_success() {
                return Err(resp.text().await.unwrap_or_default());
            }
            let v: Value = resp.json().await.map_err(|e| e.to_string())?;
            let text = v["message"]["content"].as_str().unwrap_or_default().to_string();
            Ok::<String, String>(clean_caption(&text))
        }
        .await;
        match result {
            Ok(text) if !text.is_empty() => {
                failures_in_a_row = 0;
                let _ = training::set_caption(&state.pool, &account_id, &dataset.id, &img.id, &text).await;
                set_caption_run(&state, &dataset.id, |c| c.done += 1);
            }
            other => {
                failures_in_a_row += 1;
                let why = other.err().unwrap_or_else(|| "empty caption".into());
                tracing::warn!(image = %img.id, error = %why, "couldn't caption a picture");
                set_caption_run(&state, &dataset.id, |c| {
                    c.failed += 1;
                    c.error = Some(why.chars().take(300).collect());
                });
                if failures_in_a_row >= 3 {
                    break;
                }
            }
        }
    }
    set_caption_run(&state, &dataset.id, |c| c.running = false);
}

/// The model's reply as one keyword line: no reasoning block, no blank lines, and
/// none of the keywords that describe what the character always looks like (the
/// model sometimes adds them anyway; see [`CAPTION_PROMPT`]).
pub fn clean_caption(text: &str) -> String {
    let t = strip_thinking(text);
    let line = t.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").trim_end_matches('.');
    line.split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty() && !is_identity(k))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A model's reply without any `<think>…</think>` reasoning block.
pub fn strip_thinking(text: &str) -> String {
    let mut t = text.to_string();
    while let (Some(a), Some(b)) = (t.find("<think>"), t.find("</think>")) {
        if b < a {
            break;
        }
        t.replace_range(a..b + "</think>".len(), "");
    }
    t
}

/// A keyword about the character's own look rather than the picture: skin, hair,
/// eyes, face shape, age, build. Hair accessories and expressions stay.
fn is_identity(keyword: &str) -> bool {
    let k = keyword.to_lowercase();
    let accessory = ["clip", "band", "tie", "pin", "bow", "net", "dryer", "brush", "cut ", "salon"];
    if k.contains("hair") && !accessory.iter().any(|a| k.contains(a)) {
        return true;
    }
    const LOOKS: &[&str] = &[
        "skin", "complexion", "freckle", "eyes", "eye colour", "eye color", "eyebrow", "face shape", "oval face",
        "round face", "facial features", "cheekbone", "jawline", "young adult", "middle-aged", "middle aged",
        "years old", "elderly", "teenage", "slender", "petite", "curvy", "body type", "fur colour", "fur color",
        "blonde", "brunette", "redhead",
    ];
    // "slim build", but not "school building".
    k.ends_with(" build") || LOOKS.iter().any(|w| k.contains(w))
}

// ---------------------------------------------------------------- trainings

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TrainRequest {
    /// Training steps (500 to 6000). 2000 suits 15 to 30 pictures.
    pub steps: Option<u32>,
    /// 1 to 4 test prompts drawn at each checkpoint; leave out for ones made from
    /// the captions.
    pub sample_prompts: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TrainingView {
    #[serde(flatten)]
    pub training: Training,
    pub status: JobStatus,
    pub error: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// Jobs ahead in the GPU queue (while queued).
    pub position: Option<i64>,
    pub progress: Option<outpost_engines::comfy::JobProgress>,
    /// Seconds left, once the pace is known.
    pub eta_secs: Option<u64>,
    /// Saved so far, oldest first; step 0 holds the test pictures from before training.
    pub checkpoints: Vec<Checkpoint>,
    /// The step that became a character, if one did.
    pub chosen_step: Option<i64>,
    /// The test prompts, in the order of each checkpoint's pictures.
    pub sample_prompts: Vec<String>,
}

async fn training_view(state: &AppState, t: Training) -> ApiResult<TrainingView> {
    let job = jobs::get_any(&state.pool, &t.job_id).await?;
    let position = if job.status == JobStatus::Queued {
        Some(jobs::position(&state.pool, &job).await?)
    } else {
        None
    };
    let live = state.queue.running().filter(|r| r.job_id == job.id);
    let run_dir = training::run_dir(&train_job::root(state), &t.id);
    let chosen_step = training::characters(&state.pool, Some(t.account_id.as_str()))
        .await?
        .into_iter()
        .find(|c| c.training_id.as_deref() == Some(t.id.as_str()))
        .and_then(|c| c.step);
    Ok(TrainingView {
        checkpoints: trainer::checkpoints(&run_dir, t.steps as u32),
        status: job.status,
        error: job.error,
        started_at: job.started_at,
        finished_at: job.finished_at,
        position,
        progress: live.as_ref().map(|r| r.progress.clone()),
        eta_secs: live.as_ref().and_then(|r| r.eta_secs()),
        chosen_step,
        sample_prompts: t.prompts(),
        training: t,
    })
}

/// Queue a LoRA training for this dataset. It runs when the graphics card is free
/// and takes it for an hour or more.
#[utoipa::path(post, path = "/api/datasets/{id}/train", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Dataset id")),
    request_body = TrainRequest,
    responses((status = 201, description = "Queued", body = TrainingView), (status = 409, description = "Not ready", body = ErrorBody)))]
pub async fn start_training(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<TrainRequest>,
) -> ApiResult<(StatusCode, Json<TrainingView>)> {
    require_person(&auth)?;
    let dataset = training::get_dataset(&state.pool, &auth.account.id, &id).await?;
    let t = queue_training(
        &state,
        &auth.account.id,
        jobs::priority_for(auth.session.kind),
        &dataset,
        req.steps,
        req.sample_prompts,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(training_view(&state, t).await?)))
}

/// Check a dataset is ready and put its training in the queue.
pub async fn queue_training(
    state: &AppState,
    account_id: &str,
    priority: i64,
    dataset: &Dataset,
    steps: Option<u32>,
    prompts: Option<Vec<String>>,
) -> ApiResult<Training> {
    let steps = steps.unwrap_or(2000);
    let prompts = match prompts.filter(|p| !p.is_empty()) {
        Some(p) => training::clean_prompts(&dataset.trigger_word, &p)?,
        None => {
            let captions: Vec<String> = training::images(&state.pool, account_id, &dataset.id)
                .await?
                .into_iter()
                .map(|i| i.caption)
                .collect();
            training::suggest_prompts(&dataset.trigger_word, &captions)
        }
    };
    if !(500..=6000).contains(&steps) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", "Use 500 to 6000 steps."));
    }
    if dataset.image_count < 5 {
        return Err(ApiError::conflict("Add at least 5 pictures first (15 to 30 is best)."));
    }
    let toolkit = train_job::toolkit_dir(state).await;
    // A helper PC with ai-toolkit trains when this server can't.
    if state.helpers.for_training().is_none() {
        trainer::check_install(&toolkit).map_err(|m| ApiError::new(StatusCode::CONFLICT, "toolkit_missing", m))?;
    }
    if captions_of(state, &dataset.id).is_some_and(|c| c.running) {
        return Err(ApiError::conflict("Captions are still being written. Train once they're done."));
    }
    for t in training::trainings(&state.pool, account_id, &dataset.id).await? {
        if !jobs::get_any(&state.pool, &t.job_id).await?.status.finished() {
            return Err(ApiError::conflict("This character is already training or waiting to."));
        }
    }
    let job = jobs::create(
        &state.pool,
        account_id,
        PACK_ID,
        &json!({ "dataset_id": dataset.id, "steps": steps }),
        priority,
    )
    .await?;
    let t = match training::add_training(&state.pool, account_id, dataset, &job.id, steps as i64, &prompts).await {
        Ok(t) => t,
        Err(e) => {
            let _ = jobs::finish(&state.pool, &job.id, JobStatus::Failed, Some("Couldn't record the training.")).await;
            return Err(e.into());
        }
    };
    state.queue.wake.notify_one();
    Ok(t)
}

/// A training with its progress, checkpoints and test pictures.
#[utoipa::path(get, path = "/api/trainings/{id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Training id")),
    responses((status = 200, description = "Training", body = TrainingView), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_training(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<TrainingView>> {
    let t = training::get_training(&state.pool, &auth.account.id, &id).await?;
    Ok(Json(training_view(&state, t).await?))
}

/// One of a training's test pictures.
#[utoipa::path(get, path = "/api/trainings/{id}/samples/{file}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Training id"), ("file" = String, Path, description = "Picture file name")),
    responses((status = 200, description = "The picture"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_sample(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, file)): Path<(String, String)>,
) -> ApiResult<Response> {
    let t = training::get_training(&state.pool, &auth.account.id, &id).await?;
    let run_dir = training::run_dir(&train_job::root(&state), &t.id);
    let path = trainer::sample_path(&run_dir, &file)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "No such picture."))?;
    let mime = if file.ends_with(".png") { "image/png" } else { "image/jpeg" };
    send_file(&path, mime).await
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UseRequest {
    /// The checkpoint to keep.
    pub step: u32,
    /// The character's name; leave out for the dataset's.
    pub name: Option<String>,
}

/// Keep one checkpoint as the character: it goes into the LoRA folder as
/// `<trigger>.safetensors` and shows in Create › Characters and chat by name.
#[utoipa::path(post, path = "/api/trainings/{id}/use", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Training id")),
    request_body = UseRequest,
    responses((status = 200, description = "The character", body = Character), (status = 409, description = "Name taken", body = ErrorBody)))]
pub async fn use_checkpoint(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<UseRequest>,
) -> ApiResult<Json<Character>> {
    require_person(&auth)?;
    let t = training::get_training(&state.pool, &auth.account.id, &id).await?;
    let run_dir = training::run_dir(&train_job::root(&state), &t.id);
    let cp = trainer::checkpoints(&run_dir, t.steps as u32)
        .into_iter()
        .find(|c| c.step == req.step && !c.file.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That checkpoint isn't saved."))?;
    let src = trainer::checkpoint_path(&run_dir, &cp.file)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That checkpoint's file is missing."))?;
    // Into my own LoRA folder, so only I see and use it.
    let name = format!("{}.safetensors", t.trigger_word);
    let lora_file = format!("{}/{name}", auth.account.id);
    let existing = training::character_by_lora(&state.pool, &lora_file).await?;
    let dir = state.assets.account_loras_dir(&auth.account.id);
    if existing.is_none() && dir.join(&name).exists() {
        return Err(ApiError::conflict(format!(
            "You already have a LoRA called {name}. Change the trigger word, or replace it with Add LoRA."
        )));
    }
    let copy = async {
        tokio::fs::create_dir_all(&dir).await?;
        let part = dir.join(format!(".{name}.part"));
        tokio::fs::copy(&src, &part).await?;
        tokio::fs::rename(&part, dir.join(&name)).await
    };
    copy.await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", format!("Couldn't copy the LoRA: {e}")))?;
    let dataset_name = training::get_dataset(&state.pool, &auth.account.id, &t.dataset_id)
        .await
        .map(|d| d.name)
        .unwrap_or_else(|_| t.trigger_word.clone());
    let name = req.name.filter(|n| !n.trim().is_empty()).unwrap_or(dataset_name);
    // What it is and how it's drawn, so chat can say "zk4t woman", not just "zk4t".
    let captions: Vec<String> = training::images(&state.pool, &auth.account.id, &t.dataset_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.caption)
        .collect();
    let (subject, style) = training::subject_and_style(&t.trigger_word, &captions);
    let c = training::save_character(
        &state.pool,
        &auth.account.id,
        NewCharacter {
            name: &name,
            trigger_word: &t.trigger_word,
            lora_file: &lora_file,
            training_id: Some(&t.id),
            step: Some(i64::from(req.step)),
            subject: subject.as_deref(),
            style: style.as_deref(),
        },
    )
    .await?;
    crate::webhooks::character_created(&state, &c);
    Ok(Json(c))
}

// ---------------------------------------------------------------- characters

#[derive(Debug, Deserialize, IntoParams)]
pub struct CharactersQuery {
    /// Everyone's (owner only).
    #[serde(default)]
    pub all: bool,
}

/// My characters (the owner can ask for everyone's).
#[utoipa::path(get, path = "/api/characters", tag = "training", security(("bearer" = [])),
    params(CharactersQuery),
    responses((status = 200, description = "Characters", body = [Character])))]
pub async fn list_characters(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<CharactersQuery>,
) -> ApiResult<Json<Vec<Character>>> {
    if q.all {
        auth.require_owner()?;
        return Ok(Json(training::characters(&state.pool, None).await?));
    }
    Ok(Json(training::characters(&state.pool, Some(auth.account.id.as_str())).await?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LoraFile {
    /// File name in the LoRA folder.
    pub file: String,
    #[serde(flatten)]
    pub info: outpost_engines::assets::LoraInfo,
    /// The character already using it (anyone's), if one is.
    pub character: Option<String>,
}

/// My LoRA files (and shared ones), with what each says about itself, for adding one
/// trained elsewhere as a character.
#[utoipa::path(get, path = "/api/characters/lora-files", tag = "training", security(("bearer" = [])),
    responses((status = 200, description = "LoRA files", body = [LoraFile])))]
pub async fn lora_files(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<LoraFile>>> {
    let everyone = training::characters(&state.pool, None).await?;
    let out = crate::routes::create::visible_loras(&state, &auth.account.id)
        .await
        .into_iter()
        .map(|file| LoraFile {
            info: state.assets.lora_info(&file).unwrap_or_default(),
            character: everyone.iter().find(|c| c.lora_file == file).map(|c| c.name.clone()),
            file,
        })
        .collect();
    Ok(Json(out))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NewCharacterRequest {
    /// A Z-Image LoRA already in the LoRA folder.
    pub lora_file: String,
    /// What to call them in Create and chat, e.g. "Kat".
    pub name: String,
    /// The word it was trained with.
    pub trigger_word: String,
    /// What it is: "woman", "man", "dog"... Goes beside the trigger word in chat.
    pub subject: Option<String>,
    /// How it's drawn: "photo", "cartoon style"...
    pub style: Option<String>,
}

/// Make a character from a LoRA trained elsewhere (Outpost leaves the file alone if
/// the character is removed).
#[utoipa::path(post, path = "/api/characters", tag = "training", security(("bearer" = [])),
    request_body = NewCharacterRequest,
    responses((status = 201, description = "Added", body = Character), (status = 400, description = "Not accepted", body = ErrorBody), (status = 409, description = "Already a character", body = ErrorBody)))]
pub async fn add_character(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<NewCharacterRequest>,
) -> ApiResult<(StatusCode, Json<Character>)> {
    require_person(&auth)?;
    let info = crate::routes::create::visible_loras(&state, &auth.account.id)
        .await
        .contains(&req.lora_file)
        .then(|| state.assets.lora_info(&req.lora_file))
        .flatten()
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid", format!("{} isn't in the LoRA folder.", req.lora_file)))?;
    if !info.z_image {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "not_z_image",
            "That LoRA isn't for Z-Image, so the Characters workflow can't use it.",
        ));
    }
    if let Some(c) = training::character_by_lora(&state.pool, &req.lora_file).await? {
        return Err(ApiError::conflict(format!("{} is already the character {}.", req.lora_file, c.name)));
    }
    let tidy = |v: Option<String>| v.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty() && s.chars().count() <= 40);
    let (subject, style) = (tidy(req.subject), tidy(req.style));
    let c = training::save_character(
        &state.pool,
        &auth.account.id,
        NewCharacter {
            name: &req.name,
            trigger_word: &req.trigger_word,
            lora_file: &req.lora_file,
            training_id: None,
            step: None,
            subject: subject.as_deref(),
            style: style.as_deref(),
        },
    )
    .await?;
    crate::webhooks::character_created(&state, &c);
    Ok((StatusCode::CREATED, Json(c)))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CharacterUpdate {
    pub name: String,
}

/// Rename a character.
#[utoipa::path(patch, path = "/api/characters/{id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Character id")),
    request_body = CharacterUpdate,
    responses((status = 200, description = "Renamed", body = Character), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn rename_character(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<CharacterUpdate>,
) -> ApiResult<Json<Character>> {
    require_person(&auth)?;
    Ok(Json(training::rename_character(&state.pool, &auth.account.id, &id, &req.name).await?))
}

/// Remove a character. A LoRA Outpost trained is deleted from the LoRA folder too.
#[utoipa::path(delete, path = "/api/characters/{id}", tag = "training", security(("bearer" = [])),
    params(("id" = String, Path, description = "Character id")),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_character(
    State(state): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_person(&auth)?;
    let owner = auth.require_owner().is_ok();
    let c = training::get_character(&state.pool, &auth.account.id, &id).await.ok();
    if c.is_none() && owner {
        // Someone else's: the owner may, with the PIN on a phone.
        auth.require_fresh_pin(&state, &headers).await?;
    }
    let c = training::delete_character(&state.pool, (!owner).then_some(auth.account.id.as_str()), &id).await?;
    if c.training_id.is_some() && !c.lora_file.contains(['/', '\\']) && !c.lora_file.contains("..") {
        let _ = tokio::fs::remove_file(state.assets.loras_dir().join(&c.lora_file)).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captions_are_one_clean_line() {
        assert_eq!(clean_caption("<think>hmm</think>\n\nwoman, beach, smiling.\nextra"), "woman, beach, smiling");
        assert_eq!(clean_caption(""), "");
    }

    #[test]
    fn captions_leave_out_what_the_character_always_looks_like() {
        let c = clean_caption(
            "woman, long wavy brown hair, oval face, fair skin, young adult, slender build, blue sundress, \
             sunglasses, hair clip, smiling, beach, school building, metallic jacket, realistic photo",
        );
        assert_eq!(
            c,
            "woman, blue sundress, sunglasses, hair clip, smiling, beach, school building, metallic jacket, realistic photo"
        );
    }
}
