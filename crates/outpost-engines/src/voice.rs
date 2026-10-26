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
