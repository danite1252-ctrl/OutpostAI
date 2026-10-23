//! Ollama: one portable archive from the official GitHub release.

use std::path::Path;

use anyhow::{anyhow, bail};
use tokio::process::Command;

use crate::manager::{Ctx, Installed};
use crate::versions::{self, exe, OLLAMA_CONTEXT_LENGTH, OLLAMA_PORT, OLLAMA_VERSION};
use crate::{archive, download};

pub async fn install(ctx: &Ctx) -> anyhow::Result<Installed> {
    let dir = ctx.slot.dir.clone();
    let asset = versions::ollama_asset();
    let file = dir.join("download").join(asset);

    ctx.step(1, 3, "Downloading Ollama");
    let hash = download::download(&ctx.client, &versions::ollama_url(), &file, ctx.download_progress()).await?;

    ctx.step(2, 3, "Checking the download");
    match download::fetch_text(&ctx.client, &versions::ollama_checksums_url()).await {
        Ok(listing) => match download::checksum_for(&listing, asset) {
            Some(expected) if expected == hash => ctx.slot.log.push("Checksum OK"),
            Some(_) => {
                let _ = tokio::fs::remove_file(&file).await;
                bail!("The Ollama download is damaged (checksum mismatch). Please try again.");
            }
            None => ctx.slot.log.push("No published checksum for this file; continuing"),
        },
        Err(e) => ctx
            .slot
            .log
            .push(format!("Could not fetch checksums ({e}); continuing")),
    }

    ctx.step(3, 3, "Unpacking");
    let app = dir.join("app");
    let (archive_path, app_dir) = (file.clone(), app.clone());
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if app_dir.exists() {
            std::fs::remove_dir_all(&app_dir)?;
        }
        archive::extract(&archive_path, &app_dir)
    })
    .await??;
    let _ = tokio::fs::remove_file(&file).await;
    if archive::find_file(&app, &exe("ollama")).is_none() {
        bail!("Unpacked Ollama but could not find {} inside it.", exe("ollama"));
    }
    Ok(Installed {
        version: OLLAMA_VERSION.to_string(),
        torch_variant: None,
    })
}

/// `ollama serve` on 127.0.0.1, keeping models in Outpost's models folder so a
/// reinstall never deletes them.
pub fn command(dir: &Path, models_root: &Path) -> anyhow::Result<Command> {
    let app = dir.join("app");
    let bin = archive::find_file(&app, &exe("ollama"))
        .ok_or_else(|| anyhow!("Ollama's files are missing. Reinstall it from Settings."))?;
    let models = models_root.join("ollama");
    std::fs::create_dir_all(&models)?;
    let mut cmd = Command::new(bin);
    cmd.arg("serve")
        .current_dir(&app)
        .env("OLLAMA_HOST", format!("127.0.0.1:{OLLAMA_PORT}"))
        .env("OLLAMA_MODELS", models)
        // A 64K window for agents, kept affordable: flash attention plus an 8-bit KV
        // cache roughly halves the memory the window needs.
        .env("OLLAMA_CONTEXT_LENGTH", OLLAMA_CONTEXT_LENGTH.to_string())
        .env("OLLAMA_FLASH_ATTENTION", "1")
        .env("OLLAMA_KV_CACHE_TYPE", "q8_0");
    Ok(cmd)
}
