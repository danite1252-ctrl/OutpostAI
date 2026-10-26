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

/// The voice service (speech to text and text to speech), Outpost's own script.
pub const VOICE_PORT: u16 = 8190;
/// Its version: bump with any change to `voice/server.py` or the packages below.
pub const VOICE_VERSION: &str = "1.0.0";
/// Python for the voice service (kokoro-onnx supports 3.10 to 3.13).
pub const VOICE_PYTHON: &str = "3.12";
/// Speech recognition (MIT) and speech (MIT, on ONNX Runtime).
pub const VOICE_PACKAGES: [&str; 2] = ["faster-whisper==1.2.1", "kokoro-onnx==0.6.1"];
/// Kokoro's model and voices (54), from the kokoro-onnx project's model release.
pub const KOKORO_MODEL_URL: &str =
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.1/kokoro-v1.0.onnx";
pub const KOKORO_VOICES_URL: &str =
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.1/voices-v1.0.bin";

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
