use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::OnceCell;

use crate::gpu::{self, Gpu};
use crate::process::{self, Log};
use crate::types::{EngineId, EngineInfo, EngineState, EnginesStatus, Progress};
use crate::{ai_toolkit, comfyui, ollama, versions, voice};

#[derive(Debug)]
pub enum EngineError {
    /// An install or start is already in progress.
    Busy,
    /// Another copy is running outside Outpost on the engine's port.
    External,
    NotInstalled,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EngineError::Busy => "That engine is busy installing or starting.",
            EngineError::External => {
                "It's already running outside Outpost. Outpost is using that copy; close it first to let Outpost manage it."
            }
            EngineError::NotInstalled => "Install it first.",
        })
    }
}

/// What gets written to `<engine>/installed.json` after a successful install.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Installed {
    pub version: String,
    #[serde(default)]
    pub torch_variant: Option<String>,
}

pub(crate) struct SlotState {
    pub state: EngineState,
    pub installed: Option<Installed>,
    pub progress: Option<Progress>,
    pub error: Option<String>,
    pub child: Option<Child>,
}

pub(crate) struct Slot {
    pub dir: PathBuf,
    pub log: Log,
    pub inner: Mutex<SlotState>,
}

impl Slot {
    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn marker(&self) -> PathBuf {
        self.dir.join("installed.json")
    }
}

/// Everything an installer needs, owned so it can run on a background task.
pub(crate) struct Ctx {
    pub slot: Arc<Slot>,
    pub engines_root: PathBuf,
    pub client: reqwest::Client,
    pub gpus: Vec<Gpu>,
}

impl Ctx {
    pub fn step(&self, index: u32, count: u32, step: &str) {
        self.slot.log.push(format!("== {step}"));
        self.slot.lock().progress = Some(Progress {
            step: step.to_string(),
            step_index: index,
            step_count: count,
            percent: None,
            detail: None,
        });
    }

    pub fn progress(&self, percent: Option<f32>, detail: Option<String>) {
        if let Some(p) = self.slot.lock().progress.as_mut() {
            p.percent = percent;
            p.detail = detail;
        }
    }

    pub fn torch_variant(&self) -> &'static str {
        torch_variant(&self.gpus)
    }

    /// Callback for `download::download` that feeds the progress bar.
    pub fn download_progress(&self) -> impl FnMut(u64, Option<u64>) + '_ {
        move |done, total| {
            let percent = total
                .filter(|t| *t > 0)
                .map(|t| (done as f32 / t as f32 * 100.0).min(100.0));
            self.progress(percent, Some(crate::download::describe(done, total)));
        }
    }
}

fn torch_variant(gpus: &[Gpu]) -> &'static str {
    if gpus.iter().any(|g| g.vendor == "nvidia") {
        "cu130"
    } else {
        "cpu"
    }
}

pub struct EngineManager {
    engines_root: PathBuf,
    models_root: PathBuf,
    download_client: reqwest::Client,
    probe_client: reqwest::Client,
    slots: HashMap<EngineId, Arc<Slot>>,
    /// ai-toolkit, for LoRA training: installed like an engine, but never left running.
    trainer: Arc<Slot>,
    /// A listening model being downloaded (its id), or why the last download failed.
    listening: Arc<Mutex<ListeningFetch>>,
    gpus: OnceCell<Vec<Gpu>>,
}

/// A listening (speech-to-text) model download.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ListeningFetch {
    /// The model downloading now.
    pub model: Option<String>,
    /// How far, 0-1, roughly.
    pub progress: Option<f32>,
    /// The model the last failed download was for, and why.
    pub failed: Option<String>,
    pub error: Option<String>,
}

/// Outpost's own ai-toolkit install, as the apps see it.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TrainerInstall {
    /// Installed by Outpost (in `dir`).
    pub installed: bool,
    pub installing: bool,
    pub version: Option<String>,
    pub progress: Option<Progress>,
    /// Why the last install failed.
    pub error: Option<String>,
    /// Where Outpost installs it.
    pub dir: String,
    /// The pinned version an install gets.
    pub pinned_version: String,
    pub log_tail: Vec<String>,
}

impl EngineManager {
    /// Engines live in `<data_dir>/engines`, models in `<data_dir>/models`.
    /// Nothing starts until [`EngineManager::init`].
    pub fn new(data_dir: &Path) -> Arc<Self> {
        let engines_root = data_dir.join("engines");
        let slots = EngineId::ALL
            .into_iter()
            .map(|id| {
                let dir = engines_root.join(id.as_str());
                let installed = read_marker(&dir.join("installed.json"));
                let state = if installed.is_some() {
                    EngineState::Stopped
                } else {
                    EngineState::NotInstalled
                };
                let slot = Slot {
                    log: Log::new(dir.join("logs").join(format!("{}.log", id.as_str()))),
                    dir,
                    inner: Mutex::new(SlotState {
                        state,
                        installed,
                        progress: None,
                        error: None,
                        child: None,
                    }),
                };
                (id, Arc::new(slot))
            })
            .collect();
        let trainer_dir = engines_root.join("ai-toolkit");
        let trainer_installed = read_marker(&trainer_dir.join("installed.json"));
        let trainer = Arc::new(Slot {
            log: Log::new(trainer_dir.join("logs").join("ai-toolkit-install.log")),
            inner: Mutex::new(SlotState {
                state: if trainer_installed.is_some() { EngineState::Stopped } else { EngineState::NotInstalled },
                installed: trainer_installed,
                progress: None,
                error: None,
                child: None,
            }),
            dir: trainer_dir,
        });
        Arc::new(EngineManager {
            trainer,
            listening: Default::default(),
            models_root: data_dir.join("models"),
            engines_root,
            download_client: reqwest::Client::builder()
                .user_agent(concat!("Outpost/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
            probe_client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_millis(1500))
                .build()
                .expect("HTTP client"),
            slots,
            gpus: OnceCell::new(),
        })
    }

    /// The ComfyUI that Outpost installed (its `app` folder, holding `input` and `output`).
    pub fn comfyui_app_dir(&self) -> PathBuf {
        self.engines_root.join(EngineId::Comfyui.as_str()).join("app")
    }

    /// Detect GPUs and start every engine Outpost installed, unless a copy is
    /// already running outside Outpost.
    pub async fn init(self: &Arc<Self>) {
        let gpus = self.gpus().await;
        tracing::info!(?gpus, "graphics cards");
        for id in EngineId::ALL {
            let up = self.probe(id).await;
            let slot = &self.slots[&id];
            let installed = slot.lock().installed.is_some();
            if up {
                slot.lock().state = EngineState::External;
                tracing::info!(engine = id.as_str(), "already running outside Outpost; using it");
            } else if installed {
                if let Err(e) = self.start(id) {
                    tracing::warn!(engine = id.as_str(), error = %e, "could not start");
                }
            }
        }
    }

    pub async fn gpus(&self) -> Vec<Gpu> {
        self.gpus.get_or_init(gpu::detect).await.clone()
    }

    async fn probe(&self, id: EngineId) -> bool {
        matches!(self.probe_client.get(id.health_url()).send().await, Ok(r) if r.status().is_success())
    }

    pub async fn status(&self) -> EnginesStatus {
        let mut engines = Vec::new();
        for id in EngineId::ALL {
            let slot = &self.slots[&id];
            // Engines Outpost isn't running itself may have been started or closed outside it.
            let state = slot.lock().state;
            if matches!(
                state,
                EngineState::NotInstalled | EngineState::External | EngineState::Stopped
            ) {
                let up = self.probe(id).await;
                let mut st = slot.lock();
                if matches!(
                    st.state,
                    EngineState::NotInstalled | EngineState::External | EngineState::Stopped
                ) {
                    st.state = match (up, st.installed.is_some()) {
                        (true, _) => EngineState::External,
                        (false, true) => EngineState::Stopped,
                        (false, false) => EngineState::NotInstalled,
                    };
                }
            }
            let st = slot.lock();
            engines.push(EngineInfo {
                id,
                name: id.display_name().to_string(),
                state: st.state,
                managed: st.installed.is_some() && st.state != EngineState::External,
                version: st.installed.as_ref().map(|i| i.version.clone()),
                pinned_version: match id {
                    EngineId::Ollama => versions::OLLAMA_VERSION,
                    EngineId::Comfyui => versions::COMFYUI_VERSION,
                    EngineId::Voice => versions::VOICE_VERSION,
                }
                .to_string(),
                url: id.base_url(),
                size_hint: match id {
                    EngineId::Ollama => "about 2 GB download",
                    EngineId::Comfyui => "about 4 GB download (Python + PyTorch), 7 GB on disk",
                    EngineId::Voice => "about 1 GB download (speech recognition and 54 voices), runs on the CPU",
                }
                .to_string(),
                progress: st.progress.clone(),
                error: st.error.clone(),
                log_tail: slot.log.tail(12),
            });
        }
        let gpus = self.gpus().await;
        EnginesStatus {
            torch_variant: torch_variant(&gpus).to_string(),
            gpus,
            engines,
        }
    }

    /// Download and set up an engine in the background, then start it.
    pub fn install(self: &Arc<Self>, id: EngineId) -> Result<(), EngineError> {
        let slot = self.slots[&id].clone();
        {
            let mut st = slot.lock();
            match st.state {
                EngineState::Installing | EngineState::Starting => return Err(EngineError::Busy),
                EngineState::External => return Err(EngineError::External),
                _ => {}
            }
            if let Some(mut child) = st.child.take() {
                process::kill_tree(&mut child);
            }
            st.state = EngineState::Installing;
            st.error = None;
            st.progress = None;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let ctx = Ctx {
                slot: slot.clone(),
                engines_root: me.engines_root.clone(),
                client: me.download_client.clone(),
                gpus: me.gpus().await,
            };
            slot.log.push(format!("Installing {}", id.display_name()));
            let result = match id {
                EngineId::Ollama => ollama::install(&ctx).await,
                EngineId::Comfyui => comfyui::install(&ctx).await,
                EngineId::Voice => voice::install(&ctx, &me.models_root).await,
            };
            match result {
                Ok(installed) => {
                    let saved = serde_json::to_vec_pretty(&installed)
                        .map_err(anyhow::Error::from)
                        .and_then(|bytes| std::fs::write(slot.marker(), bytes).map_err(anyhow::Error::from));
                    if let Err(e) = saved {
                        fail(&slot, format!("Installed, but could not save the install record: {e}"));
                        return;
                    }
                    slot.log
                        .push(format!("{} {} installed", id.display_name(), installed.version));
                    {
                        let mut st = slot.lock();
                        st.installed = Some(installed);
                        st.state = EngineState::Stopped;
                        st.progress = None;
                    }
                    if let Err(e) = me.start(id) {
                        fail(&slot, e.to_string());
                    }
                }
                Err(e) => fail(&slot, format!("{e:#}")),
            }
        });
        Ok(())
    }

    /// Start an installed engine and watch it.
    pub fn start(self: &Arc<Self>, id: EngineId) -> Result<(), EngineError> {
        let slot = self.slots[&id].clone();
        let cmd: Command = {
            let mut st = slot.lock();
            match st.state {
                EngineState::Running | EngineState::Starting => return Ok(()),
                EngineState::Installing => return Err(EngineError::Busy),
                EngineState::External => return Err(EngineError::External),
                _ => {}
            }
            if st.installed.is_none() {
                return Err(EngineError::NotInstalled);
            }
            let cmd = match id {
                EngineId::Ollama => {
                    let vram = self.gpus.get().and_then(|g| g.iter().map(|g| g.vram_mb).max()).unwrap_or(0);
                    ollama::command(&slot.dir, &self.models_root, versions::context_length_for(vram))
                }
                EngineId::Comfyui => comfyui::command(&slot.dir, &self.models_root),
                EngineId::Voice => voice::command(&slot.dir, &self.models_root),
            };
            match cmd {
                Ok(c) => {
                    st.error = None;
                    c
                }
                Err(e) => {
                    st.state = EngineState::Error;
                    st.error = Some(format!("{e:#}"));
                    return Ok(());
                }
            }
        };
        slot.log.push(format!("Starting {}", id.display_name()));
        let child = match process::spawn(cmd, &slot.log) {
            Ok(c) => c,
            Err(e) => {
                fail(&slot, format!("Could not start {}: {e}", id.display_name()));
                return Ok(());
            }
        };
        {
            let mut st = slot.lock();
            st.child = Some(child);
            st.state = EngineState::Starting;
        }
        let me = self.clone();
        tokio::spawn(async move { me.watch(id).await });
        Ok(())
    }

    /// Follow a started engine: Starting -> Running, or Error if it exits.
    async fn watch(self: Arc<Self>, id: EngineId) {
        let slot = self.slots[&id].clone();
        // ComfyUI's first start compiles and caches a lot; give it time.
        let start_limit = match id {
            EngineId::Ollama => Duration::from_secs(60),
            EngineId::Comfyui => Duration::from_secs(600),
            EngineId::Voice => Duration::from_secs(120),
        };
        let started = tokio::time::Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let exited = {
                let mut st = slot.lock();
                if !matches!(st.state, EngineState::Starting | EngineState::Running) {
                    return; // stopped or reinstalled
                }
                match st.child.as_mut().map(|c| c.try_wait()) {
                    Some(Ok(Some(status))) => Some(status.to_string()),
                    Some(Err(e)) => Some(e.to_string()),
                    Some(Ok(None)) => None,
                    None => return,
                }
            };
            if let Some(status) = exited {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let tail = slot.log.tail(4).join("\n");
                let mut st = slot.lock();
                st.child = None;
                st.state = EngineState::Error;
                st.error = Some(format!(
                    "{} stopped unexpectedly ({status}).\n{tail}",
                    id.display_name()
                ));
                return;
            }
            let state = slot.lock().state;
            if state == EngineState::Starting {
                if self.probe(id).await {
                    slot.lock().state = EngineState::Running;
                    slot.log
                        .push(format!("{} is ready at {}", id.display_name(), id.base_url()));
                } else if started.elapsed() > start_limit {
                    let mut st = slot.lock();
                    if let Some(mut child) = st.child.take() {
                        process::kill_tree(&mut child);
                    }
                    st.state = EngineState::Error;
                    st.error = Some(format!(
                        "{} did not answer within {}s.",
                        id.display_name(),
                        start_limit.as_secs()
                    ));
                    return;
                }
            } else {
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        }
    }

    pub fn stop(&self, id: EngineId) -> Result<(), EngineError> {
        let slot = &self.slots[&id];
        let mut st = slot.lock();
        match st.state {
            EngineState::Installing => return Err(EngineError::Busy),
            EngineState::External => return Err(EngineError::External),
            _ => {}
        }
        if let Some(mut child) = st.child.take() {
            process::kill_tree(&mut child);
            slot.log.push(format!("Stopped {}", id.display_name()));
        }
        st.state = if st.installed.is_some() {
            EngineState::Stopped
        } else {
            EngineState::NotInstalled
        };
        Ok(())
    }

    /// The listening models, with which are downloaded.
    pub fn listening_models(&self) -> Vec<voice::ListeningModel> {
        voice::listening_models(&self.models_root)
    }

    pub fn listening_downloaded(&self, id: &str) -> bool {
        voice::listening_downloaded(&self.models_root, id)
    }

    /// The listening model download in progress (with how far it's got), or the last failure.
    pub fn listening_fetch(&self) -> ListeningFetch {
        let mut f = self.listening.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(m) = &f.model {
            f.progress = voice::listening_progress(&self.models_root, m);
        }
        f
    }

    /// Download a listening model in the background; `then` runs once it's there.
    pub fn fetch_listening(
        self: &Arc<Self>,
        id: &str,
        then: impl FnOnce() + Send + 'static,
    ) -> Result<(), EngineError> {
        if !voice::is_listening_model(id) {
            return Err(EngineError::NotInstalled);
        }
        {
            let mut f = self.listening.lock().unwrap_or_else(|p| p.into_inner());
            if f.model.is_some() {
                return Err(EngineError::Busy);
            }
            *f = ListeningFetch { model: Some(id.to_string()), ..Default::default() };
        }
        let me = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let slot = me.slots[&EngineId::Voice].clone();
            slot.log.push(format!("Downloading the listening model {id}"));
            let result = voice::fetch_listening(&slot.dir, &me.models_root, &id, &slot.log).await;
            let mut f = me.listening.lock().unwrap_or_else(|p| p.into_inner());
            f.model = None;
            f.progress = None;
            match result {
                Ok(()) if voice::listening_downloaded(&me.models_root, &id) => {
                    slot.log.push(format!("Listening model {id} ready"));
                    drop(f);
                    then();
                }
                other => {
                    let e = other.err().map(|e| format!("{e:#}")).unwrap_or_else(|| "The download didn't finish.".into());
                    slot.log.push(format!("ERROR: {e}"));
                    f.failed = Some(id);
                    f.error = Some(e);
                }
            }
        });
        Ok(())
    }

    /// The ai-toolkit Outpost installs (holds `run.py` and `venv`).
    pub fn trainer_dir(&self) -> PathBuf {
        self.trainer.dir.join("app")
    }

    pub fn trainer_install(&self) -> TrainerInstall {
        let st = self.trainer.lock();
        TrainerInstall {
            installed: st.installed.is_some(),
            installing: st.state == EngineState::Installing,
            version: st.installed.as_ref().map(|i| i.version.clone()),
            progress: st.progress.clone(),
            error: st.error.clone(),
            dir: self.trainer_dir().to_string_lossy().to_string(),
            pinned_version: versions::AI_TOOLKIT_VERSION.to_string(),
            log_tail: self.trainer.log.tail(20),
        }
    }

    /// Download and set up ai-toolkit in the background (or update it to the pinned version).
    pub fn install_trainer(self: &Arc<Self>) -> Result<(), EngineError> {
        let slot = self.trainer.clone();
        {
            let mut st = slot.lock();
            if st.state == EngineState::Installing {
                return Err(EngineError::Busy);
            }
            st.state = EngineState::Installing;
            st.error = None;
            st.progress = None;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let ctx = Ctx {
                slot: slot.clone(),
                engines_root: me.engines_root.clone(),
                client: me.download_client.clone(),
                gpus: me.gpus().await,
            };
            slot.log.push("Installing ai-toolkit");
            // A new install replaces the old one: it's not installed until this one finishes.
            let _ = std::fs::remove_file(slot.marker());
            slot.lock().installed = None;
            match ai_toolkit::install(&ctx).await {
                Ok(installed) => {
                    let saved = serde_json::to_vec_pretty(&installed)
                        .map_err(anyhow::Error::from)
                        .and_then(|bytes| std::fs::write(slot.marker(), bytes).map_err(anyhow::Error::from));
                    if let Err(e) = saved {
                        fail(&slot, format!("Installed, but could not save the install record: {e}"));
                        return;
                    }
                    slot.log.push(format!("ai-toolkit {} installed", installed.version));
                    let mut st = slot.lock();
                    st.installed = Some(installed);
                    st.state = EngineState::Stopped;
                    st.progress = None;
                }
                Err(e) => fail(&slot, format!("{e:#}")),
            }
        });
        Ok(())
    }

    /// Stop every engine Outpost started. Call on exit.
    pub fn shutdown(&self) {
        for id in EngineId::ALL {
            let _ = self.stop(id);
        }
    }
}

fn fail(slot: &Slot, message: String) {
    slot.log.push(format!("ERROR: {message}"));
    let mut st = slot.lock();
    st.state = EngineState::Error;
    st.error = Some(message);
    st.progress = None;
}

fn read_marker(path: &Path) -> Option<Installed> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torch_variant_follows_gpu() {
        assert_eq!(torch_variant(&[]), "cpu");
        let g = Gpu {
            vendor: "nvidia".into(),
            name: "RTX".into(),
            vram_mb: 16000,
        };
        assert_eq!(torch_variant(&[g]), "cu130");
    }

    #[tokio::test]
    async fn fresh_folder_means_not_installed_and_start_refuses() {
        let dir = std::env::temp_dir().join(format!("outpost-engines-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = EngineManager::new(&dir);
        for id in EngineId::ALL {
            assert_eq!(m.slots[&id].lock().state, EngineState::NotInstalled);
        }
        assert!(matches!(
            m.start(EngineId::Comfyui),
            Err(EngineError::NotInstalled) | Err(EngineError::External)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
