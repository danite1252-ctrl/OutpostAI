//! Outpost engine manager.
//!
//! Installs pinned versions of Ollama and ComfyUI into Outpost's own folder, starts
//! them as hidden child processes, watches their health and restarts nothing behind
//! the user's back. If an engine is already running on its usual port (e.g. the user
//! installed Ollama themselves), Outpost uses that one and marks it `external`.

mod archive;
mod comfyui;
mod download;
pub mod gpu;
mod manager;
mod ollama;
mod process;
pub mod types;
pub mod versions;

pub use manager::{EngineError, EngineManager};
pub use types::{EngineId, EngineInfo, EngineState, EnginesStatus, Progress};
