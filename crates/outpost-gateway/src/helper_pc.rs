//! This PC as a helper for an Outpost server (the other side of [`crate::helpers`]).
//!
//! The desktop app, while it uses a server, can lend this PC's graphics card for Create jobs
//! the server can't run. It checks in every few seconds with what this PC can run; when
//! the server hands it a job it starts ComfyUI here (only ComfyUI: never Ollama), fetches the
//! job's pictures and LoRA, runs it, reports progress (stopping if it's cancelled), and sends
//! the results back to the server's gallery. Nothing is kept here but the LoRA (cached for
//! next time) and the models.
//!
//! **Paused** (e.g. while gaming) it takes no jobs and stops ComfyUI, so the card is free.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use outpost_core::jobs;
use outpost_engines::catalog::Fit;
use outpost_engines::comfy::{ComfyError, JobProgress};
use outpost_engines::packs;
use outpost_engines::{EngineId, EngineState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Notify;

use crate::{AppState, Config};

/// How often to check in.
const EVERY: Duration = Duration::from_secs(5);
/// Unload ComfyUI's models after this long without a job.
const IDLE_FREE: Duration = Duration::from_secs(5 * 60);

/// What the Settings screen shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HelperStatus {
    pub enabled: bool,
    pub paused: bool,
    /// `off`, `paused`, `ready` (waiting for work), `working` or `problem`.
    pub state: String,
    pub message: Option<String>,
    /// The workflow it's running now.
    pub pack_id: Option<String>,
    pub progress: Option<JobProgress>,
    /// Jobs finished since the app started.
    pub done: u32,
    /// This PC's card, in MiB.
    pub vram_mb: u64,
    /// Workflows downloaded here.
    pub ready_packs: Vec<String>,
    /// ComfyUI is installed here.
    pub comfyui: bool,
    /// ComfyUI is installing here now.
    pub comfyui_installing: bool,
    /// Why the last ComfyUI install here failed.
    pub comfyui_error: Option<String>,
}

/// A workflow as this PC sees it.
#[derive(Debug, Clone, Serialize)]
pub struct LocalPack {
    pub id: String,
    pub name: String,
    pub vram_gb: f32,
    /// How it fits this PC's card.
    pub fit: Fit,
    pub models: outpost_engines::assets::PackModels,
}

pub struct HelperPc {
    /// This PC's engines, models and ComfyUI (its own Outpost data folder; no database).
    local: AppState,
    enabled: AtomicBool,
    paused: AtomicBool,
    status: Mutex<HelperStatus>,
    wake: Notify,
}

/// Where the server is and who to sign in as (an owner's paired-device token; the first
/// that works is used).
pub trait Server: Send + Sync + 'static {
    fn url(&self) -> String;
    fn tokens(&self) -> Vec<String>;
}

#[derive(Debug, Deserialize)]
struct Poll {
    job: Option<Job>,
}

#[derive(Debug, Deserialize)]
struct Job {
    job_id: String,
    pack_id: String,
    inputs: Value,
    #[serde(default)]
    images: Vec<String>,
    lora: Option<String>,
    lora_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ProgressReply {
    cancel: bool,
}

impl HelperPc {
    /// This PC's Outpost data folder (where its ComfyUI and models are).
    pub async fn new(data_dir: &Path, enabled: bool, paused: bool) -> anyhow::Result<Arc<Self>> {
        let pool = outpost_core::db::open_in_memory().await?;
        let mut config = Config::local_default();
        config.data_dir = data_dir.to_path_buf();
        let local = AppState::with_pool(pool, config);
        Ok(Arc::new(Self {
            local,
            enabled: AtomicBool::new(enabled),
            paused: AtomicBool::new(paused),
            status: Mutex::new(HelperStatus { enabled, paused, state: "off".into(), ..Default::default() }),
            wake: Notify::new(),
        }))
    }

    /// This PC's engines (so the app can stop them when it closes).
    pub fn engines(&self) -> Arc<outpost_engines::EngineManager> {
        self.local.engines.clone()
    }

    pub fn set(&self, enabled: bool, paused: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.paused.store(paused, Ordering::Relaxed);
        self.edit(|s| {
            s.enabled = enabled;
            s.paused = paused;
        });
        self.wake.notify_one();
    }

    pub fn status(&self) -> HelperStatus {
        self.status.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn edit(&self, f: impl FnOnce(&mut HelperStatus)) {
        f(&mut self.status.lock().unwrap_or_else(|p| p.into_inner()));
    }

    fn say(&self, state: &str, message: Option<String>) {
        self.edit(|s| {
            s.state = state.into();
            s.message = message;
        });
    }

    /// Every workflow, with whether it's downloaded here and fits this card.
    pub async fn packs(&self) -> Vec<LocalPack> {
        let hw = crate::routes::models::measure(&self.local).await;
        packs::built_in()
            .into_iter()
            .map(|p| LocalPack {
                fit: crate::routes::create::pack_fit(p.vram_gb, &hw),
                models: self.local.assets.status(&p),
                id: p.id,
                name: p.name,
                vram_gb: p.vram_gb,
            })
            .collect()
    }

    /// Download a workflow's models here (in the background; [`Self::packs`] shows progress).
    pub fn install_pack(&self, id: &str) -> anyhow::Result<()> {
        let pack = packs::find(id).ok_or_else(|| anyhow::anyhow!("No workflow called {id}."))?;
        self.local.assets.install(&pack)
    }

    /// Install ComfyUI here (in the background).
    pub fn install_comfyui(&self) -> anyhow::Result<()> {
        self.local.engines.install(EngineId::Comfyui).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.wake.notify_one();
        Ok(())
    }

    /// ComfyUI here: installed, installing, and the last install's error.
    async fn comfyui_here(&self) -> (bool, bool, Option<String>) {
        let status = self.local.engines.status().await;
        match status.engines.into_iter().find(|e| e.id == EngineId::Comfyui) {
            Some(e) => (
                matches!(e.state, EngineState::Stopped | EngineState::Starting | EngineState::Running | EngineState::External)
                    || (e.state == EngineState::Error && e.managed),
                e.state == EngineState::Installing,
                e.error.filter(|_| e.state == EngineState::Error),
            ),
            None => (false, false, None),
        }
    }

    /// Give the card back: unload ComfyUI's models and stop the ComfyUI Outpost started.
    async fn release_card(&self) {
        if self.local.comfy.is_up().await {
            self.local.comfy.free().await;
        }
        let _ = self.local.engines.stop(EngineId::Comfyui);
    }

    /// Check in and run jobs, until the app closes.
    pub async fn run(self: Arc<Self>, server: impl Server) {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client");
        let mut token: Option<String> = None;
        let mut was_working = false;
        let mut last_job: Option<Instant> = None;
        loop {
            let enabled = self.enabled.load(Ordering::Relaxed);
            let paused = self.paused.load(Ordering::Relaxed);
            if !enabled {
                self.say("off", None);
                if was_working {
                    self.release_card().await;
                    was_working = false;
                }
                let _ = tokio::time::timeout(Duration::from_secs(60), self.wake.notified()).await;
                continue;
            }
            if paused && was_working {
                self.release_card().await;
                was_working = false;
            }
            if was_working && last_job.is_some_and(|t| t.elapsed() > IDLE_FREE) {
                // Idle a while: keep ComfyUI up but hand back the card's memory.
                self.local.comfy.free().await;
                last_job = None;
            }

            let hw = crate::routes::models::measure_cached(&self.local).await;
            let (comfyui, installing, install_error) = self.comfyui_here().await;
            let ready: Vec<String> = if comfyui {
                packs::built_in()
                    .into_iter()
                    .filter(|p| self.local.assets.status(p).ready)
                    .map(|p| p.id)
                    .collect()
            } else {
                Vec::new()
            };
            self.edit(|s| {
                s.vram_mb = hw.vram_mb;
                s.ready_packs = ready.clone();
                s.comfyui = comfyui;
                s.comfyui_installing = installing;
                s.comfyui_error = install_error.clone();
            });
            let body = json!({"vram_mb": hw.vram_mb, "ram_mb": hw.ram_mb, "ready_packs": ready, "paused": paused});

            match self.check_in(&client, &server, &mut token, &body).await {
                Err(problem) => self.say("problem", Some(problem)),
                Ok(None) => {
                    let waiting = if paused {
                        ("paused", Some("Paused: not taking jobs, and ComfyUI is stopped.".to_string()))
                    } else if !comfyui {
                        ("ready", Some("ComfyUI isn't installed on this PC yet.".to_string()))
                    } else if ready.is_empty() {
                        ("ready", Some("No workflows are downloaded on this PC yet.".to_string()))
                    } else {
                        ("ready", None)
                    };
                    self.say(waiting.0, waiting.1);
                }
                Ok(Some(job)) => {
                    let t = token.clone().unwrap_or_default();
                    was_working = true;
                    self.edit(|s| {
                        s.state = "working".into();
                        s.message = None;
                        s.pack_id = Some(job.pack_id.clone());
                        s.progress = None;
                    });
                    self.work(&client, &server.url(), &t, job).await;
                    self.edit(|s| {
                        s.pack_id = None;
                        s.progress = None;
                        s.done += 1;
                    });
                    last_job = Some(Instant::now());
                    // Straight back for the next one.
                    continue;
                }
            }
            let _ = tokio::time::timeout(EVERY, self.wake.notified()).await;
        }
    }

    /// One check-in; tries each sign-in until one is an owner's.
    async fn check_in(
        &self,
        client: &reqwest::Client,
        server: &impl Server,
        token: &mut Option<String>,
        body: &Value,
    ) -> Result<Option<Job>, String> {
        let mut tokens = server.tokens();
        if let Some(t) = token.as_ref() {
            tokens.retain(|x| x != t);
            tokens.insert(0, t.clone());
        }
        if tokens.is_empty() {
            return Err("Nobody is signed in to the server on this PC.".into());
        }
        let url = format!("{}/api/helpers/poll", server.url());
        let mut refused = false;
        for t in tokens {
            let resp = client
                .post(&url)
                .bearer_auth(&t)
                .timeout(Duration::from_secs(20))
                .json(body)
                .send()
                .await
                .map_err(|_| "Can't reach the server right now.".to_string())?;
            match resp.status().as_u16() {
                200 => {
                    *token = Some(t);
                    let poll: Poll = resp.json().await.map_err(|e| format!("The server's reply was garbled: {e}"))?;
                    return Ok(poll.job);
                }
                401 | 403 => refused = true,
                404 => return Err("The server is too old for helper PCs: update it first.".into()),
                code => return Err(format!("The server said {code}.")),
            }
        }
        *token = None;
        Err(if refused {
            "Only an owner's sign-in can lend this PC. Sign in as an owner on this PC.".into()
        } else {
            "The server refused this PC.".into()
        })
    }

    async fn work(&self, client: &reqwest::Client, base: &str, token: &str, job: Job) {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(JobProgress { phase: "Getting ready".into(), ..Default::default() }));
        let prompt_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        // Report progress (and hear about a cancel) while the job runs, every few seconds,
        // and at least every 15 s so the server knows this PC is still there.
        let reporter = {
            let (client, cancel, progress, prompt_id) = (client.clone(), cancel.clone(), progress.clone(), prompt_id.clone());
            let url = format!("{base}/api/helpers/jobs/{}/progress", job.job_id);
            let token = token.to_string();
            let comfy = self.local.comfy.clone();
            tokio::spawn(async move {
                let mut sent: Option<(String, u32)> = None;
                let mut sent_at = Instant::now() - Duration::from_secs(60);
                loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let now = progress.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    let key = (now.phase.clone(), now.step);
                    if sent.as_ref() == Some(&key) && sent_at.elapsed() < Duration::from_secs(15) {
                        continue;
                    }
                    let reply = client.post(&url).bearer_auth(&token).timeout(Duration::from_secs(15)).json(&now).send().await;
                    let stop = match reply {
                        Ok(r) if r.status().is_success() => r.json::<ProgressReply>().await.map(|p| p.cancel).unwrap_or(false),
                        // The server no longer knows the job (it restarted, or gave up on it).
                        Ok(r) if r.status().as_u16() == 404 => true,
                        _ => false,
                    };
                    sent = Some(key);
                    sent_at = Instant::now();
                    if stop && !cancel.swap(true, Ordering::Relaxed) {
                        let id = prompt_id.lock().unwrap_or_else(|p| p.into_inner()).clone();
                        if let Some(id) = id {
                            comfy.dequeue(&id).await;
                            comfy.interrupt(&id).await;
                        }
                    }
                }
            })
        };

        let outcome = self.run_job(client, base, token, &job, &cancel, &progress, &prompt_id).await;
        reporter.abort();
        let finish = match &outcome {
            Ok(()) => json!({"ok": true}),
            Err(ComfyError::Interrupted) => json!({"ok": false, "cancelled": true}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        };
        if let Err(e) = &outcome {
            tracing::warn!(job = %job.job_id, pack = %job.pack_id, error = %e, "a job for the server didn't finish");
        }
        let url = format!("{base}/api/helpers/jobs/{}/finish", job.job_id);
        for _ in 0..3 {
            match client.post(&url).bearer_auth(token).timeout(Duration::from_secs(20)).json(&finish).send().await {
                Ok(_) => break,
                Err(_) => tokio::time::sleep(Duration::from_secs(5)).await,
            }
        }
        crate::create::clear_comfy_copies(&self.local).await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_job(
        &self,
        client: &reqwest::Client,
        base: &str,
        token: &str,
        job: &Job,
        cancel: &Arc<AtomicBool>,
        progress: &Arc<Mutex<JobProgress>>,
        prompt_id: &Arc<Mutex<Option<String>>>,
    ) -> Result<(), ComfyError> {
        let set_phase = |phase: &str| {
            let mut p = progress.lock().unwrap_or_else(|p| p.into_inner());
            *p = JobProgress { phase: phase.into(), ..Default::default() };
        };
        let fail = |m: String| ComfyError::Failed(m);
        let pack = packs::find(&job.pack_id).ok_or_else(|| fail(format!("This PC's Outpost doesn't know {}: update it.", job.pack_id)))?;
        if !self.local.assets.status(&pack).ready {
            return Err(fail(format!("{}'s models aren't downloaded on this PC.", pack.name)));
        }
        set_phase("Starting ComfyUI");
        crate::create::ensure_comfyui(&self.local).await?;
        crate::create::free_ollama(&self.local).await;
        let comfy = &self.local.comfy;
        let job_url = format!("{base}/api/helpers/jobs/{}", job.job_id);

        // Pictures, from the server to this ComfyUI.
        set_phase("Fetching pictures");
        let values: serde_json::Map<String, Value> = job.inputs.as_object().cloned().unwrap_or_default();
        let mut uploaded = HashMap::new();
        for key in &job.images {
            let data = get_bytes(client, &format!("{job_url}/input/{key}"), token)
                .await
                .map_err(|e| fail(format!("Couldn't fetch a picture from the server: {e}")))?;
            let name = values
                .get(key)
                .and_then(|v| v.get("$file"))
                .and_then(Value::as_str)
                .and_then(|rel| rel.rsplit('/').next())
                .unwrap_or("input.png")
                .to_string();
            uploaded.insert(key.clone(), comfy.upload_image(data, &name).await?);
        }
        // The LoRA, kept here for next time.
        let mut loras = Vec::new();
        if let Some(rel) = &job.lora {
            let path = self.local.assets.lora_path(rel).ok_or_else(|| fail("The LoRA's name isn't usable.".into()))?;
            let have = std::fs::metadata(&path).ok().map(|m| m.len());
            if have.is_none() || (job.lora_bytes.is_some() && have != job.lora_bytes) {
                set_phase("Fetching the LoRA");
                download(client, &format!("{job_url}/lora"), token, &path)
                    .await
                    .map_err(|e| fail(format!("Couldn't fetch the LoRA from the server: {e}")))?;
            }
            loras.push(rel.clone());
        }
        let built = pack
            .build(&values, jobs::random_seed, &uploaded, &loras)
            .map_err(|e| ComfyError::Rejected(e.to_string()))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(ComfyError::Interrupted);
        }
        let id = comfy.submit(&built.graph).await?;
        *prompt_id.lock().unwrap_or_else(|p| p.into_inner()) = Some(id.clone());
        if cancel.load(Ordering::Relaxed) {
            comfy.dequeue(&id).await;
            comfy.interrupt(&id).await;
        }
        let limit = match pack.output.kind {
            packs::OutputKind::Video => Duration::from_secs(90 * 60),
            packs::OutputKind::Image => Duration::from_secs(30 * 60),
            // Shaping, texturing and baking a model takes a while on a smaller card.
            packs::OutputKind::Model => Duration::from_secs(45 * 60),
        };
        let files = {
            let cancel = cancel.clone();
            let progress = progress.clone();
            let status = &self.status;
            comfy
                .wait(&id, limit, move || cancel.load(Ordering::Relaxed), move |p| {
                    status.lock().unwrap_or_else(|e| e.into_inner()).progress = Some(p.clone());
                    *progress.lock().unwrap_or_else(|e| e.into_inner()) = p;
                })
                .await?
        };

        set_phase("Sending the results");
        let mut sent = 0;
        for (_, file) in files.iter().filter(|(node, _)| *node == pack.output.node) {
            let data = comfy.fetch(file).await?;
            let mut url = format!("{job_url}/result?name={}", urlencode(&file.filename));
            if let Some(seed) = built.seed {
                url.push_str(&format!("&seed={seed}"));
            }
            let resp = client
                .post(&url)
                .bearer_auth(token)
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .timeout(Duration::from_secs(600))
                .body(data)
                .send()
                .await
                .map_err(|e| fail(format!("Couldn't send a result to the server: {e}")))?;
            if !resp.status().is_success() {
                return Err(fail(format!("The server didn't take a result ({}).", resp.status())));
            }
            sent += 1;
        }
        if sent == 0 {
            return Err(fail("ComfyUI finished but returned nothing.".into()));
        }
        Ok(())
    }
}

async fn get_bytes(client: &reqwest::Client, url: &str, token: &str) -> Result<Vec<u8>, String> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("the server said {}", resp.status()));
    }
    Ok(resp.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

/// Stream a file to `path` (via a temporary name, so a cut-off download leaves nothing).
async fn download(client: &reqwest::Client, url: &str, token: &str, path: &Path) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut resp = client.get(url).bearer_auth(token).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("the server said {}", resp.status()));
    }
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    let part = path.with_extension("part");
    let mut out = tokio::fs::File::create(&part).await.map_err(|e| e.to_string())?;
    let result = async {
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            out.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        out.flush().await.map_err(|e| e.to_string())
    }
    .await;
    drop(out);
    match result {
        Ok(()) => tokio::fs::rename(&part, path).await.map_err(|e| e.to_string()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_names_are_escaped_for_the_url() {
        assert_eq!(super::urlencode("out_00001_.png"), "out_00001_.png");
        assert_eq!(super::urlencode("a b&c.png"), "a%20b%26c.png");
    }
}
