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

/// A spoken reply, no longer than `max` characters: whole sentences, cut at the last one that
/// fits (a voice device shouldn't read a page out).
pub fn spoken_cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    match cut.rfind(['.', '!', '?']) {
        Some(i) if i > max / 3 => cut[..=i].to_string(),
        _ => format!("{}…", cut.trim_end()),
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
    /// The listening model chosen (speech to text).
    pub listening: String,
    /// The one used now: the chosen one once it's downloaded, else `small`.
    pub listening_in_use: String,
    /// The listening models to choose from.
    pub listening_models: Vec<outpost_engines::ListeningModel>,
    /// A listening model downloading now, or the last failed download.
    pub listening_fetch: outpost_engines::ListeningFetch,
}

/// The listening model chosen (valid, else the default).
async fn chosen_listening(state: &AppState) -> String {
    outpost_core::settings::get(&state.pool, outpost_core::settings::VOICE_LISTENING)
        .await
        .ok()
        .flatten()
        .filter(|m| state.engines.listening_models().iter().any(|l| &l.id == m))
        .unwrap_or_else(|| outpost_engines::DEFAULT_LISTENING.to_string())
}

/// The listening model to use now: the chosen one once it's downloaded.
pub async fn listening_in_use(state: &AppState) -> String {
    let chosen = chosen_listening(state).await;
    if state.engines.listening_downloaded(&chosen) {
        chosen
    } else {
        outpost_engines::DEFAULT_LISTENING.to_string()
    }
}

/// The voice service's voices (none when it isn't answering).
pub async fn voice_ids() -> Vec<String> {
    let Ok(client) = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(60)).build() else {
        return Vec::new();
    };
    let Ok(r) = client.get(voice_url("/voices")).send().await else {
        return Vec::new();
    };
    let v: Value = r.json().await.unwrap_or(Value::Null);
    v["voices"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
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
    let mut voices: Vec<VoiceInfo> = Vec::new();
    if matches!(st, EngineState::Running | EngineState::External) {
        voices = voice_ids().await.iter().map(|v| describe_voice(v)).collect();
    }
    // English first, then by language and name.
    voices.sort_by_key(|v| (!v.language.ends_with("English"), v.language.clone(), v.name.clone()));
    Json(VoiceStatus {
        ready: !voices.is_empty(),
        state: st,
        voices,
        default_voice: DEFAULT_VOICE.into(),
        listening: chosen_listening(&state).await,
        listening_in_use: listening_in_use(&state).await,
        listening_models: state.engines.listening_models(),
        listening_fetch: state.engines.listening_fetch(),
    })
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListeningRequest {
    /// `base`, `small` or `large-v3-turbo`.
    pub model: String,
}

/// Choose the listening model (owner). One that isn't downloaded yet is downloaded first
/// (follow it with `GET /api/voice`); until then the current one carries on.
#[utoipa::path(put, path = "/api/voice/listening", tag = "voice", security(("bearer" = [])),
    request_body = ListeningRequest,
    responses((status = 200, description = "Chosen", body = VoiceStatus), (status = 400, description = "Unknown model", body = ErrorBody),
        (status = 409, description = "Voice isn't installed, or another is downloading", body = ErrorBody)))]
pub async fn set_listening(State(state): State<AppState>, auth: Auth, Json(req): Json<ListeningRequest>) -> ApiResult<Json<VoiceStatus>> {
    auth.require_owner()?;
    if !state.engines.listening_models().iter().any(|l| l.id == req.model) {
        return Err(ApiError::bad_request(format!("There's no listening model called {}.", req.model)));
    }
    if !state.engines.listening_downloaded(&req.model) {
        let installed = state
            .engines
            .status()
            .await
            .engines
            .iter()
            // Outpost's own install (not another copy answering on its port): models go in its folder.
            .any(|e| e.id == EngineId::Voice && e.managed && !matches!(e.state, EngineState::NotInstalled | EngineState::Installing));
        if !installed {
            return Err(ApiError::conflict("Install Voice in Settings › Engines first."));
        }
        state.engines.fetch_listening(&req.model, || {}).map_err(|_| {
            ApiError::conflict("Another listening model is downloading; try again when it's done.")
        })?;
    }
    outpost_core::settings::set(&state.pool, outpost_core::settings::VOICE_LISTENING, &req.model).await?;
    Ok(status(State(state), auth).await)
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

pub(crate) fn decode_audio(encoded: &str) -> Result<Vec<u8>, ApiError> {
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

pub(crate) fn clean_language(l: Option<&str>) -> Option<String> {
    l.map(str::trim)
        .filter(|l| (2..=8).contains(&l.len()) && l.chars().all(|c| c.is_ascii_alphabetic() || c == '-'))
        .map(|l| l.to_ascii_lowercase())
}

/// Send audio to the voice service.
pub async fn transcribe_bytes(state: &AppState, audio: Vec<u8>, language: Option<String>, hints: &str) -> ApiResult<Transcript> {
    if audio.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid", "The recording is empty."));
    }
    let mut query: Vec<(&str, String)> = vec![("model", listening_in_use(state).await)];
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
pub(crate) async fn name_hints(state: &AppState, account_id: &str) -> String {
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
        // Tables (Markdown or tab-separated) aren't read out.
        if in_code || t.starts_with("[[outpost-job:") || t.starts_with('|') || t.matches('\t').count() >= 2 || t.matches(" | ").count() >= 2 {
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

/// One spoken turn from the phone's wake word: what was said after "Hey Jarvis".
#[derive(Debug, Deserialize, ToSchema)]
pub struct TurnRequest {
    /// The recording (a WAV), as base64.
    pub audio: String,
    /// The persona that answers.
    pub persona_id: String,
    #[serde(default)]
    pub language: Option<String>,
}

/// What to say back.
#[derive(Debug, Serialize, ToSchema)]
pub struct TurnReply {
    /// What was heard ("" when it was nothing: go back to listening for the wake word).
    pub heard: String,
    /// The reply, as text.
    pub reply: String,
    /// It asked a yes-or-no question: listen for the answer.
    pub asking: bool,
    /// The reply spoken in the persona's voice (a WAV, base64), when voice is installed.
    pub audio: Option<String>,
}

/// Whisper's noise words ("you", "Thank you for watching"), or nothing at all.
pub fn is_speech(text: &str) -> bool {
    let t = text.trim().trim_end_matches(['.', '!', '?']).to_lowercase();
    let letters = t.chars().filter(|c| c.is_alphanumeric()).count();
    letters >= 2 && !["you", "thank you for watching", "thanks for watching", "thank you"].contains(&t.as_str())
}

/// The phone's wake word: hear what was said, have the persona answer it (as in a chat: its
/// tools and your rules; anything that asks first is asked aloud), and speak the answer.
#[utoipa::path(post, path = "/api/voice/turn", tag = "voice", security(("bearer" = [])), request_body = TurnRequest,
    responses((status = 200, description = "What to say back", body = TurnReply), (status = 400, description = "Unreadable audio", body = ErrorBody)))]
pub async fn turn(State(state): State<AppState>, auth: Auth, Json(req): Json<TurnRequest>) -> ApiResult<Json<TurnReply>> {
    auth.require_person("Talking by wake word")?;
    let audio = decode_audio(&req.audio)?;
    let persona = personas::get_usable(&state.pool, &auth.account.id, &req.persona_id).await?;
    let hints = name_hints(&state, &auth.account.id).await;
    let heard = transcribe_bytes(&state, audio, clean_language(req.language.as_deref()), &hints).await?.text.trim().to_string();
    if !is_speech(&heard) {
        return Ok(Json(TurnReply { heard: String::new(), reply: String::new(), asking: false, audio: None }));
    }
    let agent = crate::wyoming::VoiceAgent { account_id: auth.account.id.clone(), persona_id: persona.id.clone() };
    let (reply, asking) = crate::wyoming::answer(&state, &agent, &heard, &crate::wyoming::PHONE)
        .await
        .map_err(|why| ApiError::new(StatusCode::BAD_GATEWAY, "reply_failed", why))?;
    let voice = persona.voice.clone().filter(|v| !v.is_empty()).unwrap_or_else(|| DEFAULT_VOICE.to_string());
    let said: String = reply.chars().take(MAX_SPEAK_CHARS).collect();
    let audio = speak_bytes(&state, &said, &voice, 1.0).await.ok().map(|w| base64::engine::general_purpose::STANDARD.encode(w));
    Ok(Json(TurnReply { heard, reply, asking, audio }))
}

#[cfg(test)]
mod turn_tests {
    #[test]
    fn spoken_replies_are_whole_sentences_and_skip_tables() {
        let long = "It'll be 22 degrees and dry. ".repeat(40);
        let cut = super::spoken_cap(&long, 200);
        assert!(cut.len() <= 200 && cut.ends_with('.'), "{cut}");
        assert_eq!(super::spoken_cap("Short.", 200), "Short.");
        let table = "Sydney today: 22 and dry.\nStation\tTemp\tWind\nObservatory Hill\t18.9\tS 28\n| a | b | c |";
        assert_eq!(super::speakable(table), "Sydney today: 22 and dry.");
    }

    #[test]
    fn noise_words_are_not_speech() {
        assert!(super::is_speech("Turn off the kitchen lamp."));
        assert!(super::is_speech("yes"));
        assert!(!super::is_speech("you"));
        assert!(!super::is_speech("Thank you for watching!"));
        assert!(!super::is_speech("  ."));
    }
}

/// Did they really say the wake word? The two seconds the phone heard it in.
#[derive(Debug, Deserialize, ToSchema)]
pub struct HeardRequest {
    /// The audio (a WAV), as base64.
    pub audio: String,
    /// The wake word ("Hey Jarvis").
    pub word: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeardReply {
    /// It was said: go ahead (beep and listen).
    pub heard: bool,
    /// What Whisper heard.
    pub text: String,
}

/// The wake word's key word ("jarvis" of "Hey Jarvis") in what was heard, allowing for
/// Whisper's spelling ("Jarvis," "Jervis").
pub fn said_word(text: &str, word: &str) -> bool {
    let clean = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let Some(key) = clean(word).into_iter().filter(|w| w != "hey" && w != "ok" && w != "okay").last() else {
        return false;
    };
    let room = if key.chars().count() >= 6 { 2 } else { 1 };
    clean(text).iter().any(|w| w.contains(&key) || distance(w, &key) <= room)
}

/// Edits between two words (Levenshtein).
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = (row[j + 1] + 1).min(row[j] + 1).min(prev + usize::from(ca != cb));
            prev = here;
        }
    }
    row[b.len()]
}

/// The phone thinks it heard its wake word: check what was said (TV and talk can sound like
/// it to the phone's small detector), before it beeps.
#[utoipa::path(post, path = "/api/voice/heard", tag = "voice", security(("bearer" = [])), request_body = HeardRequest,
    responses((status = 200, description = "Whether the word was said", body = HeardReply), (status = 400, description = "Unreadable audio", body = ErrorBody)))]
pub async fn heard(State(state): State<AppState>, auth: Auth, Json(req): Json<HeardRequest>) -> ApiResult<Json<HeardReply>> {
    auth.require_person("Checking the wake word")?;
    let audio = decode_audio(&req.audio)?;
    // No hints: a hint can make Whisper "hear" the very word it's told to listen for.
    let text = transcribe_bytes(&state, audio, None, "").await?.text.trim().to_string();
    Ok(Json(HeardReply { heard: said_word(&text, &req.word), text }))
}

#[cfg(test)]
mod heard_tests {
    use super::said_word;

    #[test]
    fn the_wake_word_is_heard_allowing_for_spelling() {
        assert!(said_word("Hey Jarvis.", "Hey Jarvis"));
        assert!(said_word("hey jervis turn on the lamp", "Hey Jarvis"));
        assert!(said_word("Alexa?", "Alexa"));
        assert!(said_word("Hey, Mycroft.", "Hey Mycroft"));
        assert!(!said_word("and here's the weather for the weekend", "Hey Jarvis"));
        assert!(!said_word("Hey there!", "Hey Jarvis"));
        assert!(!said_word("", "Hey Jarvis"));
        assert!(!said_word("I'll text you", "Alexa"));
    }
}
