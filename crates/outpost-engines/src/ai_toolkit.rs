//! Installing ai-toolkit (for LoRA training) the way ComfyUI is installed: a pinned source
//! download, in its own Python environment that uv builds. Nothing touches a system Python,
//! and git isn't needed (requirements fetched from git are fetched as archives instead).
//!
//! It lands in `<data>/engines/ai-toolkit/app` (with `run.py` and `venv`), which is where
//! [`crate::trainer`] runs it from. Base models download into the Hugging Face cache on the
//! first training, as before.

use crate::comfyui::{ensure_uv, uv_cmd};
use crate::manager::{Ctx, Installed};
use crate::versions::{self, AI_TOOLKIT_PYTHON, AI_TOOLKIT_TORCH, AI_TOOLKIT_VERSION};
use crate::{archive, download, process};

const STEPS: u32 = 6;

pub(crate) async fn install(ctx: &Ctx) -> anyhow::Result<Installed> {
    let dir = ctx.slot.dir.clone();
    let log = &ctx.slot.log;

    ctx.step(1, STEPS, "Downloading uv (Python manager)");
    let uv = ensure_uv(ctx).await?;

    ctx.step(2, STEPS, "Installing Python");
    process::run(uv_cmd(&uv, ctx, &["python", "install", AI_TOOLKIT_PYTHON]), log).await?;

    ctx.step(3, STEPS, "Downloading ai-toolkit");
    let zip = dir.join("download").join(format!("ai-toolkit-{AI_TOOLKIT_VERSION}.zip"));
    download::download(&ctx.client, &versions::ai_toolkit_url(), &zip, ctx.download_progress()).await?;
    let (zip_path, staging, app) = (zip.clone(), dir.join("staging"), dir.join("app"));
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        if app.exists() {
            std::fs::remove_dir_all(&app)?;
        }
        archive::extract(&zip_path, &staging)?;
        archive::unpack_single_folder(&staging, &app)
    })
    .await??;
    let _ = tokio::fs::remove_file(&zip).await;
    let app = dir.join("app");

    ctx.step(4, STEPS, "Creating ai-toolkit's Python environment");
    let venv = app.join("venv");
    let venv_arg = venv.to_string_lossy().to_string();
    process::run(uv_cmd(&uv, ctx, &["venv", "--python", AI_TOOLKIT_PYTHON, &venv_arg]), log).await?;
    let python = crate::comfyui::venv_python(&venv).to_string_lossy().to_string();

    ctx.step(5, STEPS, "Installing PyTorch for NVIDIA (about 3 GB)");
    let mut args = vec!["pip", "install", "--python", python.as_str()];
    args.extend(AI_TOOLKIT_TORCH);
    args.extend(["--index-url", versions::TORCH_INDEX_NVIDIA]);
    process::run(uv_cmd(&uv, ctx, &args), log).await?;

    ctx.step(6, STEPS, "Installing ai-toolkit's requirements (a few GB)");
    let mut lines = Vec::new();
    for file in ["requirements_base.txt", "requirements.txt"] {
        if let Ok(text) = tokio::fs::read_to_string(app.join(file)).await {
            lines.push(without_git(&text));
        }
    }
    let reqs = app.join("outpost-requirements.txt");
    tokio::fs::write(&reqs, lines.join("\n")).await?;
    let reqs = reqs.to_string_lossy().to_string();
    process::run(uv_cmd(&uv, ctx, &["pip", "install", "--python", &python, "-r", &reqs]), log).await?;

    Ok(Installed { version: AI_TOOLKIT_VERSION.to_string(), torch_variant: Some("cu130".into()) })
}

/// Requirements with `git+https://github.com/<owner>/<repo>.git@<ref>` turned into that
/// commit's archive (so git isn't needed), and `-r` includes left out (read separately).
pub(crate) fn without_git(text: &str) -> String {
    text.lines()
        .filter_map(|line| {
            let l = line.trim();
            if l.starts_with("-r ") || l.starts_with("--requirement") {
                return None;
            }
            let Some(rest) = l.strip_prefix("git+https://github.com/") else {
                return Some(line.to_string());
            };
            let (repo, git_ref) = rest.split_once('@')?;
            let repo = repo.trim_end_matches(".git");
            let name = repo.rsplit('/').next()?;
            let git_ref = git_ref.split(['#', ' ']).next()?;
            Some(format!("{name} @ https://github.com/{repo}/archive/{git_ref}.zip"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn git_requirements_become_archives() {
        let text = "torchao==0.10.0\ngit+https://github.com/huggingface/diffusers.git@c9438378\n#pip install git+https://x\n-r requirements_base.txt\nscipy==1.12.0";
        assert_eq!(
            super::without_git(text),
            "torchao==0.10.0\ndiffusers @ https://github.com/huggingface/diffusers/archive/c9438378.zip\n#pip install git+https://x\nscipy==1.12.0"
        );
    }
}
