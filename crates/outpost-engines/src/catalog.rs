//! Curated chat and embedding models for Ollama, with badges and a "does it fit"
//! score for this PC. Sizes are Ollama's listed download sizes (October 2026);
//! after a download the real size from Ollama is shown instead. Unfiltered models
//! are listed first, Qwen at the top, as the owner prefers them.

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
    /// Listed first: unfiltered models (Qwen preferred, plus Gemma, GLM, gpt-oss and more).
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
        model: "gemma4:31b",
        name: "Gemma 4 31B",
        category: Category::General,
        size_gb: 20.0,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 87,
        description: "The dense Gemma 4. Excellent writing and vision on 24 to 32 GB cards.",
        page: "https://ollama.com/library/gemma4",
    },
    Entry {
        model: "qwen3.5:35b",
        name: "Qwen 3.5 35B-A3B",
        category: Category::General,
        size_gb: 22.0,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 86,
        description: "Mixture-of-experts Qwen 3.5: big-model quality, quick replies. For 32 GB cards.",
        page: "https://ollama.com/library/qwen3.5",
    },
    Entry {
        model: "qwen3.5:2b",
        name: "Qwen 3.5 2B",
        category: Category::General,
        size_gb: 3.1,
        context_k: 256,
        badges: &[Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 25,
        description: "Very small and quick. For older laptops and CPU-only machines.",
        page: "https://ollama.com/library/qwen3.5",
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
    // Other unfiltered families (sizes and tags from ollama.com, October 2026).
    // Ranked below the 9B Qwen so Qwen stays the pick on 16 GB, as the owner prefers.
    Entry {
        model: "huihui_ai/gemma-4-abliterated:26b",
        name: "Gemma 4 26B Abliterated",
        category: Category::Uncensored,
        size_gb: 18.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 82,
        description: "Google's Gemma 4 with refusals removed. Strong writer with vision; best on 20 GB+ cards.",
        page: "https://ollama.com/huihui_ai/gemma-4-abliterated",
    },
    Entry {
        model: "huihui_ai/gemma-4-abliterated:12b",
        name: "Gemma 4 12B Abliterated",
        category: Category::Uncensored,
        size_gb: 7.6,
        context_k: 128,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 68,
        description: "Mid-size Gemma 4 without refusals. Natural, creative writing; reads images.",
        page: "https://ollama.com/huihui_ai/gemma-4-abliterated",
    },
    Entry {
        model: "huihui_ai/gemma-4-abliterated:e4b",
        name: "Gemma 4 E4B Abliterated",
        category: Category::Uncensored,
        size_gb: 9.6,
        context_k: 128,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 40,
        description: "Small, efficient Gemma 4 for laptops and 8 GB cards. Reads images.",
        page: "https://ollama.com/huihui_ai/gemma-4-abliterated",
    },
    Entry {
        model: "huihui_ai/glm-4.7-flash-abliterated:latest",
        name: "GLM 4.7 Flash Abliterated",
        category: Category::Uncensored,
        size_gb: 19.0,
        context_k: 198,
        badges: &[Uncensored, Tools, Thinking, Code, LongContext],
        license: None,
        rank: 78,
        description:
            "Zhipu's fast 30B-class mixture-of-experts model, no refusals. Good at code and agents; text only.",
        page: "https://ollama.com/huihui_ai/glm-4.7-flash-abliterated",
    },
    Entry {
        model: "huihui_ai/gpt-oss-abliterated:20b",
        name: "gpt-oss 20B Abliterated",
        category: Category::Uncensored,
        size_gb: 14.0,
        context_k: 128,
        badges: &[Uncensored, Tools, Thinking, LongContext],
        license: Some("Apache-2.0"),
        rank: 74,
        description: "OpenAI's open-weight model with refusals removed. Strong reasoning and tool use; text only.",
        page: "https://ollama.com/huihui_ai/gpt-oss-abliterated",
    },
    Entry {
        model: "huihui_ai/granite4.1-abliterated:8b",
        name: "Granite 4.1 8B Abliterated",
        category: Category::Uncensored,
        size_gb: 5.3,
        context_k: 128,
        badges: &[Uncensored, Tools, LongContext],
        license: Some("Apache-2.0"),
        rank: 48,
        description: "IBM Granite 4.1 without refusals. Quick and good with tools; text only.",
        page: "https://ollama.com/huihui_ai/granite4.1-abliterated",
    },
    // Sizes for every machine, from 8 GB laptops to 48 GB workstations (ollama.com, October 2026).
    Entry {
        model: "huihui_ai/qwen3.5-abliterated:0.8b",
        name: "Qwen 3.5 0.8B Abliterated",
        category: Category::Uncensored,
        size_gb: 1.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 20,
        description: "The smallest uncensored Qwen. For old laptops and CPU-only machines; simple tasks only.",
        page: "https://ollama.com/huihui_ai/qwen3.5-abliterated",
    },
    Entry {
        model: "huihui_ai/Qwen3.6-abliterated:27b",
        name: "Qwen 3.6 27B Abliterated",
        category: Category::Uncensored,
        size_gb: 17.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 86,
        description: "Qwen 3.6 with refusals removed, with better agentic coding. For 24 GB cards.",
        page: "https://ollama.com/huihui_ai/Qwen3.6-abliterated",
    },
    Entry {
        model: "huihui_ai/Qwen3.6-abliterated:35b",
        name: "Qwen 3.6 35B-A3B Abliterated",
        category: Category::Uncensored,
        size_gb: 24.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 89,
        description: "Mixture-of-experts Qwen 3.6 without refusals. Quick for its size on 32 GB cards.",
        page: "https://ollama.com/huihui_ai/Qwen3.6-abliterated",
    },
    Entry {
        model: "huihui_ai/gemma-4-abliterated:31b",
        name: "Gemma 4 31B Abliterated",
        category: Category::Uncensored,
        size_gb: 20.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 85,
        description: "Dense Gemma 4 without refusals. Rich writing and vision for 24 to 32 GB cards.",
        page: "https://ollama.com/huihui_ai/gemma-4-abliterated",
    },
    Entry {
        model: "huihui_ai/gemma-4-abliterated:48b",
        name: "Gemma 4 48B Abliterated",
        category: Category::Uncensored,
        size_gb: 33.0,
        context_k: 256,
        badges: &[Uncensored, Vision, Tools, Thinking, LongContext],
        license: None,
        rank: 93,
        description: "The largest Gemma 4, refusals removed. Top quality on 48 GB workstations.",
        page: "https://ollama.com/huihui_ai/gemma-4-abliterated",
    },
    Entry {
        model: "huihui_ai/granite4.1-abliterated:30b",
        name: "Granite 4.1 30B Abliterated",
        category: Category::Uncensored,
        size_gb: 17.0,
        context_k: 128,
        badges: &[Uncensored, Tools, LongContext],
        license: Some("Apache-2.0"),
        rank: 72,
        description: "Large IBM Granite 4.1 without refusals. Dependable with tools; text only.",
        page: "https://ollama.com/huihui_ai/granite4.1-abliterated",
    },
    Entry {
        model: "huihui_ai/granite4.1-abliterated:3b",
        name: "Granite 4.1 3B Abliterated",
        category: Category::Uncensored,
        size_gb: 2.1,
        context_k: 128,
        badges: &[Uncensored, Tools, LongContext],
        license: Some("Apache-2.0"),
        rank: 30,
        description: "Tiny Granite without refusals. Fast on any laptop; text only.",
        page: "https://ollama.com/huihui_ai/granite4.1-abliterated",
    },
    Entry {
        model: "huihui_ai/llama3.3-abliterated:70b",
        name: "Llama 3.3 70B Abliterated",
        category: Category::Uncensored,
        size_gb: 43.0,
        context_k: 128,
        badges: &[Uncensored, LongContext],
        license: None,
        rank: 80,
        description: "Meta's Llama 3.3 70B without refusals. Older but very knowledgeable; needs 48 GB.",
        page: "https://ollama.com/huihui_ai/llama3.3-abliterated",
    },
    Entry {
        model: "dolphin3:8b",
        name: "Dolphin 3.0 8B",
        category: Category::Uncensored,
        size_gb: 4.9,
        context_k: 128,
        badges: &[Uncensored, LongContext],
        license: None,
        rank: 45,
        description:
            "Classic uncensored Llama 3.1 fine-tune by Cognitive Computations. Older, but steerable and light.",
        page: "https://ollama.com/library/dolphin3",
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

/// Memory a model needs: its weights plus working space, which grows a little with the
/// context window Outpost gives it on this card (0.6 GB, plus 0.6 GB more at 64K: flash
/// attention and the 8-bit KV cache keep the window cheap).
fn needed_gb(size_gb: f32, context: u32) -> f32 {
    size_gb * 1.15 + 0.6 + 0.6 * (context as f32 / 65_536.0)
}

pub fn fit(size_gb: f32, hw: &Hardware) -> Fit {
    let need = needed_gb(size_gb, crate::versions::context_length_for(hw.vram_mb));
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

    // Within each category: the recommendation; then everything that fits, best
    // first down to the smallest; then models too big for the graphics card,
    // closest to fitting first: "runs slowly" before "too big for this PC".
    let rank = |m: &str| CATALOG.iter().find(|e| e.model == m).map(|e| e.rank).unwrap_or(0);
    entries.sort_by(|a, b| {
        let within = if a.fit == Fit::Fits {
            rank(&b.model).cmp(&rank(&a.model))
        } else {
            a.size_gb.total_cmp(&b.size_gb)
        };
        (a.category as u8)
            .cmp(&(b.category as u8))
            .then(b.recommended.cmp(&a.recommended))
            .then((a.fit as u8).cmp(&(b.fit as u8)))
            .then(within)
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
    fn unfiltered_has_more_than_qwen_but_qwen_leads() {
        let entries = build(&hw(16, 64));
        let unfiltered: Vec<_> = entries.iter().filter(|e| e.category == Category::Uncensored).collect();
        assert!(unfiltered.iter().any(|e| e.model.contains("gemma")));
        assert!(unfiltered.iter().any(|e| !e.model.to_lowercase().contains("qwen")));
        assert!(unfiltered.iter().all(|e| e.badges.contains(&Badge::Uncensored)));
        // Every chat model here handles Outpost's 64K window (Hermes needs it).
        assert!(unfiltered.iter().all(|e| e.context_k >= 64));
        assert!(unfiltered[0].model.contains("qwen"));
    }

    #[test]
    fn order_is_pick_then_fits_big_to_small_then_too_big() {
        for (vram, ram) in [(8, 16), (16, 64), (24, 64), (48, 128)] {
            let entries = build(&hw(vram, ram));
            for cat in [Category::Uncensored, Category::General] {
                let list: Vec<_> = entries.iter().filter(|e| e.category == cat).collect();
                assert!(list[0].recommended, "{vram} GB: pick first");
                let rest = &list[1..];
                // Fit groups never go backwards.
                assert!(
                    rest.windows(2).all(|w| (w[0].fit as u8) <= (w[1].fit as u8)),
                    "{vram} GB groups"
                );
                // Fitting models go best to smallest; bigger ones smallest first.
                let rank = |m: &str| CATALOG.iter().find(|e| e.model == m).unwrap().rank;
                for w in rest.windows(2).filter(|w| w[0].fit == w[1].fit) {
                    if w[0].fit == Fit::Fits {
                        assert!(
                            rank(&w[0].model) >= rank(&w[1].model),
                            "{vram} GB: {} before {}",
                            w[0].model,
                            w[1].model
                        );
                    } else {
                        assert!(
                            w[0].size_gb <= w[1].size_gb,
                            "{vram} GB: {} before {}",
                            w[0].model,
                            w[1].model
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_size_of_machine_gets_a_fitting_pick() {
        // 8 GB laptop to 48 GB workstation: something unfiltered fits fully on each.
        for vram in [8, 12, 16, 24, 32, 48] {
            let entries = build(&hw(vram, 32));
            let pick = entries
                .iter()
                .find(|e| e.category == Category::Uncensored && e.recommended)
                .unwrap();
            assert_eq!(pick.fit, Fit::Fits, "{vram} GB pick {}", pick.model);
            // Qwen is the owner's preference wherever a Qwen is the best that fits.
            if vram < 48 {
                assert!(
                    pick.model.to_lowercase().contains("qwen"),
                    "{vram} GB pick {}",
                    pick.model
                );
            }
        }
        let big = build(&hw(48, 128));
        assert!(big
            .iter()
            .any(|e| e.recommended && e.model == "huihui_ai/gemma-4-abliterated:48b"));
    }

    #[test]
    fn catalogue_names_are_unique() {
        let mut names: Vec<_> = CATALOG.iter().map(|e| e.model).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), CATALOG.len());
    }
}
