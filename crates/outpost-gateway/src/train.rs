//! Training jobs: a LoRA training waits in the same GPU queue as pictures and videos
//! (pack id `train`), and the worker hands it here. This writes the captions and the
//! ai-toolkit config, frees the graphics card, runs ai-toolkit and follows it.

use std::path::PathBuf;
use std::time::Duration;

use outpost_core::jobs::Job;
use outpost_core::training;
use outpost_engines::comfy::{ComfyError, JobProgress};
use outpost_engines::trainer::{self, Outcome};
use outpost_engines::{EngineId, EngineState, Log};

use crate::AppState;

/// The queue's name for a training job.
pub const PACK_ID: &str = "train";

/// Peak video memory a Z-Image Turbo training needs with Outpost's settings (8-bit
/// model, low-VRAM mode), in GB.
pub const VRAM_GB: f32 = 16.0;

/// `<data>/training`.
pub fn root(state: &AppState) -> PathBuf {
    state.config.data_dir.join("training")
}

/// Where ai-toolkit is: the saved setting, else the usual place.
pub async fn toolkit_dir(state: &AppState) -> PathBuf {
    outpost_core::settings::get(&state.pool, outpost_core::settings::AI_TOOLKIT_DIR)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(trainer::default_dir)
}

/// Stop ComfyUI if Outpost runs it, including one still starting up (Outpost starts
/// it by itself when it opens, which can land right as a training begins).
async fn stop_comfyui(state: &AppState) {
    let status = state.engines.status().await;
    if status
        .engines
        .iter()
        .any(|e| e.id == EngineId::Comfyui && matches!(e.state, EngineState::Running | EngineState::Starting))
    {
        let _ = state.engines.stop(EngineId::Comfyui);
        tracing::info!("stopped ComfyUI for a training; it starts again for the next picture");
    }
}

/// Unloading is asynchronous: Ollama and ComfyUI let go of their memory a few
/// seconds after being asked. Starting the trainer before then makes Windows put part
/// of the model in shared memory for the whole run, several times slower. So wait
/// until they're really gone (and, where nvidia-smi can tell, the card is nearly
/// empty), for up to a minute.
async fn wait_for_free_gpu(state: &AppState) {
    state.queue.update(|r| r.progress.phase = "Waiting for the GPU to empty".into());
    let started = std::time::Instant::now();
    loop {
        let ollama = crate::create::ollama_loaded(state).await;
        let comfy = state.comfy.is_up().await;
        let used = outpost_engines::gpu::used_mb().await;
        let card_ok = used.map_or(true, |mb| mb < 2500);
        if comfy {
            // Came up while we waited: stop it again.
            stop_comfyui(state).await;
        }
        if ollama.is_empty() && !comfy && card_ok {
            tracing::info!(used_mb = ?used, "GPU is free for training");
            break;
        }
        if started.elapsed() > Duration::from_secs(60) || state.queue.cancelled() {
            tracing::warn!(ollama = ?ollama, comfyui_up = comfy, used_mb = ?used, "GPU still busy after a minute; training anyway");
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    // A moment more for the driver to hand the memory back.
    tokio::time::sleep(Duration::from_secs(3)).await;
}

/// Run one training job to the end.
pub async fn run(state: AppState, job: Job) -> Result<(), ComfyError> {
    let state = &state;
    let failed = |m: String| ComfyError::Failed(m);
    let t = training::training_for_job(&state.pool, &job.id)
        .await
        .map_err(|_| failed("This training's record is missing.".into()))?;
    let toolkit = toolkit_dir(state).await;
    trainer::check_install(&toolkit).map_err(failed)?;
    let root = root(state);
    let dataset = training::get_dataset(&state.pool, &t.account_id, &t.dataset_id)
        .await
        .map_err(|_| failed("The dataset was deleted.".into()))?;
    state.queue.update(|r| r.progress.phase = "Writing captions".into());
    let n = training::write_captions(&state.pool, &root, &t.account_id, &dataset.id)
        .await
        .map_err(|e| failed(e.to_string()))?;
    if n == 0 {
        return Err(failed("The dataset has no pictures.".into()));
    }

    // The whole card for training: ComfyUI and Ollama let go of theirs.
    state.queue.update(|r| r.progress.phase = "Freeing the GPU".into());
    state.comfy.free().await;
    stop_comfyui(state).await;
    crate::create::free_ollama(state).await;
    wait_for_free_gpu(state).await;
    if state.queue.cancelled() {
        return Err(ComfyError::Interrupted);
    }

    let run_dir = training::run_dir(&root, &t.id);
    tokio::fs::create_dir_all(&run_dir)
        .await
        .map_err(|e| failed(format!("Couldn't make the training folder: {e}")))?;
    let spec = trainer::Spec {
        trigger: t.trigger_word.clone(),
        dataset_dir: training::dataset_dir(&root, &dataset.id),
        run_dir: run_dir.clone(),
        steps: t.steps as u32,
        rank: 32,
        learning_rate: 1e-4,
        sample_prompts: match t.prompts() {
            p if !p.is_empty() => p,
            _ => trainer::sample_prompts(&t.trigger_word),
        },
    };
    let config_file = run_dir.join("config.json");
    let text = serde_json::to_string_pretty(&trainer::config(&spec)).unwrap_or_default();
    tokio::fs::write(&config_file, text)
        .await
        .map_err(|e| failed(format!("Couldn't write the training settings: {e}")))?;
    tracing::info!(training = %t.id, trigger = %t.trigger_word, steps = t.steps, images = n, "starting a LoRA training");

    let log = Log::new(run_dir.join("train.log"));
    let queue = state.queue.clone();
    let mut last_logged = 0u32;
    let training_id = t.id.clone();
    let outcome = trainer::run(
        &toolkit,
        &config_file,
        &log,
        {
            let queue = state.queue.clone();
            move || queue.cancelled()
        },
        move |p| {
            if p.step >= last_logged + 100 || (p.step == p.steps && p.step != last_logged && p.steps > 0) {
                last_logged = p.step;
                tracing::info!(training = %training_id, step = p.step, steps = p.steps, "training");
            }
            let jp = JobProgress {
                phase: p.phase.clone(),
                step: p.step,
                steps: p.steps,
                percent: None,
            };
            queue.update(|r| {
                r.track(&jp);
                r.progress = jp;
            });
        },
    )
    .await;
    // Let the log's last lines reach the disk.
    tokio::time::sleep(Duration::from_millis(300)).await;
    match outcome {
        Outcome::Done => {
            let saved = trainer::checkpoints(&run_dir, t.steps as u32)
                .iter()
                .filter(|c| !c.file.is_empty())
                .count();
            tracing::info!(training = %t.id, checkpoints = saved, "training finished");
            if saved == 0 {
                return Err(failed("ai-toolkit finished but saved no LoRA. See train.log in the run folder.".into()));
            }
            Ok(())
        }
        Outcome::Cancelled => Err(ComfyError::Interrupted),
        Outcome::Failed(m) => Err(failed(m)),
    }
}

/// For an unattended test run (`outpost-gateway --queue-training <folder>`): make a
/// dataset for the owner from a folder on this PC, caption pictures that have no
/// `.txt`, and queue its training, just as Train in the desktop app does.
pub async fn queue_from_folder(state: AppState, folder: PathBuf, name: String, trigger: Option<String>, steps: u32) {
    match queue_from_folder_inner(&state, &folder, &name, trigger, steps).await {
        Ok(id) => tracing::info!(training = %id, "test training queued; follow it in Train or the queue"),
        Err(e) => tracing::error!(error = %e, "couldn't queue the test training"),
    }
}

async fn queue_from_folder_inner(
    state: &AppState,
    folder: &std::path::Path,
    name: &str,
    trigger: Option<String>,
    steps: u32,
) -> anyhow::Result<String> {
    use anyhow::anyhow;
    use outpost_core::accounts::{self, Role};
    let owner = accounts::list(&state.pool)
        .await?
        .into_iter()
        .find(|a| a.role == Role::Owner)
        .ok_or_else(|| anyhow!("no owner account yet: run first-time setup in the desktop app"))?;
    let word = trigger.unwrap_or_else(|| outpost_core::trigger::from_name(name));
    let check = outpost_core::trigger::check(&word);
    if !check.ok {
        tracing::warn!(word = %word, problem = ?check.problem, suggestion = ?check.suggestion, "trigger word isn't ideal; using it anyway");
    }
    // Run again: reuse the dataset (and its captions) from last time.
    let existing = training::list_datasets(&state.pool, &owner.id)
        .await?
        .into_iter()
        .find(|d| d.name == name && d.trigger_word == word && d.image_count > 0);
    let dataset = match existing {
        Some(d) => {
            tracing::info!(dataset = %d.id, pictures = d.image_count, "reusing the dataset from the last run");
            d
        }
        None => {
            let d = training::create_dataset(&state.pool, &owner.id, name, &word).await?;
            let (added, skipped) = training::import_folder(&state.pool, &root(state), &owner.id, &d.id, folder).await?;
            tracing::info!(dataset = %d.id, added, skipped = ?skipped, "pictures imported");
            d
        }
    };

    // Captions need Ollama and a model that can see; give Ollama time to start.
    let mut models = Vec::new();
    for _ in 0..60 {
        models = crate::routes::training::vision_models(state).await;
        if !models.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let todo: Vec<_> = training::images(&state.pool, &owner.id, &dataset.id)
        .await?
        .into_iter()
        .filter(|i| i.caption.is_empty())
        .collect();
    match models.first() {
        Some(model) if !todo.is_empty() => {
            tracing::info!(%model, pictures = todo.len(), "writing captions");
            crate::routes::training::caption_all(state.clone(), owner.id.clone(), dataset.clone(), model.clone(), todo).await;
            let d = training::get_dataset(&state.pool, &owner.id, &dataset.id).await?;
            tracing::info!(captioned = d.captioned_count, of = d.image_count, "captions written");
        }
        None if !todo.is_empty() => {
            tracing::warn!("no vision model in Ollama: training with the trigger word as the only caption")
        }
        _ => {}
    }
    let dataset = training::get_dataset(&state.pool, &owner.id, &dataset.id).await?;
    let t = crate::routes::training::queue_training(state, &owner.id, 0, &dataset, Some(steps), None)
        .await
        .map_err(|e| anyhow!(e.message))?;
    Ok(t.id)
}
