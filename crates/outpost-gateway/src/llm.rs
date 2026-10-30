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

/// Where the user's clock is: minutes ahead of UTC, and the zone's name if known.
#[derive(Debug, Clone, PartialEq)]
pub struct UserClock {
    pub offset_minutes: i32,
    pub zone: Option<String>,
}

impl UserClock {
    /// From what an app sent; nonsense is ignored.
    pub fn from_app(offset_minutes: Option<i32>, zone: Option<&str>) -> Option<UserClock> {
        let offset_minutes = offset_minutes.filter(|m| (-14 * 60..=14 * 60).contains(m))?;
        let zone = zone
            .map(str::trim)
            .filter(|z| !z.is_empty() && z.len() <= 64)
            .filter(|z| z.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c)))
            .map(str::to_string);
        Some(UserClock { offset_minutes, zone })
    }

    fn setting_key(account_id: &str) -> String {
        format!("clock.{account_id}")
    }

    /// Remember an account's clock, so apps that don't send one (the API, MCP) use it too.
    pub async fn save(&self, state: &AppState, account_id: &str) {
        let value = format!("{}|{}", self.offset_minutes, self.zone.as_deref().unwrap_or(""));
        let _ = outpost_core::settings::set(&state.pool, &Self::setting_key(account_id), &value).await;
    }

    pub async fn load(state: &AppState, account_id: &str) -> Option<UserClock> {
        let v = outpost_core::settings::get(&state.pool, &Self::setting_key(account_id)).await.ok()??;
        let (offset, zone) = v.split_once('|')?;
        UserClock::from_app(offset.parse().ok(), Some(zone))
    }
}

const CLOCK_PREFIX: &str = "Current date and time for the user:";

/// Takes the clock line off the end of a system prompt (see [`system_prompt`]).
fn split_clock(system: &str) -> (&str, Option<&str>) {
    match system.rfind(CLOCK_PREFIX) {
        Some(i) => (system[..i].trim_end(), Some(&system[i..])),
        None => (system, None),
    }
}

/// "Current date and time for the user: Friday, 30 October 2026, 12:14 pm (Australia/Sydney,
/// UTC+11:00)." Models have no clock of their own, so without this they guess (usually
/// the date their training ended).
pub fn clock_line(now: i64, clock: Option<&UserClock>) -> String {
    const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
        "December",
    ];
    let offset = clock.map(|c| c.offset_minutes).unwrap_or(0);
    let local = now + i64::from(offset) * 60;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    // The date part of diagnostics::rfc3339 (YYYY-MM-DD...).
    let date = crate::diagnostics::rfc3339(days * 86_400);
    let (y, m, d) = (&date[0..4], date[5..7].parse::<usize>().unwrap_or(1), date[8..10].parse::<u32>().unwrap_or(1));
    let (h, min) = (secs / 3600, secs % 3600 / 60);
    let (h12, ampm) = match h {
        0 => (12, "am"),
        1..=11 => (h, "am"),
        12 => (12, "pm"),
        _ => (h - 12, "pm"),
    };
    let sign = if offset < 0 { '-' } else { '+' };
    let utc = format!("UTC{sign}{:02}:{:02}", offset.abs() / 60, offset.abs() % 60);
    let zone = match clock {
        Some(UserClock { zone: Some(z), .. }) => format!("{z}, {utc}"),
        Some(_) => utc,
        None => "UTC; the user's own time zone isn't known".into(),
    };
    format!(
        "{CLOCK_PREFIX} {}, {d} {} {y}, {h12}:{min:02} {ampm} ({zone}). \
         Use this whenever the date, day or time matters.",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[m.clamp(1, 12) - 1],
    )
}

/// The persona's prompt plus memories, if it uses them, and the user's date and time.
/// `query` (the user's message) picks which learned memories are relevant.
pub async fn system_prompt(
    state: &AppState,
    account_id: &str,
    persona: Option<&Persona>,
    query: Option<&str>,
) -> Result<String, ApiError> {
    let clock = clock_line(outpost_core::now(), UserClock::load(state, account_id).await.as_ref());
    let Some(p) = persona else { return Ok(clock) };
    let memories = if p.use_memory {
        crate::memory::recall(state, account_id, p, query).await?
    } else {
        Vec::new()
    };
    let prompt = outpost_core::memory::compose_system_prompt(&p.system_prompt, &memories);
    Ok(if prompt.is_empty() { clock } else { format!("{prompt}\n\n{clock}") })
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
    // The clock goes beside the newest message instead: in a long chat, small models
    // otherwise repeat a wrong date from their own earlier replies.
    let (system, clock) = split_clock(system);
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
    if let Some(clock) = clock {
        let note = format!(
            "{clock} Earlier messages in this chat may give other dates or times; they are out of date or wrong."
        );
        let at = messages.iter().rposition(|m| m["role"] == "user").unwrap_or(messages.len());
        messages.insert(at, json!({ "role": "system", "content": note }));
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

    #[test]
    fn the_clock_reads_like_a_person_would_say_it() {
        // 2026-10-30 01:14 UTC is 12:14 pm in Sydney (UTC+11).
        let now = 1_793_322_840;
        let sydney = UserClock::from_app(Some(660), Some("Australia/Sydney"));
        assert_eq!(
            clock_line(now, sydney.as_ref()),
            "Current date and time for the user: Friday, 30 October 2026, 12:14 pm (Australia/Sydney, UTC+11:00). \
             Use this whenever the date, day or time matters."
        );
        // West of UTC, the date can still be the day before.
        let la = UserClock::from_app(Some(-420), Some("America/Los_Angeles"));
        assert!(clock_line(now, la.as_ref()).contains("Thursday, 29 October 2026, 6:14 pm (America/Los_Angeles, UTC-07:00)"));
        assert!(clock_line(now, None).contains("1:14 am (UTC; the user's own time zone isn't known)"));
        // Midnight and noon.
        assert!(clock_line(1_793_318_400 - 11 * 3600, sydney.as_ref()).contains("12:00 am"));
        // In a chat, the clock sits just before the newest message, not at the top.
        let history = vec![msg(1, "user", "date?"), msg(2, "assistant", "23 September 2025"), msg(3, "user", "and now?")];
        let system = format!("Be terse.\n\n{}", clock_line(now, sydney.as_ref()));
        let body = ollama_chat_body("m", &system, &history, &HashMap::new(), false, None);
        let m = body["messages"].as_array().unwrap();
        assert_eq!(m[0]["content"], "Be terse.");
        assert_eq!(m[3]["role"], "system");
        assert!(m[3]["content"].as_str().unwrap().starts_with("Current date and time for the user: Friday, 30 October 2026"));
        assert_eq!(m[4]["content"], "and now?");
        // Nonsense from an app is ignored.
        assert!(UserClock::from_app(Some(5000), None).is_none());
        assert_eq!(UserClock::from_app(Some(60), Some("Not a zone!")).unwrap().zone, None);
    }

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
