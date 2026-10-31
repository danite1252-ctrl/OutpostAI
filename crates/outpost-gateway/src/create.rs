//! The Create worker: runs queued jobs in ComfyUI, one at a time, oldest first.
//!
//! For each job it makes sure ComfyUI is up (starting it if Outpost installed it),
//! frees the GPU from Ollama's chat models, builds the pack's graph, follows
//! progress, then copies the results into the account's gallery.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use outpost_core::attachments;
use outpost_core::jobs::{self, Job, JobStatus};
use outpost_core::media::{self, NewMedia};
use outpost_engines::comfy::{ComfyError, JobProgress};
use outpost_engines::packs::{self, InputKind};
use outpost_engines::{EngineId, EngineState};
use serde_json::Value;
use tokio::sync::Notify;

use crate::AppState;

/// The job running now and how far it has got.
#[derive(Debug, Clone)]
pub struct Running {
    pub job_id: String,
    /// `train` for a LoRA training.
    pub pack_id: String,
    pub prompt_id: Option<String>,
    pub progress: JobProgress,
    /// Cancel was asked for; checked before the job reaches ComfyUI.
    pub cancelled: bool,
    /// When the current sampling pass was first seen, and at which step, for the
    /// time left.
    pub sampling: Option<(std::time::Instant, u32, u32)>,
}

impl Running {
    pub(crate) fn track(&mut self, p: &JobProgress) {
        if p.steps == 0 || p.step == 0 {
            return;
        }
        match self.sampling {
            // A new pass (or the first): start timing from here.
            Some((_, first, steps)) if p.step >= first && p.steps == steps => {}
            _ => self.sampling = Some((std::time::Instant::now(), p.step, p.steps)),
        }
    }

    /// Seconds left in this sampling pass, from the pace so far.
    pub fn eta_secs(&self) -> Option<u64> {
        let (since, first, steps) = self.sampling?;
        let step = self.progress.step;
        if step <= first || self.progress.steps != steps {
            return None;
        }
        let per_step = since.elapsed().as_secs_f64() / f64::from(step - first);
        Some((per_step * f64::from(steps.saturating_sub(step))).ceil() as u64)
    }
}

/// A job running on a helper PC.
#[derive(Debug, Clone)]
pub struct RemoteRun {
    pub run: Running,
    /// The helper's name, e.g. "Dan's gaming PC".
    pub helper: String,
    /// The helper's sign-in (only it may report on the job).
    pub session_id: String,
    pub last_heard: std::time::Instant,
}

/// Shared between the worker and the API.
#[derive(Default)]
pub struct Queue {
    /// Wakes the worker when a job is added.
    pub wake: Notify,
    running: Mutex<Option<Running>>,
    /// Jobs running on helper PCs, by job id.
    remote: Mutex<HashMap<String, RemoteRun>>,
}

impl Queue {
    pub fn running(&self) -> Option<Running> {
        self.running.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// A job's progress, wherever it runs (here or on a helper PC).
    pub fn progress_of(&self, job_id: &str) -> Option<Running> {
        self.running()
            .filter(|r| r.job_id == job_id)
            .or_else(|| self.remote_run(job_id).map(|r| r.run))
    }

    pub fn remote_run(&self, job_id: &str) -> Option<RemoteRun> {
        self.remote.lock().unwrap_or_else(|p| p.into_inner()).get(job_id).cloned()
    }

    pub fn remote_runs(&self) -> Vec<RemoteRun> {
        self.remote.lock().unwrap_or_else(|p| p.into_inner()).values().cloned().collect()
    }

    pub(crate) fn remote_start(&self, run: RemoteRun) {
        self.remote.lock().unwrap_or_else(|p| p.into_inner()).insert(run.run.job_id.clone(), run);
    }

    /// Progress from the helper; returns whether the job was cancelled.
    pub(crate) fn remote_update(&self, job_id: &str, progress: JobProgress) -> Option<bool> {
        let mut map = self.remote.lock().unwrap_or_else(|p| p.into_inner());
        let r = map.get_mut(job_id)?;
        r.run.track(&progress);
        r.run.progress = progress;
        r.last_heard = std::time::Instant::now();
        Some(r.run.cancelled)
    }

    pub(crate) fn remote_end(&self, job_id: &str) -> Option<RemoteRun> {
        self.remote.lock().unwrap_or_else(|p| p.into_inner()).remove(job_id)
    }

    fn set_running(&self, r: Option<Running>) {
        *self.running.lock().unwrap_or_else(|p| p.into_inner()) = r;
    }

    /// Ask the running job to stop. Returns its ComfyUI prompt id if it has one
    /// (the caller interrupts ComfyUI); otherwise it stops before reaching ComfyUI. A job
    /// on a helper PC is told to stop the next time the helper reports.
    pub fn cancel(&self, job_id: &str) -> Option<String> {
        if let Some(r) = self.remote.lock().unwrap_or_else(|p| p.into_inner()).get_mut(job_id) {
            r.run.cancelled = true;
            return None;
        }
        let mut guard = self.running.lock().unwrap_or_else(|p| p.into_inner());
        let r = guard.as_mut().filter(|r| r.job_id == job_id)?;
        r.cancelled = true;
        r.prompt_id.clone()
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.running().is_some_and(|r| r.cancelled)
    }

    /// A LoRA training has the graphics card (chat waits, so it can't run out of memory).
    pub fn training(&self) -> bool {
        self.running().is_some_and(|r| r.pack_id == crate::train::PACK_ID)
    }

    pub(crate) fn update(&self, f: impl FnOnce(&mut Running)) {
        if let Some(r) = self.running.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            f(r);
        }
    }
}

/// Run forever, taking jobs as they come.
pub async fn worker(state: AppState) {
    match jobs::recover(&state.pool).await {
        Ok(n) if n > 0 => tracing::info!(count = n, "marked jobs interrupted by a restart as failed"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "could not tidy the job queue"),
    }
    // Copies left in ComfyUI by jobs before a restart.
    clear_comfy_copies(&state).await;
    loop {
        // Only what runs here; jobs only a helper PC can run wait for it.
        let here = crate::helpers::local_packs(&state).await;
        let job = match jobs::take_next_of(&state.pool, &here).await {
            Ok(Some(job)) => job,
            Ok(None) => {
                // Sleep until a job arrives (or check again in a while anyway).
                let _ = tokio::time::timeout(Duration::from_secs(30), state.queue.wake.notified()).await;
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not read the job queue");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        state.queue.set_running(Some(Running {
            job_id: job.id.clone(),
            pack_id: job.pack_id.clone(),
            prompt_id: None,
            progress: JobProgress {
                phase: "Starting".into(),
                ..Default::default()
            },
            cancelled: false,
            sampling: None,
        }));
        // Run each job in its own task so a bug in one can't stop the queue.
        let task = if job.pack_id == crate::train::PACK_ID {
            tokio::spawn(crate::train::run(state.clone(), job.clone()))
        } else {
            tokio::spawn(run(state.clone(), job.clone()))
        };
        let outcome = match task.await {
            Ok(result) => result,
            Err(e) => {
                tracing::error!(job = %job.id, error = %e, "create job crashed");
                Err(ComfyError::Failed(
                    "Outpost hit an internal error running this job.".into(),
                ))
            }
        };
        let (status, error) = match &outcome {
            Ok(()) => (JobStatus::Done, None),
            Err(ComfyError::Interrupted) => (JobStatus::Cancelled, None),
            Err(e) => (JobStatus::Failed, Some(e.to_string())),
        };
        if let Some(e) = &error {
            tracing::warn!(job = %job.id, pack = %job.pack_id, error = %e, "create job failed");
        }
        if let Err(e) = jobs::finish(&state.pool, &job.id, status, error.as_deref()).await {
            tracing::warn!(error = %e, "could not record a finished job");
        }
        crate::webhooks::job_finished(&state, &job.id).await;
        state.queue.set_running(None);
        remove_inputs(&state, &job).await;
        clear_comfy_copies(&state).await;
        // Queue empty: give the GPU back to chat.
        if matches!(jobs::queued_count(&state.pool).await, Ok(0)) {
            state.comfy.free().await;
        }
    }
}

/// Delete the pictures a job was given once it has finished with them.
pub(crate) async fn remove_inputs(state: &AppState, job: &Job) {
    let values: serde_json::Map<String, Value> = serde_json::from_str(&job.inputs).unwrap_or_default();
    let files: Vec<_> = values
        .values()
        .filter_map(|v| v.get("$file").and_then(Value::as_str))
        .filter(|rel| rel.starts_with(&format!("{}/inputs/", job.account_id)) && !rel.contains(".."))
        .map(|rel| state.config.files_dir().join(rel))
        .collect();
    attachments::remove_files(&files).await;
}

/// ComfyUI keeps its own copy of every result (`output/outpost`) and of every picture it
/// was given (`input/outpost`). Outpost has already copied results into the gallery (where
/// deleting really deletes) and keeps job pictures itself, so these copies are only clutter
/// that would fill the disk. One job runs at a time, so between jobs nothing is using them.
/// Only Outpost's own subfolders of the ComfyUI it installed are touched.
pub async fn clear_comfy_copies(state: &AppState) {
    let app = state.engines.comfyui_app_dir();
    for sub in ["output", "input"] {
        let dir = app.join(sub).join("outpost");
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => tracing::debug!(dir = %dir.display(), "cleared ComfyUI's copies"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(dir = %dir.display(), error = %e, "couldn't clear ComfyUI's copies"),
        }
    }
}

/// How long a job may run before Outpost gives up on it.
fn time_limit(pack: &packs::Pack) -> Duration {
    match pack.output.kind {
        packs::OutputKind::Video => Duration::from_secs(90 * 60),
        packs::OutputKind::Image => Duration::from_secs(30 * 60),
        // Shaping, texturing and baking a model takes a while on a smaller card.
        packs::OutputKind::Model => Duration::from_secs(45 * 60),
    }
}

async fn run(state: AppState, job: Job) -> Result<(), ComfyError> {
    let state = &state;
    let job = &job;
    let pack = packs::find(&job.pack_id).ok_or_else(|| ComfyError::Failed(format!("Unknown pack {}.", job.pack_id)))?;
    if !state.assets.status(&pack).ready {
        return Err(ComfyError::Failed(format!(
            "{}'s models aren't downloaded yet. Get them in Create first.",
            pack.name
        )));
    }
    ensure_comfyui(state).await?;
    state.queue.update(|r| r.progress.phase = "Freeing the GPU".into());
    free_ollama(state).await;

    let values: serde_json::Map<String, Value> = serde_json::from_str(&job.inputs).unwrap_or_default();
    // Hand any input pictures to ComfyUI first.
    let mut uploaded = HashMap::new();
    for input in pack.inputs.iter().filter(|i| i.kind == InputKind::Image) {
        let Some(rel) = values
            .get(&input.key)
            .and_then(|v| v.get("$file"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let data = attachments::read_job_input(&state.config.files_dir(), &job.account_id, rel)
            .await
            .map_err(|_| ComfyError::Failed(format!("The picture for {} is missing.", input.label)))?;
        let name = rel.rsplit('/').next().unwrap_or("input.png");
        let comfy_name = state.comfy.upload_image(data, name).await?;
        uploaded.insert(input.key.clone(), comfy_name);
    }
    let built = pack
        .build(
            &values,
            jobs::random_seed,
            &uploaded,
            &crate::routes::create::visible_loras(state, &job.account_id).await,
        )
        .map_err(|e| ComfyError::Rejected(e.to_string()))?;
    if let Some(seed) = built.seed {
        let _ = jobs::set_seed(&state.pool, &job.id, seed).await;
    }
    if state.queue.cancelled() {
        return Err(ComfyError::Interrupted);
    }
    let prompt_id = state.comfy.submit(&built.graph).await?;
    state.queue.update(|r| r.prompt_id = Some(prompt_id.clone()));
    // Cancelled while it was being submitted.
    if state.queue.cancelled() {
        state.comfy.dequeue(&prompt_id).await;
        state.comfy.interrupt(&prompt_id).await;
    }

    let queue = state.queue.clone();
    let files = state
        .comfy
        .wait(
            &prompt_id,
            time_limit(&pack),
            {
                let queue = state.queue.clone();
                move || queue.cancelled()
            },
            move |p| {
                queue.update(|r| {
                    r.track(&p);
                    r.progress = p;
                })
            },
        )
        .await?;

    state.queue.update(|r| r.progress.phase = "Saving".into());
    let prompt = values.get("prompt").and_then(Value::as_str);
    let mut saved = 0;
    for (node, file) in files.iter().filter(|(node, _)| *node == pack.output.node) {
        let data = state.comfy.fetch(file).await?;
        let saved_media = media::add(
            &state.pool,
            &state.config.files_dir(),
            NewMedia {
                account_id: &job.account_id,
                job_id: Some(&job.id),
                pack_id: Some(&pack.id),
                prompt,
                seed: built.seed,
                source_name: &file.filename,
                data: &data,
                created_at: None,
            },
        )
        .await
        .map_err(|e| ComfyError::Failed(e.to_string()))?;
        if pack.print_height(&values).is_some() {
            state.queue.update(|r| r.progress.phase = "Making it ready for 3D printing".into());
            crate::print3d::after_saved(state, &pack, &values, &saved_media.id).await;
        }
        tracing::debug!(job = %job.id, node, "saved a result");
        saved += 1;
    }
    if saved == 0 {
        return Err(ComfyError::Failed("ComfyUI finished but returned nothing.".into()));
    }
    Ok(())
}

/// ComfyUI must be answering. Start it if Outpost installed it and it's stopped.
pub(crate) async fn ensure_comfyui(state: &AppState) -> Result<(), ComfyError> {
    if state.comfy.is_up().await {
        return Ok(());
    }
    let status = state.engines.status().await;
    let info = status.engines.iter().find(|e| e.id == EngineId::Comfyui);
    match info.map(|e| e.state) {
        Some(EngineState::Running) | Some(EngineState::External) | Some(EngineState::Starting) => {}
        Some(EngineState::Stopped) | Some(EngineState::Error) => {
            state.queue.update(|r| r.progress.phase = "Starting ComfyUI".into());
            state
                .engines
                .start(EngineId::Comfyui)
                .map_err(|e| ComfyError::Failed(format!("Couldn't start ComfyUI: {e}")))?;
        }
        _ => {
            return Err(ComfyError::Failed(
                "ComfyUI isn't installed. Install it in Settings > Engines.".into(),
            ))
        }
    }
    // First start after an install can take a while (it compiles and loads nodes).
    for _ in 0..120 {
        if state.comfy.is_up().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(ComfyError::Offline)
}

/// The models Ollama has in memory right now (none if it isn't running).
pub(crate) async fn ollama_loaded(state: &AppState) -> Vec<String> {
    let base = state.config.ollama_url.trim_end_matches('/').to_string();
    let Ok(resp) = state.http.get(format!("{base}/api/ps")).send().await else {
        return Vec::new();
    };
    let Ok(body) = resp.json::<Value>().await else { return Vec::new() };
    body.get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Unload Ollama's models so image and video jobs get the whole GPU. Chat loads its
/// model again on the next message.
pub(crate) async fn free_ollama(state: &AppState) {
    let base = state.config.ollama_url.trim_end_matches('/').to_string();
    for model in ollama_loaded(state).await {
        let _ = state
            .http
            .post(format!("{base}/api/generate"))
            .json(&serde_json::json!({ "model": model, "keep_alive": 0 }))
            .send()
            .await;
        tracing::info!(%model, "unloaded chat model to free the GPU");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(step: u32, steps: u32) -> JobProgress {
        JobProgress {
            phase: "Generating".into(),
            step,
            steps,
            percent: None,
        }
    }

    #[test]
    fn time_left_follows_the_pass() {
        let mut r = Running {
            job_id: "j".into(),
            pack_id: "z_image_turbo".into(),
            prompt_id: None,
            progress: JobProgress::default(),
            cancelled: false,
            sampling: None,
        };
        let p = at(1, 8);
        r.track(&p);
        r.progress = p;
        // One step seen: no pace yet.
        assert_eq!(r.eta_secs(), None);
        let first = r.sampling.unwrap().0;
        let p = at(4, 8);
        r.track(&p);
        r.progress = p;
        assert_eq!(r.sampling.unwrap().0, first, "same pass keeps its start");
        assert!(r.eta_secs().is_some());
        // A second pass (e.g. a video's second sampler) starts timing again.
        let p = at(1, 20);
        r.track(&p);
        r.progress = p;
        assert_eq!(r.sampling.unwrap().2, 20);
        assert_eq!(r.eta_secs(), None);
    }
}

/// Make a queue for an AppState.
pub fn new_queue() -> Arc<Queue> {
    Arc::new(Queue::default())
}
