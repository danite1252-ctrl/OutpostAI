use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

/// The port Outpost listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 7420;
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone)]
pub struct Config {
    /// Where the database and per-account files live.
    pub data_dir: PathBuf,
    /// Address to listen on. 127.0.0.1 by default; 0.0.0.0 when network access is on.
    pub bind: SocketAddr,
    /// Ollama's HTTP address.
    pub ollama_url: String,
}

impl Config {
    /// Defaults for a normal install: `%LOCALAPPDATA%\Outpost` on Windows,
    /// `~/.local/share/Outpost` on Linux, listening on 127.0.0.1:7420.
    pub fn local_default() -> Self {
        Config {
            data_dir: default_data_dir(),
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT)),
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("outpost.db")
    }

    /// Per-account files such as chat images: `<data>/files/<account_id>/...`.
    pub fn files_dir(&self) -> PathBuf {
        self.data_dir.join("files")
    }
}

pub fn default_data_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.data_local_dir().join("Outpost"))
        .unwrap_or_else(|| PathBuf::from("outpost-data"))
}
