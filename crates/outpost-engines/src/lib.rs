//! Outpost engine manager.
//!
//! Installs pinned versions of Ollama and ComfyUI into Outpost's own folder, starts
//! them as hidden child processes, watches their health and restarts nothing behind
//! the user's back. If an engine is already running on its usual port (e.g. the user
//! installed Ollama themselves), Outpost uses that one and marks it `external`.

mod ai_toolkit;
mod archive;
pub mod assets;
pub mod catalog;
pub mod comfy;
mod comfyui;
mod voice;
mod download;
pub mod gpu;
pub mod hardware;
mod manager;
pub mod models;
mod ollama;
pub mod packs;
mod process;
pub mod rate;
pub mod trainer;
pub mod types;
pub mod versions;

pub use manager::{EngineError, EngineManager, TrainerInstall};
pub use process::Log;
pub use types::{EngineId, EngineInfo, EngineState, EnginesStatus, Progress};
