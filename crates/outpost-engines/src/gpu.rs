//! Graphics card detection. Phase 0.5 knows NVIDIA (via nvidia-smi, which ships with
//! the driver); AMD and Intel detection arrive with the hardware probe in phase 1.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Gpu {
    /// `nvidia` for now.
    pub vendor: String,
    pub name: String,
    /// Total video memory in MiB.
    pub vram_mb: u64,
}

/// Find NVIDIA cards. Returns an empty list when nvidia-smi is missing or fails.
pub async fn detect() -> Vec<Gpu> {
    let mut cmd = tokio::process::Command::new("nvidia-smi");
    cmd.args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    crate::process::hide_window(&mut cmd);
    let output = match tokio::time::timeout(std::time::Duration::from_secs(10), cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => out,
        _ => return Vec::new(),
    };
    parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout))
}

/// Video memory in use right now on the first NVIDIA card, in MiB. `None` when it
/// can't be told (no NVIDIA card, or nvidia-smi isn't there).
pub async fn used_mb() -> Option<u64> {
    // nvidia-smi isn't always on PATH; the driver puts it in System32.
    for program in ["nvidia-smi", r"C:\Windows\System32\nvidia-smi.exe"] {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        crate::process::hide_window(&mut cmd);
        if let Ok(Ok(out)) = tokio::time::timeout(std::time::Duration::from_secs(10), cmd.output()).await {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).lines().next()?.trim().parse().ok();
            }
        }
    }
    None
}

fn parse_nvidia_smi(text: &str) -> Vec<Gpu> {
    text.lines()
        .filter_map(|line| {
            let (name, mem) = line.rsplit_once(',')?;
            Some(Gpu {
                vendor: "nvidia".into(),
                name: name.trim().to_string(),
                vram_mb: mem.trim().parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvidia_smi_csv() {
        let gpus = parse_nvidia_smi("NVIDIA GeForce RTX 4070 Ti SUPER, 16376\nNVIDIA RTX A2000, 6138\n\n");
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 4070 Ti SUPER");
        assert_eq!(gpus[0].vram_mb, 16376);
        assert!(parse_nvidia_smi("garbage").is_empty());
    }
}
