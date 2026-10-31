//! The voice service: Outpost's own small Python server (`voice/server.py`) running
//! faster-whisper for speech to text and Kokoro for speech, on the CPU, in its own
//! uv-built environment. Models live in `<data>/models/voice` and are fetched at
//! install; at run time it works offline.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use tokio::process::Command;

use crate::comfyui::{ensure_uv, uv_cmd, venv_python};
use crate::manager::{Ctx, Installed};
use crate::versions::{self, VOICE_PORT, VOICE_PYTHON, VOICE_VERSION};
use crate::{download, process};

const STEPS: u32 = 6;
const SERVER: &str = include_str!("../voice/server.py");

fn models_dir(models_root: &Path) -> PathBuf {
    models_root.join("voice")
}

fn write_server(dir: &Path) -> anyhow::Result<PathBuf> {
    let app = dir.join("app");
    std::fs::create_dir_all(&app)?;
    let path = app.join("server.py");
    std::fs::write(&path, SERVER)?;
    Ok(path)
}

pub async fn install(ctx: &Ctx, models_root: &Path) -> anyhow::Result<Installed> {
    let dir = ctx.slot.dir.clone();
    let log = &ctx.slot.log;

    ctx.step(1, STEPS, "Downloading uv (Python manager)");
    let uv = ensure_uv(ctx).await?;

    ctx.step(2, STEPS, "Installing Python");
    process::run(uv_cmd(&uv, ctx, &["python", "install", VOICE_PYTHON]), log).await?;

    ctx.step(3, STEPS, "Creating the voice service's Python environment");
    let venv = dir.join("venv");
    if venv.exists() {
        tokio::fs::remove_dir_all(&venv).await?;
    }
    let venv_arg = venv.to_string_lossy().to_string();
    process::run(uv_cmd(&uv, ctx, &["venv", "--python", VOICE_PYTHON, &venv_arg]), log).await?;
    let python = venv_python(&venv).to_string_lossy().to_string();

    ctx.step(4, STEPS, "Installing speech recognition and speech (about 300 MB)");
    let mut args = vec!["pip", "install", "--python", python.as_str()];
    args.extend(versions::VOICE_PACKAGES);
    process::run(uv_cmd(&uv, ctx, &args), log).await?;

    ctx.step(5, STEPS, "Downloading Kokoro's voices (about 350 MB)");
    let kokoro = models_dir(models_root).join("kokoro");
    for (url, name) in [
        (versions::KOKORO_MODEL_URL, "kokoro-v1.0.onnx"),
        (versions::KOKORO_VOICES_URL, "voices-v1.0.bin"),
    ] {
        let target = kokoro.join(name);
        if target.is_file() {
            log.push(format!("{name} already present"));
            continue;
        }
        download::download(&ctx.client, url, &target, ctx.download_progress()).await?;
    }

    ctx.step(6, STEPS, "Downloading the speech-recognition model (about 500 MB)");
    let server = write_server(&dir)?;
    let mut cmd = Command::new(&python);
    cmd.arg(&server).arg("--prefetch");
    env(&mut cmd, models_root);
    // Only the install may reach Hugging Face.
    cmd.env_remove("HF_HUB_OFFLINE");
    process::run(cmd, log).await?;

    Ok(Installed {
        version: VOICE_VERSION.to_string(),
        torch_variant: None,
    })
}

fn env(cmd: &mut Command, models_root: &Path) {
    cmd.env("OUTPOST_VOICE_PORT", VOICE_PORT.to_string())
        .env("OUTPOST_VOICE_MODELS", models_dir(models_root))
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("HF_HUB_DISABLE_TELEMETRY", "1")
        .env("HF_HUB_OFFLINE", "1");
}

/// The voice service on 127.0.0.1:8190, offline.
pub fn command(dir: &Path, models_root: &Path) -> anyhow::Result<Command> {
    let python = venv_python(&dir.join("venv"));
    if !python.is_file() {
        return Err(anyhow!("The voice service's files are missing. Reinstall it from Settings."));
    }
    // Always the server that came with this Outpost.
    let server = write_server(dir)?;
    let mut cmd = Command::new(python);
    cmd.arg(server).current_dir(dir);
    env(&mut cmd, models_root);
    Ok(cmd)
}

// ------------------------------------------------------------------ listening models

/// A speech-to-text model Outpost offers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ListeningModel {
    /// The faster-whisper name, e.g. `large-v3-turbo`.
    pub id: String,
    /// "Quick", "Balanced", "Most accurate".
    pub label: String,
    pub description: String,
    /// Download size, roughly.
    pub size_mb: u32,
    /// Downloaded on this machine.
    pub downloaded: bool,
}

/// (id, Hugging Face repo, label, description, size in MB)
const LISTENING: [(&str, &str, &str, &str, u32); 3] = [
    ("base", "Systran/faster-whisper-base", "Quick", "Fastest; fine for short, clear commands.", 145),
    ("small", "Systran/faster-whisper-small", "Balanced", "The default: quick, and good for everyday talking.", 485),
    (
        "large-v3-turbo",
        "mobiuslabsgmbh/faster-whisper-large-v3-turbo",
        "Most accurate",
        "Whisper large-v3-turbo: best with accents, names and noisy rooms; a little slower to reply.",
        1620,
    ),
];

/// The default listening model (what Voice installs).
pub const DEFAULT_LISTENING: &str = "small";

/// Where faster-whisper keeps a model (Hugging Face's cache layout).
fn listening_dir(models_root: &Path, repo: &str) -> PathBuf {
    models_dir(models_root).join("whisper").join(format!("models--{}", repo.replace('/', "--")))
}

fn repo_of(id: &str) -> Option<&'static str> {
    LISTENING.iter().find(|l| l.0 == id).map(|l| l.1)
}

/// Is this listening model downloaded (its weights are in a snapshot)?
pub fn listening_downloaded(models_root: &Path, id: &str) -> bool {
    let Some(repo) = repo_of(id) else { return false };
    let snapshots = listening_dir(models_root, repo).join("snapshots");
    std::fs::read_dir(snapshots)
        .map(|it| it.flatten().any(|e| e.path().join("model.bin").exists()))
        .unwrap_or(false)
}

/// The listening models, with which are downloaded.
pub fn listening_models(models_root: &Path) -> Vec<ListeningModel> {
    LISTENING
        .iter()
        .map(|(id, _, label, description, size_mb)| ListeningModel {
            id: id.to_string(),
            label: label.to_string(),
            description: description.to_string(),
            size_mb: *size_mb,
            downloaded: listening_downloaded(models_root, id),
        })
        .collect()
}

pub fn is_listening_model(id: &str) -> bool {
    repo_of(id).is_some()
}

/// How much of a listening model is on disk so far (for a progress bar), as a fraction.
pub fn listening_progress(models_root: &Path, id: &str) -> Option<f32> {
    let (_, repo, _, _, size_mb) = LISTENING.iter().find(|l| l.0 == id)?;
    fn size(p: &Path) -> u64 {
        std::fs::read_dir(p)
            .map(|it| {
                it.flatten()
                    .map(|e| match e.metadata() {
                        Ok(m) if m.is_dir() => size(&e.path()),
                        Ok(m) => m.len(),
                        Err(_) => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    // Only the blobs: the snapshot folder holds links to them.
    let have = size(&listening_dir(models_root, repo).join("blobs"));
    Some((have as f32 / (*size_mb as f32 * 1_000_000.0)).min(0.99))
}

/// Download a listening model with the voice service's own Python (the only time it goes
/// online after installing).
pub(crate) async fn fetch_listening(dir: &Path, models_root: &Path, id: &str, log: &crate::process::Log) -> anyhow::Result<()> {
    let python = venv_python(&dir.join("venv"));
    if !python.is_file() {
        return Err(anyhow!("Voice isn't installed. Install it in Settings › Engines first."));
    }
    let server = write_server(dir)?;
    let mut cmd = Command::new(python);
    cmd.arg(server).arg("--prefetch-stt").arg(id).current_dir(dir);
    env(&mut cmd, models_root);
    cmd.env_remove("HF_HUB_OFFLINE");
    process::run(cmd, log).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listening_model_counts_once_its_weights_are_there() {
        let root = std::env::temp_dir().join(format!("outpost-listening-{}", std::process::id()));
        let snap = listening_dir(&root, "mobiuslabsgmbh/faster-whisper-large-v3-turbo").join("snapshots").join("abc");
        std::fs::create_dir_all(&snap).unwrap();
        assert!(!listening_downloaded(&root, "large-v3-turbo"), "half downloaded");
        std::fs::write(snap.join("model.bin"), b"x").unwrap();
        assert!(listening_downloaded(&root, "large-v3-turbo"));
        assert!(!listening_downloaded(&root, "small"));
        assert!(!listening_downloaded(&root, "nope"));
        let all = listening_models(&root);
        assert_eq!(all.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["base", "small", "large-v3-turbo"]);
        assert!(all[2].downloaded && !all[1].downloaded);
        let _ = std::fs::remove_dir_all(&root);
    }
}
