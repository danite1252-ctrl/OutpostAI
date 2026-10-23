//! Curated chat and embedding models for Ollama, with badges and a "does it fit"
//! score for this PC. Sizes are Ollama's listed download sizes (October 2026);
//! after a download the real size from Ollama is shown instead. Unfiltered Qwen
//! models are listed first, as the owner prefers them.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::hardware::Hardware;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Badge {
    /// Reads images and screenshots; can caption training datasets.
    Vision,
    /// Reliable function calling: usable by agents and apps like Hermes.
    Tools,
    /// Shows its reasoning before answering.
    Thinking,
    /// Abliterated or no-refusal tune.
    Uncensored,
    /// Tuned for programming.
    Code,
    /// 128K tokens of context or more.
    LongContext,
    /// Turns text into vectors for memory and document search.
    Embedding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Listed first: the owner prefers unfiltered Qwen models.
    Uncensored,
    General,
    Coding,
    Embedding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Fits in video memory: fast.
    Fits,
    /// Spills into system memory: works, slower.
    Partial,
    /// Needs more memory than this PC has.
    TooLarge,
}

pub struct Entry {
    /// Name to pull from Ollama.
    pub model: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub size_gb: f32,
    pub context_k: u32,
    pub badges: &'static [Badge],
    pub license: Option<&'static str>,
    /// Higher is better within a category; used to pick recommendations.
    pub rank: u32,
    pub description: &'static str,
    pub page: &'static str,
}

use Badge::*;

pub const CATALOG: &[Entry] = &[
    Entry {
        model: "ornith-1.5:9b",
        name: "Ornith 1.5 9B",
        category: Category::General,
        size_gb: 6.6,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: Some("MIT"),
        rank: 78,
        description: "Strong all-rounder that also reads images. Good default for 8–16 GB cards.",
        page: "https://ollama.com/library/ornith-1.5",
    },
    Entry {
        model: "ornith-1.5:35b",
        name: "Ornith 1.5 35B",
        category: Category::General,
        size_gb: 23.0,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: Some("MIT"),
        rank: 95,
        description: "The bigger Ornith. Best quality here if you have 24 GB+ of video memory.",
        page: "https://ollama.com/library/ornith-1.5",
    },
    Entry {
        model: "qwen3.8:27b",
        name: "Qwen 3.8 27B",
        category: Category::General,
        size_gb: 18.0,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 92,
        description: "Latest Qwen. Excellent reasoning and tool use.",
        page: "https://ollama.com/library/qwen3.8",
    },
    Entry {
        model: "qwen3.6:27b",
        name: "Qwen 3.6 27B",
        category: Category::General,
        size_gb: 18.0,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 88,
        description: "Capable 27B generalist with vision.",
        page: "https://ollama.com/library/qwen3.6",
    },
    Entry {
        model: "gemma4:26b",
        name: "Gemma 4 26B",
        category: Category::General,
        size_gb: 16.0,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 85,
        description: "Google's Gemma 4, large size. Good writing and vision.",
        page: "https://ollama.com/library/gemma4",
    },
    Entry {
        model: "gpt-oss:20b",
        name: "gpt-oss 20B",
        category: Category::General,
        size_gb: 14.0,
        context_k: 128,
        badges: &[Tools, Thinking, LongContext],
        license: Some("Apache-2.0"),
        rank: 80,
        description: "OpenAI's open-weight model. Text only, strong at tools and reasoning.",
        page: "https://ollama.com/library/gpt-oss",
    },
    Entry {
        model: "gemma4:12b",
        name: "Gemma 4 12B",
        category: Category::General,
        size_gb: 7.7,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 74,
        description: "Mid-size Gemma 4 with vision. Fits 12 GB cards.",
        page: "https://ollama.com/library/gemma4",
    },
    Entry {
        model: "qwen3.5:9b",
        name: "Qwen 3.5 9B",
        category: Category::General,
        size_gb: 6.6,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 72,
        description: "Fast, capable 9B with vision.",
        page: "https://ollama.com/library/qwen3.5",
    },
    Entry {
        model: "gemma4:e4b",
        name: "Gemma 4 E4B",
        category: Category::General,
        size_gb: 6.6,
        context_k: 128,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 60,
        description: "Efficient small Gemma 4 for laptops and 8 GB cards.",
        page: "https://ollama.com/library/gemma4",
    },
    Entry {
        model: "qwen3.5:4b",
        name: "Qwen 3.5 4B",
        category: Category::General,
        size_gb: 3.3,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 55,
        description: "Small and quick. Runs on almost anything, even CPU only.",
        page: "https://ollama.com/library/qwen3.5",
    },
    Entry {
        model: "huihui_ai/Qwen3.8-abliterated:27b",
        name: "Qwen 3.8 27B Abliterated",
        category: Category::Uncensored,
        size_gb: 18.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 90,
        description: "Qwen 3.8 with refusals removed. Community build by huihui_ai.",
        page: "https://ollama.com/huihui_ai/Qwen3.8-abliterated",
    },
    Entry {
        model: "huihui_ai/qwen3.5-abliterated:35b",
        name: "Qwen 3.5 35B-A3B Abliterated",
        category: Category::Uncensored,
        size_gb: 24.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 88,
        description: "Mixture-of-experts Qwen 3.5 with refusals removed. Fast for its size on 24 GB cards.",
        page: "https://ollama.com/huihui_ai/qwen3.5-abliterated",
    },
    Entry {
        model: "huihui_ai/qwen3.5-abliterated:27b",
        name: "Qwen 3.5 27B Abliterated",
        category: Category::Uncensored,
        size_gb: 17.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 84,
        description: "Large uncensored Qwen 3.5 with vision. Best on 20 GB+ cards.",
        page: "https://ollama.com/huihui_ai/qwen3.5-abliterated",
    },
    Entry {
        model: "huihui_ai/qwen3.5-abliterated:9b",
        name: "Qwen 3.5 9B Abliterated",
        category: Category::Uncensored,
        size_gb: 6.6,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 70,
        description: "Uncensored 9B with vision. Community build by huihui_ai.",
        page: "https://ollama.com/huihui_ai/qwen3.5-abliterated",
    },
    Entry {
        model: "huihui_ai/qwen3.5-abliterated:4b",
        name: "Qwen 3.5 4B Abliterated",
        category: Category::Uncensored,
        size_gb: 3.3,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 50,
        description: "Small uncensored model for 8 GB cards or CPU.",
        page: "https://ollama.com/huihui_ai/qwen3.5-abliterated",
    },
    Entry {
        model: "huihui_ai/qwen3.5-abliterated:2b",
        name: "Qwen 3.5 2B Abliterated",
        category: Category::Uncensored,
        size_gb: 1.9,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 35,
        description: "Tiny uncensored Qwen for CPU-only machines.",
        page: "https://ollama.com/huihui_ai/qwen3.5-abliterated",
    },
    Entry {
        model: "qwen3.6:27b-coding",
        name: "Qwen 3.6 27B Coding",
        category: Category::Coding,
        size_gb: 18.0,
        context_k: 256,
        badges: &[Code, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 90,
        description: "Qwen 3.6 tuned for code. For the coding agent later on.",
        page: "https://ollama.com/library/qwen3.6",
    },
    Entry {
        model: "qwen3-embedding:4b",
        name: "Qwen3 Embedding 4B",
        category: Category::Embedding,
        size_gb: 2.5,
        context_k: 40,
        badges: &[Embedding],
        license: None,
        rank: 70,
        description: "Powers memory and document search. Not a chat model.",
        page: "https://ollama.com/library/qwen3-embedding",
    },
    Entry {
        model: "qwen3-embedding:0.6b",
        name: "Qwen3 Embedding 0.6B",
        category: Category::Embedding,
        size_gb: 0.64,
        context_k: 32,
        badges: &[Embedding],
        license: None,
        rank: 60,
        description: "Tiny embedding model for memory and search on small machines.",
        page: "https://ollama.com/library/qwen3-embedding",
    },
];

/// A catalogue entry as the API returns it, scored for this PC.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CatalogEntry {
    pub model: String,
    pub name: String,
    pub category: Category,
    pub size_gb: f32,
    pub context_k: u32,
    pub badges: Vec<Badge>,
    /// Licence when known; otherwise check the model page.
    pub license: Option<String>,
    pub description: String,
    pub page: String,
    pub fit: Fit,
    /// Best pick in its category for this PC.
    pub recommended: bool,
}

/// Memory a model needs: its weights plus working space for an 8K-token chat.
fn needed_gb(size_gb: f32) -> f32 {
    size_gb * 1.15 + 0.6
}

pub fn fit(size_gb: f32, hw: &Hardware) -> Fit {
    let need = needed_gb(size_gb);
    let vram = hw.vram_mb as f32 / 1024.0;
    let ram = hw.ram_mb as f32 / 1024.0;
    if vram > 0.0 && need <= vram * 0.95 {
        Fit::Fits
    } else if need <= vram + ram * 0.5 {
        Fit::Partial
    } else {
        Fit::TooLarge
    }
}

/// The whole catalogue scored for `hw`, best first within each category.
pub fn build(hw: &Hardware) -> Vec<CatalogEntry> {
    let mut entries: Vec<CatalogEntry> = CATALOG
        .iter()
        .map(|e| CatalogEntry {
            model: e.model.into(),
            name: e.name.into(),
            category: e.category,
            size_gb: e.size_gb,
            context_k: e.context_k,
            badges: e.badges.to_vec(),
            license: e.license.map(Into::into),
            description: e.description.into(),
            page: e.page.into(),
            fit: fit(e.size_gb, hw),
            recommended: false,
        })
        .collect();

    for category in [
        Category::Uncensored,
        Category::General,
        Category::Coding,
        Category::Embedding,
    ] {
        let best = |want: Fit| {
            CATALOG
                .iter()
                .enumerate()
                .filter(|(_, e)| e.category == category && fit(e.size_gb, hw) == want)
                .max_by_key(|(_, e)| e.rank)
                .map(|(i, _)| i)
        };
        // Prefer something that fits fully; on small machines, the best that runs at all.
        if let Some(i) = best(Fit::Fits).or_else(|| best(Fit::Partial)) {
            entries[i].recommended = true;
        }
    }

    let rank = |m: &str| CATALOG.iter().find(|e| e.model == m).map(|e| e.rank).unwrap_or(0);
    entries.sort_by(|a, b| {
        (a.category as u8)
            .cmp(&(b.category as u8))
            .then(b.recommended.cmp(&a.recommended))
            .then((a.fit as u8).cmp(&(b.fit as u8)))
            .then(rank(&b.model).cmp(&rank(&a.model)))
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Tier;

    fn hw(vram_gb: u64, ram_gb: u64) -> Hardware {
        Hardware {
            gpus: Vec::new(),
            vram_mb: vram_gb * 1024,
            ram_mb: ram_gb * 1024,
            cpu_threads: 16,
            disk_free_mb: 500_000,
            tier: Tier::from_vram_mb(vram_gb * 1024),
        }
    }

    #[test]
    fn fit_on_a_16gb_card() {
        let h = hw(16, 64);
        assert_eq!(fit(6.6, &h), Fit::Fits);
        assert_eq!(fit(18.0, &h), Fit::Partial);
        assert_eq!(fit(240.0, &h), Fit::TooLarge);
    }

    #[test]
    fn cpu_only_never_fits_fully() {
        let h = hw(0, 32);
        assert_eq!(fit(3.3, &h), Fit::Partial);
        assert_eq!(fit(18.0, &h), Fit::TooLarge);
    }

    #[test]
    fn one_recommendation_per_category() {
        let entries = build(&hw(16, 64));
        let general: Vec<_> = entries
            .iter()
            .filter(|e| e.category == Category::General && e.recommended)
            .collect();
        assert_eq!(general.len(), 1);
        // Unfiltered Qwen comes first and has its own pick.
        assert_eq!(entries[0].category, Category::Uncensored);
        assert!(entries[0].recommended);
        assert_eq!(entries[0].model, "huihui_ai/qwen3.5-abliterated:9b");
        assert_eq!(general[0].model, "ornith-1.5:9b");
        assert_eq!(general[0].fit, Fit::Fits);
        let big = build(&hw(32, 64));
        assert!(big.iter().any(|e| e.recommended && e.model == "ornith-1.5:35b"));
    }

    #[test]
    fn catalogue_names_are_unique() {
        let mut names: Vec<_> = CATALOG.iter().map(|e| e.model).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), CATALOG.len());
    }
}
