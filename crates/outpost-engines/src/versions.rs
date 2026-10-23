//! Pinned engine versions. Bump these deliberately and test before release;
//! "latest" from upstream can break workflows without warning.

pub const OLLAMA_VERSION: &str = "v0.40.9";
pub const UV_VERSION: &str = "0.13.5";
pub const COMFYUI_VERSION: &str = "v0.42.5";
/// ComfyUI recommends Python 3.13 (3.12 if some custom nodes need it).
pub const PYTHON_VERSION: &str = "3.13";

pub const OLLAMA_PORT: u16 = 11434;

/// Context window (tokens) Outpost starts Ollama with. Agents such as Hermes need at
/// least 64K; Ollama's own default is much smaller. With flash attention and an 8-bit
/// KV cache this fits a 9B model on a 16 GB card comfortably.
pub const OLLAMA_CONTEXT_LENGTH: u32 = 65_536;
pub const COMFYUI_PORT: u16 = 8188;

/// PyTorch wheels for NVIDIA cards (ComfyUI: 20 series and newer need cu130+).
pub const TORCH_INDEX_NVIDIA: &str = "https://download.pytorch.org/whl/cu130";
/// CPU-only wheels when no supported GPU is found. Works, but very slow.
pub const TORCH_INDEX_CPU: &str = "https://download.pytorch.org/whl/cpu";

pub fn ollama_asset() -> &'static str {
    if cfg!(windows) {
        "ollama-windows-amd64.zip"
    } else {
        "ollama-linux-amd64.tar.zst"
    }
}

pub fn ollama_url() -> String {
    format!(
        "https://github.com/ollama/ollama/releases/download/{OLLAMA_VERSION}/{}",
        ollama_asset()
    )
}

pub fn ollama_checksums_url() -> String {
    format!("https://github.com/ollama/ollama/releases/download/{OLLAMA_VERSION}/sha256sum.txt")
}

pub fn uv_asset() -> &'static str {
    if cfg!(windows) {
        "uv-x86_64-pc-windows-msvc.zip"
    } else {
        "uv-x86_64-unknown-linux-gnu.tar.gz"
    }
}

pub fn uv_url() -> String {
    format!(
        "https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{}",
        uv_asset()
    )
}

pub fn comfyui_url() -> String {
    format!("https://github.com/comfyanonymous/ComfyUI/archive/refs/tags/{COMFYUI_VERSION}.zip")
}

pub fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}
