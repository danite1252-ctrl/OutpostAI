//! Turning a persona + chat history into an Ollama request.

use outpost_core::chats::Message;
use outpost_core::personas::Persona;
use outpost_engines::catalog::Badge;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::AppState;

/// How many earlier messages go back to the model with each new one.
pub const HISTORY: usize = 40;

/// Pick the model: the chat's override, else the persona's, else the first
/// installed chat model. Also says whether that model can "think".
pub async fn resolve_model(
    state: &AppState,
    chat_model: Option<&str>,
    persona: Option<&Persona>,
) -> Result<(String, bool), ApiError> {
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
    Ok((chosen.model.clone(), chosen.badges.contains(&Badge::Thinking)))
}

/// The persona's prompt plus memories, if it uses them.
pub async fn system_prompt(state: &AppState, account_id: &str, persona: Option<&Persona>) -> Result<String, ApiError> {
    let Some(p) = persona else { return Ok(String::new()) };
    let memories = if p.use_memory {
        outpost_core::memory::for_prompt(&state.pool, account_id, Some(&p.id)).await?
    } else {
        Vec::new()
    };
    Ok(outpost_core::memory::compose_system_prompt(&p.system_prompt, &memories))
}

/// Body for Ollama's native `/api/chat`, streaming.
pub fn ollama_chat_body(
    model: &str,
    system: &str,
    history: &[Message],
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
            messages.push(json!({ "role": m.role, "content": m.content }));
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
        }
    }

    #[test]
    fn body_has_system_history_and_think_switch() {
        let history: Vec<_> = (1..=50)
            .map(|i| msg(i, if i % 2 == 1 { "user" } else { "assistant" }, &format!("m{i}")))
            .collect();
        let b = ollama_chat_body("qwen3.5:9b", "Be terse.", &history, true, None);
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs.len(), 1 + HISTORY);
        assert_eq!(msgs.last().unwrap()["content"], "m50");
        assert_eq!(b["think"], false);

        let no_think = ollama_chat_body("gemma", "", &history[..1], false, None);
        assert!(no_think.get("think").is_none());
        assert_eq!(no_think["messages"].as_array().unwrap().len(), 1);
    }
}
