//! Model files for ComfyUI packs, kept in Outpost's own models folder
//! (`<data>/models/comfyui/<folder>/<file>`) so reinstalling ComfyUI never deletes
//! them. ComfyUI is pointed at this folder with an extra model paths file.
//!
//! Downloads run in the background, resume where they stopped, and are checked
//! against the pack's SHA-256 when it gives one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use utoipa::ToSchema;

use crate::packs::{ModelFile, Pack};

/// ComfyUI model folders Outpost manages. Anything else in a pack is refused.
pub const FOLDERS: [&str; 9] = [
    "checkpoints",
    "diffusion_models",
    "text_encoders",
    "vae",
    "loras",
    "clip_vision",
    "upscale_models",
    "model_patches",
    "audio_encoders",
];

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FileStatus {
    pub file: String,
    pub folder: String,
    pub size_gb: f32,
    pub present: bool,
}

/// A pack's model download, while it runs and after.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Setup {
    pub running: bool,
    /// The file downloading now.
    pub current: Option<String>,
    /// Over all of the pack's missing files.
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub percent: Option<f32>,
    pub error: Option<String>,
    /// Which missing file is downloading (1-based) and how many there are.
    pub file_index: u32,
    pub file_count: u32,
    /// Bytes a second, once known.
    pub speed_bps: Option<u64>,
    /// Seconds left at the current speed.
    pub eta_secs: Option<u64>,
    /// When it finished or failed (Unix seconds).
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackModels {
    /// Every file is present: the pack can run.
    pub ready: bool,
    pub files: Vec<FileStatus>,
    /// Still to download, in GB.
    pub missing_gb: f32,
    pub setup: Option<Setup>,
}

pub struct AssetStore {
    root: PathBuf,
    client: reqwest::Client,
    setups: Mutex<HashMap<String, Setup>>,
    /// One download per file at a time, even when two packs share it.
    file_locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

impl AssetStore {
    /// `models_root` is `<data>/models`; pack files live in its `comfyui` folder.
    pub fn new(models_root: &Path) -> Arc<Self> {
        Arc::new(AssetStore {
            root: models_root.join("comfyui"),
            client: reqwest::Client::builder()
                .user_agent(concat!("Outpost/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(15))
                // A connection that goes quiet fails (and resumes on retry) rather than hanging.
                .read_timeout(Duration::from_secs(60))
                .build()
                .expect("HTTP client"),
            setups: Mutex::new(HashMap::new()),
            file_locks: Mutex::new(HashMap::new()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn loras_dir(&self) -> PathBuf {
        self.root.join("loras")
    }

    fn setups(&self) -> std::sync::MutexGuard<'_, HashMap<String, Setup>> {
        self.setups.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Where a pack's file lives. Refuses odd folders and names that aren't plain files.
    pub fn path_of(&self, m: &ModelFile) -> anyhow::Result<PathBuf> {
        if !FOLDERS.contains(&m.folder.as_str()) {
            bail!("unknown model folder {}", m.folder);
        }
        if m.file.is_empty() || m.file.contains(['/', '\\']) || m.file.contains("..") {
            bail!("bad model file name {}", m.file);
        }
        Ok(self.root.join(&m.folder).join(&m.file))
    }

    pub fn status(&self, pack: &Pack) -> PackModels {
        let files: Vec<FileStatus> = pack
            .models
            .iter()
            .map(|m| FileStatus {
                file: m.file.clone(),
                folder: m.folder.clone(),
                size_gb: m.size_gb,
                present: self.path_of(m).map(|p| p.is_file()).unwrap_or(false),
            })
            .collect();
        let missing_gb = files.iter().filter(|f| !f.present).map(|f| f.size_gb).sum();
        PackModels {
            ready: files.iter().all(|f| f.present),
            files,
            missing_gb,
            setup: self.setups().get(&pack.id).cloned(),
        }
    }

    /// Every pack download since Outpost started, running or finished, by pack id.
    pub fn all_setups(&self) -> Vec<(String, Setup)> {
        self.setups().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// LoRA files in the LoRA folder (`.safetensors`), by name, for packs to offer.
    pub fn loras(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(self.loras_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.to_ascii_lowercase().ends_with(".safetensors"))
            .collect();
        out.sort_by_key(|n| n.to_lowercase());
        out
    }

    /// What a LoRA file in the LoRA folder says about itself (from its header): the
    /// model it was trained for and its trigger word, when it recorded them.
    pub fn lora_info(&self, file: &str) -> Option<LoraInfo> {
        if file.contains(['/', '\\']) || file.contains("..") || !file.to_ascii_lowercase().ends_with(".safetensors") {
            return None;
        }
        lora_info_at(&self.loras_dir().join(file))
    }

    /// Download the pack's missing files in the background.
    pub fn install(self: &Arc<Self>, pack: &Pack) -> anyhow::Result<()> {
        let missing: Vec<(ModelFile, PathBuf)> = pack
            .models
            .iter()
            .map(|m| Ok((m.clone(), self.path_of(m)?)))
            .collect::<anyhow::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(_, p)| !p.is_file())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        {
            let mut setups = self.setups();
            if setups.get(&pack.id).is_some_and(|s| s.running) {
                bail!("This pack is already downloading.");
            }
            let total: u64 = missing
                .iter()
                .map(|(m, _)| m.bytes.unwrap_or_else(|| gb_to_bytes(m.size_gb)))
                .sum();
            // Room for the files plus 2 GB to spare.
            std::fs::create_dir_all(&self.root).ok();
            if let Ok(free) = fs2::available_space(&self.root) {
                if free < total + 2 * 1024 * 1024 * 1024 {
                    bail!(
                        "Not enough disk space: these models need about {:.1} GB and {:.1} GB is free.",
                        total as f64 / 1e9,
                        free as f64 / 1e9
                    );
                }
            }
            setups.insert(
                pack.id.clone(),
                Setup {
                    running: true,
                    current: None,
                    done_bytes: 0,
                    total_bytes: total,
                    percent: Some(0.0),
                    error: None,
                    file_index: 0,
                    file_count: missing.len() as u32,
                    speed_bps: None,
                    eta_secs: None,
                    finished_at: None,
                },
            );
        }
        let me = self.clone();
        let id = pack.id.clone();
        tokio::spawn(async move {
            let result = me.download_all(&id, &missing).await;
            let mut setups = me.setups();
            if let Some(s) = setups.get_mut(&id) {
                s.running = false;
                s.current = None;
                s.speed_bps = None;
                s.eta_secs = None;
                s.finished_at = Some(unix_now());
                match result {
                    Ok(()) => s.percent = Some(100.0),
                    Err(e) => {
                        tracing::warn!(pack = %id, error = %e, "pack model download failed");
                        s.error = Some(format!("{e:#}"));
                    }
                }
            }
        });
        Ok(())
    }

    fn file_lock(&self, path: &Path) -> Arc<tokio::sync::Mutex<()>> {
        self.file_locks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(path.to_path_buf())
            .or_default()
            .clone()
    }

    async fn download_all(&self, pack_id: &str, files: &[(ModelFile, PathBuf)]) -> anyhow::Result<()> {
        let mut before: u64 = 0;
        let mut rate = crate::rate::Rate::new();
        for (index, (m, path)) in files.iter().enumerate() {
            if let Some(s) = self.setups().get_mut(pack_id) {
                s.current = Some(m.file.clone());
                s.file_index = index as u32 + 1;
            }
            // Another pack may be fetching the same file: wait for it, then skip.
            let lock = self.file_lock(path);
            let _guard = lock.lock().await;
            if path.is_file() {
                before += m.bytes.unwrap_or_else(|| gb_to_bytes(m.size_gb));
                continue;
            }
            let base = before;
            let mut sized = false;
            self.download_one(m, path, |done, total| {
                if let Some(s) = self.setups().get_mut(pack_id) {
                    // The real size replaces the pack's estimate, once, when the server says.
                    if let (false, Some(t)) = (sized, total) {
                        s.total_bytes = s
                            .total_bytes
                            .saturating_sub(m.bytes.unwrap_or_else(|| gb_to_bytes(m.size_gb)))
                            .saturating_add(t);
                        sized = true;
                    }
                    s.done_bytes = base + done;
                    if s.total_bytes > 0 {
                        s.percent = Some((s.done_bytes as f32 / s.total_bytes as f32 * 100.0).min(100.0));
                    }
                    rate.update(s.done_bytes);
                    s.speed_bps = rate.bytes_per_sec();
                    s.eta_secs = rate.eta_secs(s.total_bytes.saturating_sub(s.done_bytes));
                }
            })
            .await
            .with_context(|| format!("downloading {}", m.file))?;
            before += std::fs::metadata(path)
                .map(|md| md.len())
                .unwrap_or(gb_to_bytes(m.size_gb));
        }
        Ok(())
    }

    /// Download one file to `path`, resuming a `.part` left by an earlier try.
    async fn download_one(
        &self,
        m: &ModelFile,
        path: &Path,
        mut progress: impl FnMut(u64, Option<u64>),
    ) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let part = PathBuf::from(format!("{}.part", path.display()));
        let mut hasher = Sha256::new();
        let mut have: u64 = 0;
        // Resuming: hash what is already there so the final check covers the whole
        // file. (Every built-in pack pins a checksum; without one this is skipped.)
        if m.sha256.is_none() {
            have = tokio::fs::metadata(&part).await.map(|md| md.len()).unwrap_or(0);
        } else if let Ok(mut f) = tokio::fs::File::open(&part).await {
            // Hash what we already have so the final check covers the whole file.
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = f.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                have += n as u64;
            }
        }
        let mut req = self.client.get(&m.url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = req.send().await.with_context(|| format!("could not reach {}", m.url))?;
        let status = resp.status();
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && have > 0 {
            // The part file is already complete.
        } else if !status.is_success() {
            bail!("the server refused the download ({status})");
        }
        let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
        if have > 0 && !resumed && status.is_success() {
            // Server ignored the range: start again from scratch.
            hasher = Sha256::new();
            have = 0;
        }
        let total = if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            Some(have)
        } else {
            resp.content_length().map(|len| len + have)
        };
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(resumed)
            .write(true)
            .truncate(!resumed && status.is_success())
            .open(&part)
            .await
            .with_context(|| format!("cannot write {}", part.display()))?;
        let mut done = have;
        progress(done, total);
        if status.is_success() {
            let mut last = Instant::now();
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("download interrupted (it will resume next time)")?;
                hasher.update(&chunk);
                file.write_all(&chunk).await?;
                done += chunk.len() as u64;
                if last.elapsed() >= Duration::from_millis(400) {
                    progress(done, total);
                    last = Instant::now();
                }
            }
        }
        file.flush().await?;
        drop(file);
        progress(done, total);
        if let Some(t) = total {
            if done < t {
                bail!("download ended early (it will resume next time)");
            }
        }
        if let Some(expected) = m.bytes {
            if done != expected {
                let _ = tokio::fs::remove_file(&part).await;
                bail!("{} came out the wrong size; please try again", m.file);
            }
        }
        let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if let Some(expected) = &m.sha256 {
            if !expected.eq_ignore_ascii_case(&hash) {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(anyhow!("{} is damaged (checksum mismatch); please try again", m.file));
            }
        }
        tokio::fs::rename(&part, path).await?;
        Ok(())
    }
}

fn gb_to_bytes(gb: f32) -> u64 {
    (gb as f64 * 1e9) as u64
}

/// The `extra_model_paths.yaml` that points ComfyUI at Outpost's models folder.
pub fn extra_model_paths_yaml(root: &Path) -> String {
    // Forward slashes work on Windows too and need no YAML escaping.
    let base = root.to_string_lossy().replace('\\', "/");
    let mut out = format!("outpost:\n    base_path: \"{base}\"\n    is_default: true\n");
    for f in FOLDERS {
        out.push_str(&format!("    {f}: {f}\n"));
    }
    out
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs;

    #[test]
    fn files_stay_inside_their_folders() {
        let store = AssetStore::new(Path::new("/data/models"));
        let ok = ModelFile {
            file: "ae.safetensors".into(),
            folder: "vae".into(),
            size_gb: 0.3,
            url: "https://example.com/ae".into(),
            sha256: None,
            bytes: None,
        };
        assert!(store.path_of(&ok).unwrap().ends_with("comfyui/vae/ae.safetensors"));
        for (folder, file) in [
            ("vae", "../x"),
            ("vae", "a/b"),
            ("../etc", "x"),
            ("custom_nodes", "x.py"),
        ] {
            let bad = ModelFile {
                folder: folder.into(),
                file: file.into(),
                ..ok.clone()
            };
            assert!(store.path_of(&bad).is_err(), "{folder}/{file}");
        }
    }

    #[test]
    fn status_and_loras_read_the_folder() {
        let dir = std::env::temp_dir().join(format!("outpost-assets-{}", std::process::id()));
        let store = AssetStore::new(&dir);
        let pack = packs::find("z_image_turbo").unwrap();
        let s = store.status(&pack);
        assert!(!s.ready);
        assert!(s.missing_gb > 20.0);

        for m in &pack.models {
            let p = store.path_of(m).unwrap();
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        assert!(store.status(&pack).ready);

        std::fs::create_dir_all(store.loras_dir()).unwrap();
        std::fs::write(store.loras_dir().join("d4nni.safetensors"), b"x").unwrap();
        std::fs::write(store.loras_dir().join("notes.txt"), b"x").unwrap();
        assert_eq!(store.loras(), vec!["d4nni.safetensors".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn yaml_points_at_the_folder() {
        let y = extra_model_paths_yaml(Path::new(r"C:\Users\Dan\AppData\Local\Outpost\models\comfyui"));
        assert!(y.contains("base_path: \"C:/Users/Dan/AppData/Local/Outpost/models/comfyui\""));
        assert!(y.contains("    loras: loras\n"));
        assert!(y.contains("is_default: true"));
    }
}

/// What a LoRA file says about itself.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LoraInfo {
    /// Made for Z-Image (so the Characters workflow can use it).
    pub z_image: bool,
    /// The base model it names, e.g. `zimage` or `z_image`.
    pub base: Option<String>,
    /// Its trigger word, when the trainer recorded one (ai-toolkit does).
    pub trigger: Option<String>,
    /// The name it was saved under by its trainer.
    pub trained_as: Option<String>,
}

/// Read a `.safetensors` header: an 8-byte length, then JSON naming every tensor, with
/// the trainer's notes under `__metadata__`.
pub fn lora_info_at(path: &Path) -> Option<LoraInfo> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut len = [0u8; 8];
    f.read_exact(&mut len).ok()?;
    let len = u64::from_le_bytes(len);
    if len == 0 || len > 50 * 1024 * 1024 {
        return None;
    }
    let mut header = vec![0u8; len as usize];
    f.read_exact(&mut header).ok()?;
    let header: serde_json::Value = serde_json::from_slice(&header).ok()?;
    Some(read_lora_header(&header))
}

fn read_lora_header(header: &serde_json::Value) -> LoraInfo {
    let meta = header.get("__metadata__");
    let text = |k: &str| meta.and_then(|m| m.get(k)).and_then(|v| v.as_str()).map(str::to_string);
    let base = text("ss_base_model_version").or_else(|| text("modelspec.architecture"));
    // Z-Image's transformer: `layers.N.attention...` with `adaLN_modulation`/`feed_forward`.
    let z_keys = header.as_object().is_some_and(|o| {
        o.keys().any(|k| k.contains("layers.0.attention.to_") || k.contains("layers.0.attention.qkv"))
            && o.keys().any(|k| k.contains("feed_forward.w"))
    });
    let z_named = base.as_deref().is_some_and(|b| {
        let b = b.to_ascii_lowercase().replace(['_', '-', ' '], "");
        b.contains("zimage")
    });
    // ai-toolkit and kohya: {"1_zk4t": {"zk4t": 1}}; the folder name after the repeat count.
    let trigger = text("trigger_word")
        .or_else(|| text("modelspec.trigger_phrase"))
        .or_else(|| {
            let freq: serde_json::Value = serde_json::from_str(&text("ss_tag_frequency")?).ok()?;
            let folder = freq.as_object()?.keys().next()?.clone();
            let word = folder.split_once('_').map(|(n, w)| if n.chars().all(|c| c.is_ascii_digit()) { w } else { folder.as_str() }).unwrap_or(&folder);
            Some(word.to_string())
        })
        .filter(|t| !t.trim().is_empty());
    LoraInfo {
        z_image: z_named || z_keys,
        base,
        trigger,
        trained_as: text("ss_output_name").or_else(|| text("name")),
    }
}

#[cfg(test)]
mod lora_info_tests {
    use super::*;

    #[test]
    fn lora_headers_tell_their_trigger_and_model() {
        let ai_toolkit = serde_json::json!({
            "__metadata__": {"ss_base_model_version": "zimage", "ss_tag_frequency": "{\"1_zk4t\": {\"zk4t\": 1}}", "name": "lora"},
            "diffusion_model.layers.0.attention.to_k.lora_A.weight": {},
            "diffusion_model.layers.0.feed_forward.w1.lora_A.weight": {}
        });
        let i = read_lora_header(&ai_toolkit);
        assert!(i.z_image);
        assert_eq!(i.trigger.as_deref(), Some("zk4t"));
        let kohya = serde_json::json!({
            "__metadata__": {"ss_base_model_version": "z_image", "ss_output_name": "char_zzz_kat_zimage"},
            "diffusion_model.layers.0.attention.to_k.lora_A.weight": {}
        });
        let i = read_lora_header(&kohya);
        assert!(i.z_image);
        assert_eq!(i.trigger, None);
        assert_eq!(i.trained_as.as_deref(), Some("char_zzz_kat_zimage"));
        let qwen = serde_json::json!({"transformer_blocks.0.attn.add_k_proj.alpha": {}});
        assert!(!read_lora_header(&qwen).z_image);
    }
}
