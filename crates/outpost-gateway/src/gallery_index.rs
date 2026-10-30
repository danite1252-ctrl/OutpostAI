//! Gallery search: describing pictures in the background, and finding them.
//!
//! A vision model (one that can see pictures; the same kind Train uses for captions) writes
//! a sentence or two about each picture. Its prompt and that description get an embedding
//! (the model memory uses), so a search finds pictures by meaning; without one it finds
//! them by words. Describing only happens while the graphics card is free: no Create job
//! running or waiting here, no training, and no chat for a couple of minutes.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use base64::Engine;
use outpost_core::media_index;
use outpost_core::settings;
use serde_json::{json, Value};

use crate::AppState;

/// Describe for search: short, factual, the words people search with.
pub const DESCRIBE_PROMPT: &str = "Describe this picture for a search index, in one or two plain sentences: \
the main subjects and what they are doing, the setting, notable objects, colours, time of day, and the style \
(photo, painting, cartoon, 3D render...). No preamble, no opinions.";

/// Pictures bigger than this aren't sent to the vision model.
const MAX_FILE: u64 = 30 * 1024 * 1024;
/// Chat replies in the last this-many seconds keep the describer waiting.
const CHAT_QUIET_SECS: i64 = 120;
/// Embeddings made per pass.
const EMBED_BATCH: i64 = 32;

static LAST_CHAT: AtomicI64 = AtomicI64::new(0);

/// A chat reply is starting: give the graphics card to chat for a while.
pub fn chat_active() {
    LAST_CHAT.store(outpost_core::now(), Ordering::Relaxed);
}

/// A chat reply started in the last `secs` seconds.
pub fn chat_recently(secs: i64) -> bool {
    outpost_core::now() - LAST_CHAT.load(Ordering::Relaxed) < secs
}

/// Describing is on (until the owner turns it off).
pub async fn enabled(state: &AppState) -> bool {
    settings::get(&state.pool, settings::GALLERY_DESCRIBE).await.ok().flatten().as_deref() != Some("off")
}

/// Small vision models that describe pictures well, preferred in this order when installed.
pub const DESCRIBERS: [&str; 4] = ["qwen3.5:4b", "huihui_ai/qwen3.5-abliterated:4b", "qwen3.5:2b", "huihui_ai/qwen3.5-abliterated:2b"];
/// The ones suggested for download when there's no vision model (the first that fits).
pub const SUGGESTED_DESCRIBERS: [&str; 2] = ["qwen3.5:4b", "qwen3.5:2b"];
/// The embedding model suggested for search (and memory).
pub const EMBEDDER: &str = "qwen3-embedding:0.6b";

/// The vision model that describes pictures, if one is installed: a small one of
/// [`DESCRIBERS`] if there, else the smallest that can see from 2 GB up (lightest on the card,
/// but not one too small to describe well).
pub async fn describer(state: &AppState) -> Option<String> {
    let mut seeing: Vec<(String, u64)> = state
        .models
        .installed()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|m| {
            m.badges.contains(&outpost_engines::catalog::Badge::Vision)
                && !m.badges.contains(&outpost_engines::catalog::Badge::Embedding)
        })
        .map(|m| (m.model, m.size_bytes))
        .collect();
    if let Some(pick) = DESCRIBERS.iter().find(|d| seeing.iter().any(|(m, _)| m == *d)) {
        return Some(pick.to_string());
    }
    seeing.sort_by_key(|(_, size)| (*size < 2_000_000_000, *size));
    seeing.into_iter().next().map(|(m, _)| m)
}

/// Run forever: describe pictures, then embed what needs it.
pub async fn run(state: AppState) {
    // Let start-up settle first.
    tokio::time::sleep(Duration::from_secs(60)).await;
    loop {
        let worked = pass(&state).await;
        let wait = if worked { 2 } else { 60 };
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

/// One step of work; true if it did something (so the next one comes soon).
async fn pass(state: &AppState) -> bool {
    let mut worked = embed_some(state).await;
    if !enabled(state).await || !card_free(state).await {
        return worked;
    }
    let Some(model) = describer(state).await else { return worked };
    let Ok(Some((media, path))) = media_index::next_to_describe(&state.pool, &state.config.files_dir()).await else {
        return worked;
    };
    worked = true;
    match describe(state, &model, &path).await {
        Ok(text) => {
            tracing::debug!(media = %media.id, "described a picture for search");
            let _ = media_index::set_description(&state.pool, &media.id, &model, &text).await;
        }
        Err(e) => {
            tracing::info!(media = %media.id, error = %e, "couldn't describe a picture");
            let _ = media_index::set_failed(&state.pool, &media.id, &e).await;
        }
    }
    worked
}

/// Nothing else wants the graphics card right now.
async fn card_free(state: &AppState) -> bool {
    if chat_recently(CHAT_QUIET_SECS) {
        return false;
    }
    if state.queue.running().is_some() {
        return false;
    }
    // Jobs waiting to run here (not ones waiting for a helper PC).
    let here = crate::helpers::local_packs(state).await;
    !matches!(outpost_core::jobs::has_queued_of(&state.pool, &here).await, Ok(true))
}

async fn describe(state: &AppState, model: &str, path: &std::path::Path) -> Result<String, String> {
    let size = tokio::fs::metadata(path).await.map_err(|e| e.to_string())?.len();
    if size > MAX_FILE {
        return Err("too big to describe".into());
    }
    let data = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    let thinks = state
        .models
        .installed()
        .await
        .unwrap_or_default()
        .iter()
        .any(|m| m.model == model && m.badges.contains(&outpost_engines::catalog::Badge::Thinking));
    let mut body = json!({
        "model": model,
        "stream": false,
        "messages": [{ "role": "user", "content": DESCRIBE_PROMPT, "images": [b64] }],
        "options": { "temperature": 0.2, "num_predict": 160 },
        // Hand the card back soon: ComfyUI may want it.
        "keep_alive": "2m",
    });
    if thinks {
        body["think"] = json!(false);
    }
    let url = format!("{}/api/chat", state.config.ollama_url.trim_end_matches('/'));
    let resp = tokio::time::timeout(Duration::from_secs(300), state.llm.post(&url).json(&body).send())
        .await
        .map_err(|_| "the model took too long".to_string())?
        .map_err(|_| "Ollama isn't running".to_string())?;
    if !resp.status().is_success() {
        return Err(resp.text().await.unwrap_or_default().chars().take(300).collect());
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    let text = tidy(v["message"]["content"].as_str().unwrap_or_default());
    if text.is_empty() {
        return Err("the model said nothing".into());
    }
    Ok(text)
}

/// The description as one tidy paragraph (no reasoning block, no "This image shows").
pub fn tidy(text: &str) -> String {
    let t = crate::routes::training::strip_thinking(text);
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = t.to_lowercase();
    let t = ["this image shows ", "this picture shows ", "the image shows ", "the picture shows ", "this is "]
        .iter()
        .find(|p| lower.starts_with(*p))
        .map(|p| {
            let rest = &t[p.len()..];
            let mut c = rest.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .unwrap_or(t);
    t.chars().take(600).collect()
}

/// Embed descriptions and prompts that have no embedding from the current model.
async fn embed_some(state: &AppState) -> bool {
    let Some(model) = crate::memory::embedding_model(state).await else { return false };
    let Ok(todo) = media_index::needing_embedding(&state.pool, &model, EMBED_BATCH).await else { return false };
    if todo.is_empty() {
        return false;
    }
    let texts: Vec<String> = todo.iter().map(|(_, t)| t.clone()).collect();
    let Some(vectors) = crate::memory::embed(state, &model, &texts).await else { return false };
    for ((id, _), v) in todo.iter().zip(vectors) {
        let _ = media_index::set_embedding(&state.pool, id, &model, &v).await;
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn descriptions_are_tidied() {
        assert_eq!(super::tidy("<think>x</think>\n This image shows a dog\n on a beach. "), "A dog on a beach.");
        assert_eq!(super::tidy("A red bicycle."), "A red bicycle.");
    }
}
