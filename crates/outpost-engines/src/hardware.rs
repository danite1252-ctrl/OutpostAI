//! What this PC can run: graphics cards, memory, free disk, and a simple tier.

use std::path::Path;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::gpu::Gpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// No supported GPU: models run on the CPU.
    Cpu,
    /// Up to 8 GB of video memory.
    Starter,
    /// Up to 12 GB.
    Standard,
    /// Up to 16 GB.
    Performance,
    /// More than 16 GB.
    Workstation,
}

impl Tier {
    pub fn from_vram_mb(vram_mb: u64) -> Tier {
        match vram_mb {
            0 => Tier::Cpu,
            v if v <= 8 * 1024 => Tier::Starter,
            v if v <= 12 * 1024 => Tier::Standard,
            v if v <= 16 * 1024 => Tier::Performance,
            _ => Tier::Workstation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Hardware {
    pub gpus: Vec<Gpu>,
    /// Video memory of the largest card, in MiB (0 without a supported GPU).
    pub vram_mb: u64,
    /// Total system memory in MiB.
    pub ram_mb: u64,
    /// Logical CPU cores.
    pub cpu_threads: u32,
    /// Free space on the drive that holds Outpost's data, in MiB.
    pub disk_free_mb: u64,
    pub tier: Tier,
}

/// Measure the machine. `gpus` comes from [`crate::gpu::detect`] (cached by the engine manager).
pub fn measure(gpus: Vec<Gpu>, data_dir: &Path) -> Hardware {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let ram_mb = sys.total_memory() / 1_048_576;
    let vram_mb = gpus.iter().map(|g| g.vram_mb).max().unwrap_or(0);
    // The data folder may not exist yet; measure its nearest existing parent.
    let probe = data_dir.ancestors().find(|p| p.exists()).unwrap_or(data_dir);
    let disk_free_mb = fs2::available_space(probe).map(|b| b / 1_048_576).unwrap_or(0);
    Hardware {
        tier: Tier::from_vram_mb(vram_mb),
        gpus,
        vram_mb,
        ram_mb,
        cpu_threads: std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1),
        disk_free_mb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_follow_vram() {
        assert_eq!(Tier::from_vram_mb(0), Tier::Cpu);
        assert_eq!(Tier::from_vram_mb(8188), Tier::Starter);
        assert_eq!(Tier::from_vram_mb(12282), Tier::Standard);
        assert_eq!(Tier::from_vram_mb(16376), Tier::Performance);
        assert_eq!(Tier::from_vram_mb(24564), Tier::Workstation);
    }

    #[test]
    fn measures_this_machine() {
        let hw = measure(Vec::new(), &std::env::temp_dir());
        assert!(hw.ram_mb > 0);
        assert!(hw.cpu_threads >= 1);
        assert_eq!(hw.tier, Tier::Cpu);
    }
}
