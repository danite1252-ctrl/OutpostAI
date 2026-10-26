//! ComfyUI: the pinned source release, run in its own Python environment that
//! uv builds. Nothing touches a system Python.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};
use tokio::process::Command;

use crate::manager::{Ctx, Installed};
use crate::versions::{self, exe, COMFYUI_PORT, COMFYUI_VERSION, PYTHON_VERSION};
use crate::{archive, assets, download, process};

const STEPS: u32 = 6;

pub async fn install(ctx: &Ctx) -> anyhow::Result<Installed> {
    let dir = ctx.slot.dir.clone();
    let log = &ctx.slot.log;

    ctx.step(1, STEPS, "Downloading uv (Python manager)");
    let uv = ensure_uv(ctx).await?;

    ctx.step(2, STEPS, "Installing Python");
    process::run(uv_cmd(&uv, ctx, &["python", "install", PYTHON_VERSION]), log).await?;

    ctx.step(3, STEPS, "Downloading ComfyUI");
    let zip = dir.join("download").join(format!("ComfyUI-{COMFYUI_VERSION}.zip"));
    download::download(&ctx.client, &versions::comfyui_url(), &zip, ctx.download_progress()).await?;
    let (zip_path, staging, app) = (zip.clone(), dir.join("staging"), dir.join("app"));
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        archive::extract(&zip_path, &staging)?;
        archive::unpack_single_folder(&staging, &app)
    })
    .await??;
    let _ = tokio::fs::remove_file(&zip).await;

    ctx.step(4, STEPS, "Creating ComfyUI's Python environment");
    let venv = dir.join("venv");
    if venv.exists() {
        tokio::fs::remove_dir_all(&venv).await?;
    }
    let venv_arg = venv.to_string_lossy().to_string();
    process::run(uv_cmd(&uv, ctx, &["venv", "--python", PYTHON_VERSION, &venv_arg]), log).await?;
    let python = venv_python(&venv).to_string_lossy().to_string();

    let variant = ctx.torch_variant();
    let (index, label) = if variant == "cpu" {
        (
            versions::TORCH_INDEX_CPU,
            "Installing PyTorch (no NVIDIA card found: CPU only, slow)",
        )
    } else {
        (
            versions::TORCH_INDEX_NVIDIA,
            "Installing PyTorch for NVIDIA (about 3 GB)",
        )
    };
    ctx.step(5, STEPS, label);
    process::run(
        uv_cmd(
            &uv,
            ctx,
            &[
                "pip",
                "install",
                "--python",
                &python,
                "torch",
                "torchvision",
                "torchaudio",
                "--index-url",
                index,
            ],
        ),
        log,
    )
    .await?;

    ctx.step(6, STEPS, "Installing ComfyUI's requirements");
    let requirements = dir.join("app").join("requirements.txt").to_string_lossy().to_string();
    process::run(
        uv_cmd(&uv, ctx, &["pip", "install", "--python", &python, "-r", &requirements]),
        log,
    )
    .await?;

    Ok(Installed {
        version: COMFYUI_VERSION.to_string(),
        torch_variant: Some(variant.to_string()),
    })
}

/// ComfyUI on 127.0.0.1:8188, no browser tab, reading models from Outpost's
/// models folder (`<data>/models/comfyui`) so a reinstall never deletes them.
pub fn command(dir: &Path, models_root: &Path) -> anyhow::Result<Command> {
    let app = dir.join("app");
    let python = venv_python(&dir.join("venv"));
    if !python.is_file() || !app.join("main.py").is_file() {
        return Err(anyhow!("ComfyUI's files are missing. Reinstall it from Settings."));
    }
    let models = models_root.join("comfyui");
    for folder in assets::FOLDERS {
        std::fs::create_dir_all(models.join(folder))?;
    }
    let paths_file = dir.join("outpost_model_paths.yaml");
    std::fs::write(&paths_file, assets::extra_model_paths_yaml(&models))?;
    let mut cmd = Command::new(python);
    cmd.args([
        "main.py",
        "--listen",
        "127.0.0.1",
        "--port",
        &COMFYUI_PORT.to_string(),
        "--disable-auto-launch",
    ])
    .arg("--extra-model-paths-config")
    .arg(&paths_file)
    .current_dir(&app)
    .env("PYTHONUNBUFFERED", "1");
    Ok(cmd)
}

pub(crate) fn venv_python(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// uv with every cache and Python install kept inside Outpost's engines folder.
pub(crate) fn uv_cmd(uv: &Path, ctx: &Ctx, args: &[&str]) -> Command {
    let uv_root = ctx.engines_root.join("uv");
    let mut cmd = Command::new(uv);
    cmd.args(args)
        .env("UV_PYTHON_INSTALL_DIR", uv_root.join("python"))
        .env("UV_CACHE_DIR", uv_root.join("cache"))
        .env("UV_PYTHON_PREFERENCE", "only-managed")
        .env("UV_LINK_MODE", "copy")
        .env("UV_NO_PROGRESS", "1")
        .env("UV_NO_CONFIG", "1");
    cmd
}

/// Download uv once into `<engines>/uv/bin`. Shared by everything that needs Python.
pub(crate) async fn ensure_uv(ctx: &Ctx) -> anyhow::Result<PathBuf> {
    // ComfyUI and the voice service may install at the same time; fetch uv once.
    static UV_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _one_at_a_time = UV_LOCK.get_or_init(|| tokio::sync::Mutex::new(())).lock().await;
    let uv_root = ctx.engines_root.join("uv");
    let bin = uv_root.join("bin").join(exe("uv"));
    if bin.is_file() {
        ctx.slot.log.push("uv already present");
        return Ok(bin);
    }
    let asset = versions::uv_asset();
    let file = uv_root.join("download").join(asset);
    let hash = download::download(&ctx.client, &versions::uv_url(), &file, ctx.download_progress()).await?;
    let sums = download::fetch_text(&ctx.client, &format!("{}.sha256", versions::uv_url())).await;
    match sums.ok().and_then(|s| download::checksum_for(&s, asset)) {
        Some(expected) if expected != hash => {
            let _ = tokio::fs::remove_file(&file).await;
            bail!("The uv download is damaged (checksum mismatch). Please try again.");
        }
        Some(_) => ctx.slot.log.push("Checksum OK"),
        None => ctx.slot.log.push("No checksum available for uv; continuing"),
    }
    let (archive_path, unpacked, target) = (file.clone(), uv_root.join("unpacked"), bin.clone());
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if unpacked.exists() {
            std::fs::remove_dir_all(&unpacked)?;
        }
        archive::extract(&archive_path, &unpacked)?;
        let found = archive::find_file(&unpacked, &exe("uv")).ok_or_else(|| anyhow!("uv was not in its download"))?;
        std::fs::create_dir_all(target.parent().expect("bin dir"))?;
        std::fs::copy(found, &target)?;
        let _ = std::fs::remove_dir_all(&unpacked);
        Ok(())
    })
    .await??;
    let _ = tokio::fs::remove_file(&file).await;
    Ok(bin)
}
