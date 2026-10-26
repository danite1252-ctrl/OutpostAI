//! Turning a persona + chat history into an Ollama request.

use std::collections::HashMap;

use outpost_core::chats::Message;
use outpost_core::personas::Persona;
use outpost_engines::catalog::Badge;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::AppState;

/// How many earlier messages go back to the model with each new one.
pub const HISTORY: usize = 40;

/// Most images sent back to the model with one request (the newest win).
pub const HISTORY_IMAGES: usize = 6;

/// The model a chat will use and what it can do.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub model: String,
    /// Accepts `think` (reasoning on or off).
    pub can_think: bool,
    /// Accepts images.
    pub can_see: bool,
}

/// Pick the model: the chat's override, else the persona's, else the first
/// installed chat model, and say what it can do.
pub async fn resolve_model(
    state: &AppState,
    chat_model: Option<&str>,
    persona: Option<&Persona>,
) -> Result<Resolved, ApiError> {
    // A training fills the graphics card; a chat model loading now could crash it.
    if state.queue.training() {
        return Err(ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "gpu_training",
            "The graphics card is training a character, so chat is paused until it's done. The queue shows how long is left.",
        ));
    }
    let wanted = chat_model.or_else(|| persona.and_then(|p| p.model.as_deref()));
    let installed = state.models.installed().await.map_err(|_| {
        ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "ollama_offline",
            "Ollama isn't running. Start it in Settings → Engines.",
        )
    })?;
    let chosen = match wanted {
        Some(m) => installed.iter().find(|i| i.model == m).ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "model_missing",
                format!("{m} isn't downloaded. Get it in Models, or pick another model for this persona."),
            )
        })?,
        None => installed
            .iter()
            .find(|i| !i.badges.contains(&Badge::Embedding))
            .ok_or_else(|| {
                ApiError::new(
                    axum::http::StatusCode::BAD_REQUEST,
                    "model_missing",
                    "No chat model is downloaded yet. Get one in Models.",
                )
            })?,
    };
    Ok(Resolved {
        model: chosen.model.clone(),
        can_think: chosen.badges.contains(&Badge::Thinking),
        can_see: chosen.badges.contains(&Badge::Vision),
    })
}

/// Base64 images for the newest attachments in the history window, keyed by message
/// id, for a model that can see. Unreadable files are skipped.
pub async fn history_images(state: &AppState, account_id: &str, history: &[Message]) -> HashMap<String, Vec<String>> {
    use base64::Engine;
    let start = history.len().saturating_sub(HISTORY);
    let files_dir = state.config.files_dir();
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut left = HISTORY_IMAGES;
    for m in history[start..].iter().rev() {
        for a in m.attachments.iter().rev() {
            if left == 0 {
                return out;
            }
            let Ok((_, path)) = outpost_core::attachments::get(&state.pool, &files_dir, account_id, &a.id).await else {
                continue;
            };
            let Ok(data) = outpost_core::attachments::read(&path).await else {
                continue;
            };
            out.entry(m.id.clone())
                .or_default()
                .insert(0, base64::engine::general_purpose::STANDARD.encode(data));
            left -= 1;
        }
    }
    out
}

/// The persona's prompt plus memories, if it uses them. `query` (the user's
/// message) picks which learned memories are relevant.
pub async fn system_prompt(
    state: &AppState,
    account_id: &str,
    persona: Option<&Persona>,
    query: Option<&str>,
) -> Result<String, ApiError> {
    let Some(p) = persona else { return Ok(String::new()) };
    let memories = if p.use_memory {
        crate::memory::recall(state, account_id, p, query).await?
    } else {
        Vec::new()
    };
    Ok(outpost_core::memory::compose_system_prompt(&p.system_prompt, &memories))
}

/// Body for Ollama's native `/api/chat`, streaming.
/// A chat message that started a picture (`/draw`) carries a card marker for the
/// apps, `[[outpost-job:<id>]]`; the model sees the words only.
pub fn without_job_cards(content: &str) -> String {
    content
        .lines()
        .filter(|l| !l.trim_start().starts_with("[[outpost-job:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

pub fn ollama_chat_body(
    model: &str,
    system: &str,
    history: &[Message],
    images: &HashMap<String, Vec<String>>,
    can_think: bool,
    persona: Option<&Persona>,
) -> Value {
    let mut messages = Vec::new();
    if !system.is_empty() {
        messages.push(json!({ "role": "system", "content": system }));
    }
    let start = history.len().saturating_sub(HISTORY);
    for m in &history[start..] {
        if m.role == "user" || m.role == "assistant" {
            let mut msg = json!({ "role": m.role, "content": without_job_cards(&m.content) });
            if let Some(imgs) = images.get(&m.id).filter(|i| !i.is_empty()) {
                msg["images"] = json!(imgs);
            }
            messages.push(msg);
        }
    }
    let mut body = json!({ "model": model, "messages": messages, "stream": true });
    // Only thinking models accept `think`; for them, follow the persona's switch.
    if can_think {
        body["think"] = json!(persona.is_some_and(|p| p.think));
    }
    if let Some(t) = persona.and_then(|p| p.temperature) {
        body["options"] = json!({ "temperature": t });
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(seq: i64, role: &str, content: &str) -> Message {
        Message {
            id: seq.to_string(),
            chat_id: "c".into(),
            seq,
            role: role.into(),
            content: content.into(),
            thinking: None,
            model: None,
            created_at: 0,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn body_has_system_history_and_think_switch() {
        let history: Vec<_> = (1..=50)
            .map(|i| msg(i, if i % 2 == 1 { "user" } else { "assistant" }, &format!("m{i}")))
            .collect();
        let none = HashMap::new();
        let b = ollama_chat_body("qwen3.5:9b", "Be terse.", &history, &none, true, None);
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs.len(), 1 + HISTORY);
        assert_eq!(msgs.last().unwrap()["content"], "m50");
        assert_eq!(b["think"], false);

        let no_think = ollama_chat_body("gemma", "", &history[..1], &none, false, None);
        assert!(no_think.get("think").is_none());
        assert_eq!(no_think["messages"].as_array().unwrap().len(), 1);
        assert!(no_think["messages"][0].get("images").is_none());

        let mut imgs = HashMap::new();
        imgs.insert("50".to_string(), vec!["aGk=".to_string()]);
        let seeing = ollama_chat_body("ornith", "", &history, &imgs, false, None);
        let last = seeing["messages"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(last["images"][0], "aGk=");
    }
}
