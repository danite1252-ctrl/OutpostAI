use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::gpu::Gpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EngineId {
    Ollama,
    Comfyui,
    Voice,
}

impl EngineId {
    pub const ALL: [EngineId; 3] = [EngineId::Ollama, EngineId::Comfyui, EngineId::Voice];

    pub fn as_str(self) -> &'static str {
        match self {
            EngineId::Ollama => "ollama",
            EngineId::Comfyui => "comfyui",
            EngineId::Voice => "voice",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            EngineId::Ollama => "Ollama",
            EngineId::Comfyui => "ComfyUI",
            EngineId::Voice => "Voice",
        }
    }

    pub fn port(self) -> u16 {
        match self {
            EngineId::Ollama => crate::versions::OLLAMA_PORT,
            EngineId::Comfyui => crate::versions::COMFYUI_PORT,
            EngineId::Voice => crate::versions::VOICE_PORT,
        }
    }

    pub fn base_url(self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }

    /// A cheap URL that answers 200 when the engine is up.
    pub fn health_url(self) -> String {
        match self {
            EngineId::Ollama => format!("{}/api/version", self.base_url()),
            EngineId::Comfyui => format!("{}/system_stats", self.base_url()),
            EngineId::Voice => format!("{}/health", self.base_url()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    /// Not installed by Outpost and nothing answers on its port.
    NotInstalled,
    /// Downloading or setting up.
    Installing,
    /// Installed by Outpost, not running.
    Stopped,
    /// Process started, waiting for it to answer.
    Starting,
    /// Installed by Outpost and answering.
    Running,
    /// Something else (e.g. the user's own Ollama) answers on the port; Outpost uses it.
    External,
    /// Install or start failed; see `error`.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Progress {
    /// What is happening, e.g. "Downloading Ollama".
    pub step: String,
    /// 1-based step number.
    pub step_index: u32,
    pub step_count: u32,
    /// 0–100 when known (downloads); absent for steps without a measurable size.
    pub percent: Option<f32>,
    /// Extra detail, e.g. "412 MB of 1.6 GB".
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EngineInfo {
    pub id: EngineId,
    pub name: String,
    pub state: EngineState,
    /// True when Outpost installed and runs this engine itself.
    pub managed: bool,
    /// Installed version (managed engines only).
    pub version: Option<String>,
    /// Version Outpost would install.
    pub pinned_version: String,
    pub url: String,
    /// Rough download and disk size, shown before installing.
    pub size_hint: String,
    pub progress: Option<Progress>,
    pub error: Option<String>,
    /// Last lines of the engine's or installer's output.
    pub log_tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnginesStatus {
    pub gpus: Vec<Gpu>,
    /// Which PyTorch build ComfyUI gets: `cu130` (NVIDIA) or `cpu`.
    pub torch_variant: String,
    pub engines: Vec<EngineInfo>,
}
