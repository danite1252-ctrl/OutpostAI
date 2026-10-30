//! Voice: speech to text and text to speech through Outpost's voice service (Whisper
//! and Kokoro on the CPU). Outpost's own endpoints, plus OpenAI-style
//! `/v1/audio/transcriptions` and `/v1/audio/speech` for other apps.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Multipart, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use base64::Engine;
use outpost_core::personas;
use outpost_engines::{EngineId, EngineState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{IntoParams, ToSchema};

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// The voice used when a persona has none.
pub const DEFAULT_VOICE: &str = "af_heart";
/// Longest text spoken in one go.
pub const MAX_SPEAK_CHARS: usize = 5000;
/// Largest recording accepted.
pub const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;

fn voice_url(path: &str) -> String {
    format!("{}{path}", EngineId::Voice.base_url())
}

fn offline() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "voice_offline",
        "Voice isn't running. Install or start it in Settings › Engines.",
    )
}

fn engine_failed(message: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "voice_failed", message)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VoiceInfo {
    /// e.g. `af_heart`.
    pub id: String,
    /// e.g. "Heart".
    pub name: String,
    /// e.g. "American English".
    pub language: String,
    /// `female` or `male`.
    pub gender: String,
}

/// What a Kokoro voice name says: first letter the language, second the gender.
pub fn describe_voice(id: &str) -> VoiceInfo {
    let mut chars = id.chars();
    let language = match chars.next() {
        Some('a') => "American English",
        Some('b') => "British English",
        Some('e') => "Spanish",
        Some('f') => "French",
        Some('h') => "Hindi",
        Some('i') => "Italian",
        Some('j') => "Japanese",
        Some('p') => "Brazilian Portuguese",
        Some('z') => "Mandarin Chinese",
        _ => "Other",
    };
    let gender = match chars.next() {
        Some('f') => "female",
        Some('m') => "male",
        _ => "",
    };
    let raw = id.split_once('_').map(|(_, n)| n).unwrap_or(id);
    let name = raw
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    VoiceInfo {
        id: id.to_string(),
        name,
        language: language.into(),
        gender: gender.into(),
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VoiceStatus {
    /// The voice service's engine state.
    pub state: EngineState,
    /// It's running and answering.
    pub ready: bool,
    /// Voices to speak with, English first.
    pub voices: Vec<VoiceInfo>,
    pub default_voice: String,
}

/// Whether voice is ready, and its voices.
#[utoipa::path(get, path = "/api/voice", tag = "voice", security(("bearer" = [])),
    responses((status = 200, description = "Voice status", body = VoiceStatus)))]
pub async fn status(State(state): State<AppState>, _auth: Auth) -> Json<VoiceStatus> {
    let engine = state.engines.status().await;
    let st = engine
        .engines
        .iter()
        .find(|e| e.id == EngineId::Voice)
        .map(|e| e.state)
        .unwrap_or(EngineState::NotInstalled);
    let mut voices = Vec::new();
    if matches!(st, EngineState::Running | EngineState::External) {
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(60)).build();
        if let Ok(client) = client {
            if let Ok(r) = client.get(voice_url("/voices")).send().await {
                if let Ok(v) = r.json::<Value>().await {
                    voices = v["voices"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(describe_voice)
                        .collect();
                }
            }
        }
    }
    // English first, then by language and name.
    voices.sort_by_key(|v| (!v.language.ends_with("English"), v.language.clone(), v.name.clone()));
    Json(VoiceStatus {
        ready: !voices.is_empty(),
        state: st,
        voices,
        default_voice: DEFAULT_VOICE.into(),
    })
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TranscribeRequest {
    /// The recording as base64 or a `data:audio/...;base64,` URL: WebM, Ogg, M4A, MP3,
    /// WAV... up to 25 MB.
    pub audio: String,
    /// The language spoken (`en`, `fr`...); leave out to detect it.
    pub language: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Transcript {
    pub text: String,
    /// The language heard.
    pub language: String,
    /// Seconds of audio.
    pub duration: f64,
}

fn decode_audio(encoded: &str) -> Result<Vec<u8>, ApiError> {
    let raw = match encoded.split_once(";base64,") {
        Some((head, rest)) if head.starts_with("data:") => rest,
        _ => encoded,
    };
    if raw.len() > MAX_AUDIO_BYTES / 3 * 4 + 8 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", "Recordings can be up to 25 MB."));
    }
    let cleaned: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(cleaned.as_bytes())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid", "That recording couldn't be read."))
}

fn clean_language(l: Option<&str>) -> Option<String> {
    l.map(str::trim)
        .filter(|l| (2..=8).contains(&l.len()) && l.chars().all(|c| c.is_ascii_alphabetic() || c == '-'))
        .map(|l| l.to_ascii_lowercase())
}

/// Send audio to the voice service.
pub async fn transcribe_bytes(state: &AppState, audio: Vec<u8>, language: Option<String>, hints: &str) -> ApiResult<Transcript> {
    if audio.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", "The recording is empty."));
    }
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(l) = language {
        query.push(("language", l));
    }
    if !hints.is_empty() {
        query.push(("hotwords", hints.to_string()));
    }
    let resp = state
        .llm
        .post(voice_url("/stt"))
        .query(&query)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .timeout(Duration::from_secs(300))
        .body(audio)
        .send()
        .await
        .map_err(|_| offline())?;
    let ok = resp.status().is_success();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !ok {
        return Err(engine_failed(
            v["error"].as_str().unwrap_or("The voice service couldn't hear that.").to_string(),
        ));
    }
    Ok(Transcript {
        text: v["text"].as_str().unwrap_or_default().trim().to_string(),
        language: v["language"].as_str().unwrap_or_default().to_string(),
        duration: v["duration"].as_f64().unwrap_or(0.0),
    })
}

/// Turn speech into text.
#[utoipa::path(post, path = "/api/voice/transcribe", tag = "voice", security(("bearer" = [])),
    request_body = TranscribeRequest,
    responses((status = 200, description = "What was said", body = Transcript), (status = 503, description = "Voice isn't running", body = ErrorBody)))]
pub async fn transcribe(State(state): State<AppState>, auth: Auth, Json(req): Json<TranscribeRequest>) -> ApiResult<Json<Transcript>> {
    let audio = decode_audio(&req.audio)?;
    let hints = name_hints(&state, &auth.account.id).await;
    Ok(Json(transcribe_bytes(&state, audio, clean_language(req.language.as_deref()), &hints).await?))
}

/// Your characters' and personas' names, for Whisper to listen for ("Kat, zk4t, Ada").
async fn name_hints(state: &AppState, account_id: &str) -> String {
    let mut names: Vec<String> = Vec::new();
    if let Ok(chars) = outpost_core::training::characters(&state.pool, Some(account_id)).await {
        for c in chars {
            names.push(c.name);
        }
    }
    if let Ok(ps) = personas::list(&state.pool, account_id).await {
        for p in ps {
            names.push(p.name);
        }
    }
    names.retain(|n| !n.trim().is_empty());
    names.dedup();
    names.join(", ").chars().take(400).collect()
}

/// Text as it should be read aloud: no code, Markdown marks, links or picture cards.
pub fn speakable(text: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            if in_code {
                out.push_str("(code) ");
            }
            continue;
        }
        if in_code || t.starts_with("[[outpost-job:") || t.starts_with('|') {
            continue;
        }
        let t = t.trim_start_matches(['#', '>', '-', '*', ' ']);
        out.push_str(t);
        out.push(if t.ends_with(['.', '!', '?', ':', ';']) || t.is_empty() { ' ' } else { '\n' });
    }
    // [label](url) -> label; drop emphasis and inline code marks.
    let mut s = String::new();
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' | '`' | '~' => {}
            ']' if chars.peek() == Some(&'(') => {
                for d in chars.by_ref() {
                    if d == ')' {
                        break;
                    }
                }
            }
            '[' => {}
            _ => s.push(c),
        }
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(MAX_SPEAK_CHARS).collect()
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SpeakRequest {
    pub text: String,
    /// A voice id from `GET /api/voice`; leave out for the persona's or the default.
    pub voice: Option<String>,
    /// Use this persona's voice.
    pub persona_id: Option<String>,
    /// 0.5 to 2 (default 1).
    pub speed: Option<f64>,
}

/// Speak `text` with `voice`; returns WAV bytes.
pub async fn speak_bytes(state: &AppState, text: &str, voice: &str, speed: f64) -> ApiResult<Vec<u8>> {
    let text = speakable(text);
    if text.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", "There's nothing to say."));
    }
    let resp = state
        .llm
        .post(voice_url("/tts"))
        .timeout(Duration::from_secs(300))
        .json(&json!({ "text": text, "voice": voice, "speed": speed.clamp(0.5, 2.0) }))
        .send()
        .await
        .map_err(|_| offline())?;
    if !resp.status().is_success() {
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        return Err(engine_failed(v["error"].as_str().unwrap_or("The voice service couldn't speak that.").to_string()));
    }
    Ok(resp.bytes().await.map_err(|e| engine_failed(e.to_string()))?.to_vec())
}

fn clean_voice(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(str::to_string)
}

fn wav(bytes: Vec<u8>) -> ApiResult<Response> {
    Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Couldn't send the audio."))
}

/// Read text aloud, as a WAV file.
#[utoipa::path(post, path = "/api/voice/speak", tag = "voice", security(("bearer" = [])),
    request_body = SpeakRequest,
    responses((status = 200, description = "WAV audio", content_type = "audio/wav"), (status = 503, description = "Voice isn't running", body = ErrorBody)))]
pub async fn speak(State(state): State<AppState>, auth: Auth, Json(req): Json<SpeakRequest>) -> ApiResult<Response> {
    let persona_voice = match &req.persona_id {
        Some(id) => personas::get_usable(&state.pool, &auth.account.id, id).await.ok().and_then(|p| p.voice),
        None => None,
    };
    let voice = clean_voice(req.voice.as_deref())
        .or(persona_voice)
        .unwrap_or_else(|| DEFAULT_VOICE.to_string());
    wav(speak_bytes(&state, &req.text, &voice, req.speed.unwrap_or(1.0)).await?)
}

/// OpenAI-style speech: `{"model", "input", "voice", "speed"}`. Kokoro voice ids work;
/// OpenAI's names (alloy, nova...) get a similar Kokoro voice. Always WAV.
#[utoipa::path(post, path = "/v1/audio/speech", tag = "openai", security(("bearer" = [])),
    request_body(content = Object, description = "OpenAI speech request"),
    responses((status = 200, description = "WAV audio", content_type = "audio/wav")))]
pub async fn openai_speech(State(state): State<AppState>, _auth: Auth, Json(req): Json<Value>) -> ApiResult<Response> {
    let input = req["input"].as_str().unwrap_or_default();
    let asked = req["voice"].as_str().unwrap_or_default();
    let voice = match asked {
        "alloy" | "nova" | "shimmer" | "coral" | "sage" => "af_heart",
        "echo" | "onyx" | "ash" | "verse" => "am_michael",
        "fable" | "ballad" => "bm_george",
        other => other,
    };
    let voice = clean_voice(Some(voice)).unwrap_or_else(|| DEFAULT_VOICE.to_string());
    wav(speak_bytes(&state, input, &voice, req["speed"].as_f64().unwrap_or(1.0)).await?)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct TranscriptionQuery {
    pub language: Option<String>,
}

/// OpenAI-style transcription: multipart with `file` (and optional `language`).
/// Answers `{"text": "..."}`.
#[utoipa::path(post, path = "/v1/audio/transcriptions", tag = "openai", security(("bearer" = [])),
    request_body(content_type = "multipart/form-data", description = "file: the recording; language: optional"),
    responses((status = 200, description = "{\"text\": ...}", body = Object)))]
pub async fn openai_transcriptions(
    State(state): State<AppState>,
    _auth: Auth,
    Query(q): Query<TranscriptionQuery>,
    mut form: Multipart,
) -> ApiResult<Json<Value>> {
    let mut audio = None;
    let mut language = q.language;
    while let Ok(Some(field)) = form.next_field().await {
        match field.name() {
            Some("file") => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid", "The file couldn't be read."))?;
                if bytes.len() > MAX_AUDIO_BYTES {
                    return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", "Recordings can be up to 25 MB."));
                }
                audio = Some(bytes.to_vec());
            }
            Some("language") => language = field.text().await.ok(),
            _ => {}
        }
    }
    let audio = audio.ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid", "Send the recording as \"file\"."))?;
    let t = transcribe_bytes(&state, audio, clean_language(language.as_deref()), "").await?;
    Ok(Json(json!({ "text": t.text, "language": t.language, "duration": t.duration })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_names_read_well() {
        let v = describe_voice("af_heart");
        assert_eq!((v.name.as_str(), v.language.as_str(), v.gender.as_str()), ("Heart", "American English", "female"));
        assert_eq!(describe_voice("bm_george").language, "British English");
        assert_eq!(describe_voice("zf_xiaoxiao").name, "Xiaoxiao");
    }

    #[test]
    fn replies_are_read_without_markup() {
        let said = speakable(
            "# Plan\n**Bold** and _soft_ with [a link](http://x.y).\n```rust\nfn main() {}\n```\n- one\n- two\n\nDrawing Kat: \"x\"\n\n[[outpost-job:123]]",
        );
        assert_eq!(said, "Plan Bold and soft with a link. (code) one two Drawing Kat: \"x\"");
        assert_eq!(speakable("```\nonly code\n```"), "(code)");
    }
}
