//! LoRA training with ai-toolkit (Ostris, MIT), run as a separate program.
//!
//! Outpost writes a config for the job, runs `run.py` from the ai-toolkit folder with
//! its own Python, reads the progress bar from its output, and finds the checkpoints
//! and sample pictures it leaves in the run folder:
//!
//! ```text
//! <run>/<NAME>/<NAME>_000000250.safetensors   a checkpoint every 250 steps
//! <run>/<NAME>/<NAME>.safetensors             the last one
//! <run>/<NAME>/samples/<time>__000000250_0.jpg  test pictures at each checkpoint
//! ```

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use utoipa::ToSchema;

use crate::process::{hide_window, kill_tree, Log};

/// The name ai-toolkit gives the run's folder and files.
pub const NAME: &str = "lora";
/// Steps between checkpoints (and test pictures).
pub const SAVE_EVERY: u32 = 250;
/// The longest a training may run.
pub const TIME_LIMIT: Duration = Duration::from_secs(10 * 60 * 60);

/// Where ai-toolkit is on a normal Outpost PC, unless set otherwise.
pub fn default_dir() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\dev\ai-toolkit")
    } else {
        directories_home().join("ai-toolkit")
    }
}

fn directories_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// ai-toolkit's own Python, inside its virtual environment.
pub fn python(toolkit: &Path) -> PathBuf {
    if cfg!(windows) {
        toolkit.join("venv").join("Scripts").join("python.exe")
    } else {
        toolkit.join("venv").join("bin").join("python")
    }
}

/// Is ai-toolkit installed in `toolkit`? Returns what's missing if not.
pub fn check_install(toolkit: &Path) -> Result<(), String> {
    if !toolkit.join("run.py").is_file() {
        return Err(format!(
            "ai-toolkit isn't in {}. Install it (see docs/phase4-test.md) or set its folder in Train.",
            toolkit.display()
        ));
    }
    if !python(toolkit).is_file() {
        return Err(format!(
            "ai-toolkit in {} has no venv. Run its install steps first.",
            toolkit.display()
        ));
    }
    Ok(())
}

/// What to train.
#[derive(Debug, Clone)]
pub struct Spec {
    pub trigger: String,
    /// Pictures with a `.txt` caption beside each.
    pub dataset_dir: PathBuf,
    /// The run's own folder; ai-toolkit makes `<run>/lora/` in it.
    pub run_dir: PathBuf,
    pub steps: u32,
    /// LoRA size: 16 is light, 32 the usual, 64 for lots of detail.
    pub rank: u32,
    pub learning_rate: f64,
    /// Test pictures drawn at each checkpoint.
    pub sample_prompts: Vec<String>,
}

/// Test prompts for a character: the same few scenes at every checkpoint, so they
/// can be compared side by side.
pub fn sample_prompts(trigger: &str) -> Vec<String> {
    vec![
        format!("{trigger}, a close-up portrait photo, soft window light, looking at the camera"),
        format!("{trigger}, standing on a city street in a red winter coat, snow falling, photo"),
        format!("{trigger}, sitting on the beach at sunset, summer clothes, smiling, photo"),
    ]
}

/// The ai-toolkit job, as JSON (it reads JSON and YAML alike). Mirrors the settings
/// that trained a good Z-Image Turbo character in the phase 4 spike.
pub fn config(spec: &Spec) -> Value {
    let samples: Vec<Value> = spec.sample_prompts.iter().map(|p| json!({ "prompt": p })).collect();
    json!({
        "job": "extension",
        "config": {
            "name": NAME,
            "process": [{
                "type": "diffusion_trainer",
                "training_folder": spec.run_dir,
                "device": "cuda",
                "trigger_word": spec.trigger,
                "performance_log_every": 10,
                "network": {
                    "type": "lora",
                    "linear": spec.rank,
                    "linear_alpha": spec.rank,
                    "conv": (spec.rank / 2).max(1),
                    "conv_alpha": (spec.rank / 2).max(1),
                    "network_kwargs": { "ignore_if_contains": [] }
                },
                "save": {
                    "dtype": "bf16",
                    "save_every": SAVE_EVERY,
                    // Keep every checkpoint so the best can be picked afterwards.
                    "max_step_saves_to_keep": (spec.steps / SAVE_EVERY + 1).max(4),
                    "save_format": "diffusers",
                    "push_to_hub": false
                },
                "datasets": [{
                    "folder_path": spec.dataset_dir,
                    "default_caption": "",
                    "caption_ext": "txt",
                    "caption_dropout_rate": 0.05,
                    // Encode each picture once; the VAE then leaves the card.
                    "cache_latents_to_disk": true,
                    "is_reg": false,
                    "network_weight": 1,
                    "resolution": [1024],
                    "controls": [],
                    "num_repeats": 1
                }],
                "train": {
                    "batch_size": 1,
                    "steps": spec.steps,
                    "gradient_accumulation": 1,
                    "train_unet": true,
                    "train_text_encoder": false,
                    "gradient_checkpointing": true,
                    "noise_scheduler": "flowmatch",
                    "optimizer": "adamw8bit",
                    "timestep_type": "weighted",
                    "content_or_style": "balanced",
                    "optimizer_params": { "weight_decay": 0.0001 },
                    "unload_text_encoder": false,
                    // Encode each caption (and the test prompts) once, then take the text
                    // model (about 4 GB) off the card: 1024 training then fits in 16 GB
                    // with room for Windows. Caption dropout is skipped when caching.
                    "cache_text_embeddings": true,
                    "lr": spec.learning_rate,
                    "ema_config": { "use_ema": false, "ema_decay": 0.99 },
                    "skip_first_sample": false,
                    "disable_sampling": false,
                    "dtype": "bf16",
                    "loss_type": "mse"
                },
                "logging": { "log_every": 1, "use_ui_logger": false },
                "model": {
                    "name_or_path": "Tongyi-MAI/Z-Image-Turbo",
                    "quantize": true,
                    "qtype": "qfloat8",
                    "quantize_te": true,
                    "qtype_te": "qfloat8",
                    "arch": "zimage:turbo",
                    "low_vram": true,
                    "model_kwargs": {},
                    "assistant_lora_path": "ostris/zimage_turbo_training_adapter/zimage_turbo_training_adapter_v2.safetensors"
                },
                "sample": {
                    "sampler": "flowmatch",
                    "sample_every": SAVE_EVERY,
                    "width": 1024,
                    "height": 1024,
                    "samples": samples,
                    "neg": "",
                    "seed": 42,
                    "walk_seed": true,
                    "guidance_scale": 1,
                    "sample_steps": 9
                }
            }]
        },
        "meta": { "name": NAME, "version": "1.0" }
    })
}

/// How far the training has got.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    /// "Loading the model", "Training", "Drawing test pictures", "Saving".
    pub phase: String,
    pub step: u32,
    pub steps: u32,
}

/// Read one line of ai-toolkit's output into `p`. Its progress bar looks like
/// `lora:  12%|#2        | 250/2000 [17:01<1:59:10,  4.08s/it, lr: ...]`.
pub fn read_line(line: &str, p: &mut Progress) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    if let Some(rest) = line.strip_prefix(&format!("{NAME}:")) {
        if let Some((step, steps)) = bar_counts(rest) {
            p.step = step;
            p.steps = steps;
            p.phase = "Training".into();
        }
        return;
    }
    let lower = line.to_lowercase();
    if lower.starts_with("generating samples") || lower.starts_with("generating images") {
        p.phase = "Drawing test pictures".into();
    } else if lower.starts_with("saving at step") || lower.starts_with("saved checkpoint") {
        p.phase = "Saving".into();
    } else if lower.contains("caching latents") || lower.contains("caching text") {
        p.phase = "Preparing the pictures".into();
    } else if lower.starts_with("loading") && p.step == 0 {
        p.phase = "Loading the model".into();
    } else if lower.contains("quantizing") && p.step == 0 {
        p.phase = "Loading the model".into();
    }
}

/// `250/2000` from a tqdm bar.
fn bar_counts(rest: &str) -> Option<(u32, u32)> {
    let after_bar = rest.rsplit('|').next()?;
    let counts = after_bar.trim().split_whitespace().next()?;
    let (a, b) = counts.split_once('/')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Why ai-toolkit stopped, from its last lines: the useful sentence, not the stack.
pub fn explain_failure(tail: &[String]) -> String {
    let joined = tail.join("\n");
    let lower = joined.to_lowercase();
    if lower.contains("out of memory") {
        return "The graphics card ran out of memory. Close other programs using it (games, other AI apps) and try again."
            .into();
    }
    if lower.contains("no space left") {
        return "The disk is full. Free some space and try again.".into();
    }
    if (lower.contains("401") && lower.contains("huggingface")) || lower.contains("gated repo") {
        return "ai-toolkit couldn't download the model from Hugging Face (it needs signing in there).".into();
    }
    let last = tail
        .iter()
        .rev()
        .map(|l| l.trim())
        .find(|l| l.contains("Error") || l.contains("error:") || l.contains("Exception"))
        .or_else(|| tail.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty()))
        .unwrap_or("no output");
    format!("Training stopped: {}", last.chars().take(400).collect::<String>())
}

/// How a run ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Cancelled,
    Failed(String),
}

/// Run ai-toolkit on a config file until it finishes, is cancelled or times out.
/// `on_progress` gets every change; `cancelled` is polled. Output goes to `log`.
pub async fn run(
    toolkit: &Path,
    config_file: &Path,
    log: &Log,
    cancelled: impl Fn() -> bool,
    mut on_progress: impl FnMut(&Progress),
) -> Outcome {
    if let Err(e) = check_install(toolkit) {
        return Outcome::Failed(e);
    }
    let mut cmd = Command::new(python(toolkit));
    cmd.arg("run.py")
        .arg(config_file)
        .current_dir(toolkit)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        // No coloured or interactive output.
        .env("NO_COLOR", "1")
        .env("TQDM_MININTERVAL", "1");
    hide_window(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(format!("Couldn't start ai-toolkit: {e}")),
    };

    // Both streams into one channel of lines. tqdm redraws its bar with \r, so split
    // on that as well as \n.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(512);
    for stream in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut stream = stream;
            let mut buf = [0u8; 8192];
            let mut pending: Vec<u8> = Vec::new();
            loop {
                let n = match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !pending.is_empty() {
                            let _ = tx.send(String::from_utf8_lossy(&pending).into_owned()).await;
                            pending.clear();
                        }
                    } else {
                        pending.push(b);
                    }
                }
            }
            if !pending.is_empty() {
                let _ = tx.send(String::from_utf8_lossy(&pending).into_owned()).await;
            }
        });
    }
    drop(tx);

    let started = std::time::Instant::now();
    let mut progress = Progress {
        phase: "Starting ai-toolkit".into(),
        steps: 0,
        step: 0,
    };
    on_progress(&progress);
    let mut tail: Vec<String> = Vec::new();
    let mut last_bar_logged = 0u32;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            line = rx.recv() => {
                let Some(line) = line else {
                    // Output closed: the program is ending.
                    break match child.wait().await {
                        Ok(s) if s.success() => Outcome::Done,
                        _ => Outcome::Failed(explain_failure(&tail)),
                    };
                };
                let before = progress.clone();
                read_line(&line, &mut progress);
                let is_bar = line.trim_start().starts_with(&format!("{NAME}:"));
                // Keep the log readable: the bar only every 25 steps.
                if !is_bar || progress.step >= last_bar_logged + 25 || progress.step == progress.steps {
                    log.push(line.trim_end());
                    if is_bar {
                        last_bar_logged = progress.step;
                    }
                }
                if !is_bar {
                    tail.push(line);
                    if tail.len() > 40 {
                        tail.remove(0);
                    }
                }
                if progress != before {
                    on_progress(&progress);
                }
            }
            _ = tick.tick() => {
                if cancelled() {
                    kill_tree(&mut child);
                    let _ = child.wait().await;
                    break Outcome::Cancelled;
                }
                if started.elapsed() > TIME_LIMIT {
                    kill_tree(&mut child);
                    let _ = child.wait().await;
                    break Outcome::Failed("Training ran for over 10 hours, so Outpost stopped it.".into());
                }
            }
        }
    }
}

/// One saved checkpoint and its test pictures.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct Checkpoint {
    pub step: u32,
    /// File name in the run folder.
    pub file: String,
    /// Test picture file names (in `samples/`), in prompt order.
    pub samples: Vec<String>,
}

/// The checkpoints a run has saved so far, oldest first, plus test pictures drawn
/// before training (step 0) with no file. `steps` names the final file's step.
pub fn checkpoints(run_dir: &Path, steps: u32) -> Vec<Checkpoint> {
    let out_dir = run_dir.join(NAME);
    let mut by_step: std::collections::BTreeMap<u32, Checkpoint> = Default::default();
    let prefix = format!("{NAME}_");
    for entry in std::fs::read_dir(&out_dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".safetensors") else { continue };
        let step = if stem == NAME {
            steps
        } else if let Some(n) = stem.strip_prefix(&prefix).and_then(|n| n.parse::<u32>().ok()) {
            n
        } else {
            continue;
        };
        by_step.insert(
            step,
            Checkpoint {
                step,
                file: name,
                samples: Vec::new(),
            },
        );
    }
    // Samples: <time>__<step 9 digits>_<index>.jpg
    let mut samples: Vec<(u32, u32, String)> = std::fs::read_dir(out_dir.join("samples"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let (_, rest) = name.split_once("__")?;
            let rest = rest.strip_suffix(".jpg").or_else(|| rest.strip_suffix(".png"))?;
            let (step, idx) = rest.split_once('_')?;
            Some((step.parse().ok()?, idx.parse().ok()?, name))
        })
        .collect();
    samples.sort();
    for (step, _, name) in samples {
        by_step
            .entry(step)
            .or_insert_with(|| Checkpoint {
                step,
                file: String::new(),
                samples: Vec::new(),
            })
            .samples
            .push(name);
    }
    by_step.into_values().collect()
}

/// A checkpoint file's full path, refusing anything that isn't one of the run's.
pub fn checkpoint_path(run_dir: &Path, file: &str) -> Option<PathBuf> {
    let ok = file.ends_with(".safetensors") && !file.contains(['/', '\\']) && !file.contains("..");
    ok.then(|| run_dir.join(NAME).join(file)).filter(|p| p.is_file())
}

/// A test picture's full path, likewise.
pub fn sample_path(run_dir: &Path, file: &str) -> Option<PathBuf> {
    let ok = (file.ends_with(".jpg") || file.ends_with(".png")) && !file.contains(['/', '\\']) && !file.contains("..");
    ok.then(|| run_dir.join(NAME).join("samples").join(file)).filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_progress_bar() {
        let mut p = Progress::default();
        read_line("Loading transformer", &mut p);
        assert_eq!(p.phase, "Loading the model");
        read_line(
            "lora:  12%|#2        | 250/2000 [17:01<1:59:10,  4.08s/it, lr: 1.0e-04 loss: 3.2e-01]",
            &mut p,
        );
        assert_eq!((p.step, p.steps, p.phase.as_str()), (250, 2000, "Training"));
        read_line("Generating Samples:  33%|###3      | 1/3 [00:12<00:25, 12.97s/it]", &mut p);
        assert_eq!(p.phase, "Drawing test pictures");
        assert_eq!(p.step, 250, "a sample bar isn't the training bar");
        read_line("lora:   0%|          | 0/2000 [00:00<?, ?it/s]", &mut p);
        assert_eq!(p.step, 0);
    }

    #[test]
    fn config_matches_the_spike() {
        let spec = Spec {
            trigger: "zk4t".into(),
            dataset_dir: PathBuf::from("/d"),
            run_dir: PathBuf::from("/r"),
            steps: 2000,
            rank: 32,
            learning_rate: 1e-4,
            sample_prompts: sample_prompts("zk4t"),
        };
        let c = config(&spec);
        let p = &c["config"]["process"][0];
        assert_eq!(p["trigger_word"], "zk4t");
        assert_eq!(p["network"]["linear"], 32);
        assert_eq!(p["network"]["conv"], 16);
        assert_eq!(p["save"]["max_step_saves_to_keep"], 9);
        assert_eq!(p["model"]["arch"], "zimage:turbo");
        assert_eq!(p["train"]["steps"], 2000);
        assert_eq!(p["train"]["cache_text_embeddings"], true, "keeps 1024 training inside 16 GB");
        assert!(p["sample"]["samples"][0]["prompt"].as_str().unwrap().starts_with("zk4t,"));
        assert_eq!(c["config"]["name"], NAME);
    }

    #[test]
    fn finds_checkpoints_and_their_pictures() {
        let run = std::env::temp_dir().join(format!("outpost-run-{}", std::process::id()));
        let out = run.join(NAME);
        std::fs::create_dir_all(out.join("samples")).unwrap();
        for f in ["lora_000000250.safetensors", "lora_000000500.safetensors", "lora.safetensors", "optimizer.pt"] {
            std::fs::write(out.join(f), b"x").unwrap();
        }
        for f in [
            "100__000000000_0.jpg",
            "101__000000000_1.jpg",
            "200__000000250_1.jpg",
            "199__000000250_0.jpg",
            "300__000000750_0.jpg",
        ] {
            std::fs::write(out.join("samples").join(f), b"x").unwrap();
        }
        let c = checkpoints(&run, 750);
        let steps: Vec<u32> = c.iter().map(|c| c.step).collect();
        assert_eq!(steps, vec![0, 250, 500, 750]);
        assert_eq!(c[0].file, "");
        assert_eq!(c[1].samples, vec!["199__000000250_0.jpg", "200__000000250_1.jpg"]);
        assert_eq!(c[3].file, "lora.safetensors");
        assert_eq!(c[3].samples.len(), 1);
        assert!(checkpoint_path(&run, "lora.safetensors").is_some());
        assert!(checkpoint_path(&run, "../lora.safetensors").is_none());
        assert!(sample_path(&run, "100__000000000_0.jpg").is_some());
        let _ = std::fs::remove_dir_all(&run);
    }

    #[test]
    fn failures_are_explained() {
        let tail = vec![
            "Traceback (most recent call last):".to_string(),
            "torch.OutOfMemoryError: CUDA out of memory. Tried to allocate 2.00 GiB".to_string(),
        ];
        assert!(explain_failure(&tail).contains("ran out of memory"));
        let tail = vec!["ValueError: bad folder".to_string()];
        assert_eq!(explain_failure(&tail), "Training stopped: ValueError: bad folder");
    }
}
