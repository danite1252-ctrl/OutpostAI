//! Home Assistant's voice protocol (Wyoming): Outpost offers its speech-to-text (Whisper) and
//! its voices (Kokoro) to Home Assistant on the home network, so a voice device in a room
//! listens and speaks with them.
//!
//! The wire: each event is a JSON line (`type`, and the lengths of what follows), then that
//! many bytes of JSON data merged over the line's own `data`, then that many bytes of payload
//! (raw audio). Home Assistant asks `describe` and gets `info`; for speech it sends
//! `transcribe`, `audio-start`, `audio-chunk`s and `audio-stop`, and gets a `transcript`; for a
//! voice it sends `synthesize` and gets `audio-start`, `audio-chunk`s and `audio-stop`.
//!
//! Wyoming has no sign-in, so it's off until an owner turns it on, and then answers only this
//! home network (or the addresses the owner lists).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use utoipa::ToSchema;

use crate::AppState;

/// The port Home Assistant is told to use, unless the owner picks another.
pub const DEFAULT_PORT: u16 = 10700;
/// Where the settings are kept.
pub const SETTING: &str = "voice.wyoming";
/// The protocol version we speak.
const VERSION: &str = "1.8.0";
/// The mDNS type Home Assistant looks for.
const SERVICE_TYPE: &str = "_wyoming._tcp.local.";

const MAX_LINE: u64 = 64 * 1024;
const MAX_DATA: usize = 1024 * 1024;
const MAX_PAYLOAD: usize = 4 * 1024 * 1024;
/// A connection that says nothing for this long is closed.
const IDLE: Duration = Duration::from_secs(300);
/// Audio sent back per chunk, in samples.
const CHUNK_SAMPLES: usize = 1024;

/// Wyoming's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WyomingSettings {
    /// Offered to Home Assistant.
    pub on: bool,
    pub port: u16,
    /// Only these addresses may connect (empty: anything on this home network).
    #[serde(default)]
    pub allow: Vec<String>,
    /// A persona answering as Home Assistant's conversation agent, as this person (their tools
    /// and rules). None: only listening and voices are offered.
    #[serde(default)]
    pub agent: Option<VoiceAgent>,
}

/// Who answers Home Assistant: a person and one of their personas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VoiceAgent {
    pub account_id: String,
    pub persona_id: String,
}

impl Default for WyomingSettings {
    fn default() -> Self {
        WyomingSettings { on: false, port: DEFAULT_PORT, allow: Vec::new(), agent: None }
    }
}

impl WyomingSettings {
    pub fn check(&self) -> Result<(), String> {
        if self.port < 1024 {
            return Err("Pick a port from 1024 up.".into());
        }
        if self.allow.len() > 20 {
            return Err("That's too many addresses.".into());
        }
        if let Some(bad) = self.allow.iter().find(|a| a.trim().parse::<IpAddr>().is_err()) {
            return Err(format!("\"{bad}\" isn't an address like 192.168.1.20."));
        }
        // A persona acts for someone: only Home Assistant itself may ask (Wyoming has no sign-in).
        if self.agent.is_some() && self.allow.is_empty() {
            return Err("For a persona to answer, put Home Assistant's address in \"Only from\" (Wyoming has no sign-in).".into());
        }
        Ok(())
    }

    /// May this address connect?
    pub fn allows(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
            v4 => v4,
        };
        if !self.allow.is_empty() {
            return self.allow.iter().any(|a| a.trim().parse::<IpAddr>().ok() == Some(ip));
        }
        match ip {
            IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
            IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
        }
    }
}

pub async fn settings(state: &AppState) -> WyomingSettings {
    outpost_core::settings::get(&state.pool, SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

pub async fn save(state: &AppState, s: &WyomingSettings) -> Result<(), String> {
    s.check()?;
    if let Some(a) = &s.agent {
        outpost_core::personas::get_usable(&state.pool, &a.account_id, &a.persona_id)
            .await
            .map_err(|_| "That persona isn't there any more.".to_string())?;
    }
    let clean = WyomingSettings { allow: s.allow.iter().map(|a| a.trim().to_string()).collect(), ..s.clone() };
    outpost_core::settings::set(&state.pool, SETTING, &serde_json::to_string(&clean).map_err(|e| e.to_string())?)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------
// The wire.

/// One Wyoming event.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub kind: String,
    pub data: Map<String, Value>,
    pub payload: Vec<u8>,
}

impl Event {
    pub fn new(kind: &str, data: Value) -> Self {
        Event { kind: kind.into(), data: data.as_object().cloned().unwrap_or_default(), payload: Vec::new() }
    }
    fn with_payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = payload;
        self
    }
    fn str(&self, key: &str) -> Option<&str> {
        self.data.get(key).and_then(Value::as_str)
    }
    fn num(&self, key: &str) -> Option<u64> {
        self.data.get(key).and_then(Value::as_u64)
    }
}

/// Read one event (`None` when the other end closed).
pub async fn read_event<R: AsyncBufRead + Unpin>(r: &mut R) -> std::io::Result<Option<Event>> {
    let bad = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_string());
    let mut line = Vec::new();
    let n = (&mut *r).take(MAX_LINE).read_until(b'\n', &mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        return Err(bad("header too long or cut off"));
    }
    let header: Value = serde_json::from_slice(&line).map_err(|_| bad("header isn't JSON"))?;
    let kind = header.get("type").and_then(Value::as_str).ok_or_else(|| bad("no type"))?.to_string();
    let mut data = header.get("data").and_then(Value::as_object).cloned().unwrap_or_default();
    let data_length = header.get("data_length").and_then(Value::as_u64).unwrap_or(0) as usize;
    if data_length > MAX_DATA {
        return Err(bad("data too long"));
    }
    if data_length > 0 {
        let mut buf = vec![0u8; data_length];
        r.read_exact(&mut buf).await?;
        let more: Value = serde_json::from_slice(&buf).map_err(|_| bad("data isn't JSON"))?;
        if let Some(more) = more.as_object() {
            data.extend(more.clone());
        }
    }
    let payload_length = header.get("payload_length").and_then(Value::as_u64).unwrap_or(0) as usize;
    if payload_length > MAX_PAYLOAD {
        return Err(bad("payload too long"));
    }
    let mut payload = vec![0u8; payload_length];
    if payload_length > 0 {
        r.read_exact(&mut payload).await?;
    }
    Ok(Some(Event { kind, data, payload }))
}

/// An event as bytes on the wire.
pub fn encode(e: &Event) -> Vec<u8> {
    let data = if e.data.is_empty() { Vec::new() } else { serde_json::to_vec(&e.data).unwrap_or_default() };
    let mut header = json!({ "type": e.kind, "version": VERSION });
    if !data.is_empty() {
        header["data_length"] = json!(data.len());
    }
    if !e.payload.is_empty() {
        header["payload_length"] = json!(e.payload.len());
    }
    let mut out = serde_json::to_vec(&header).unwrap_or_default();
    out.push(b'\n');
    out.extend(data);
    out.extend(&e.payload);
    out
}

async fn write<W: AsyncWrite + Unpin>(w: &mut W, e: Event) -> std::io::Result<()> {
    w.write_all(&encode(&e)).await
}

// ---------------------------------------------------------------------------------------
// Audio.

/// Raw audio wrapped as a WAV file.
pub fn wav_from_pcm(pcm: &[u8], rate: u32, width: u16, channels: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    let block = width * channels;
    out.extend(b"RIFF");
    out.extend(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(&16u32.to_le_bytes());
    out.extend(&1u16.to_le_bytes());
    out.extend(&channels.to_le_bytes());
    out.extend(&rate.to_le_bytes());
    out.extend(&(rate * u32::from(block)).to_le_bytes());
    out.extend(&block.to_le_bytes());
    out.extend(&(width * 8).to_le_bytes());
    out.extend(b"data");
    out.extend(&(pcm.len() as u32).to_le_bytes());
    out.extend(pcm);
    out
}

/// A WAV file's format (rate, bytes per sample, channels) and raw audio.
pub fn pcm_from_wav(wav: &[u8]) -> Option<(u32, u16, u16, &[u8])> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut fmt: Option<(u32, u16, u16)> = None;
    while at + 8 <= wav.len() {
        let id = &wav[at..at + 4];
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().ok()?) as usize;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= wav.len() {
            let channels = u16::from_le_bytes(wav[body + 2..body + 4].try_into().ok()?);
            let rate = u32::from_le_bytes(wav[body + 4..body + 8].try_into().ok()?);
            let bits = u16::from_le_bytes(wav[body + 14..body + 16].try_into().ok()?);
            fmt = Some((rate, bits / 8, channels));
        } else if id == b"data" {
            let (rate, width, channels) = fmt?;
            // A streamed WAV may say 0 or too much: take what's there.
            let end = if size == 0 { wav.len() } else { (body + size).min(wav.len()) };
            return Some((rate, width, channels, &wav[body..end]));
        }
        at = body + size + (size & 1);
    }
    None
}

// ---------------------------------------------------------------------------------------
// What we offer.

fn attribution() -> Value {
    json!({ "name": "Outpost", "url": "https://outpost.hartspace.cc" })
}

/// Languages Whisper hears well.
const HEARS: [&str; 20] = ["en", "de", "es", "fr", "it", "nl", "pt", "pl", "sv", "da", "no", "fi", "ru", "uk", "tr", "ja", "zh", "ko", "hi", "ar"];

/// A Kokoro voice's language, as Home Assistant names languages.
fn voice_language(id: &str) -> &'static str {
    match id.chars().next() {
        Some('a') => "en_US",
        Some('b') => "en_GB",
        Some('e') => "es_ES",
        Some('f') => "fr_FR",
        Some('h') => "hi_IN",
        Some('i') => "it_IT",
        Some('j') => "ja_JP",
        Some('p') => "pt_BR",
        Some('z') => "zh_CN",
        _ => "en_US",
    }
}

/// `info`: our listening model, our voices, and the persona answering (if one is).
pub fn info(voices: &[String], agent: Option<&str>) -> Value {
    let version = env!("CARGO_PKG_VERSION");
    let voices: Vec<Value> = voices
        .iter()
        .map(|id| {
            let v = crate::routes::voice::describe_voice(id);
            json!({
                "name": id,
                "description": format!("{} ({}{})", v.name, v.language, if v.gender.is_empty() { String::new() } else { format!(", {}", v.gender) }),
                "attribution": attribution(),
                "installed": true,
                "version": version,
                "languages": [voice_language(id)],
                "speakers": null,
            })
        })
        .collect();
    let tts = if voices.is_empty() {
        json!([])
    } else {
        json!([{
            "name": "outpost-voices",
            "description": "Outpost voices",
            "attribution": attribution(),
            "installed": true,
            "version": version,
            "voices": voices,
            "supports_synthesize_streaming": false,
        }])
    };
    json!({
        "asr": [{
            "name": "outpost-listening",
            "description": "Outpost speech-to-text",
            "attribution": attribution(),
            "installed": true,
            "version": version,
            "models": [{
                "name": "whisper",
                "description": "Whisper, on your Outpost",
                "attribution": attribution(),
                "installed": true,
                "version": version,
                "languages": HEARS,
            }],
            "supports_transcript_streaming": false,
        }],
        "tts": tts,
        "handle": match agent {
            Some(name) => json!([{
                "name": "outpost-agent",
                "description": format!("Outpost ({name})"),
                "attribution": attribution(),
                "installed": true,
                "version": version,
                "models": [{
                    "name": "persona",
                    "description": name,
                    "attribution": attribution(),
                    "installed": true,
                    "version": version,
                    // Any language: the persona answers in the one it's spoken to in.
                    "languages": [],
                }],
                "supports_handled_streaming": false,
                "supports_home_control": true,
            }]),
            None => json!([]),
        },
        "intent": [],
        "wake": [],
        "mic": [],
        "snd": [],
    })
}

fn error(text: &str, code: &str) -> Event {
    Event::new("error", json!({ "text": text, "code": code }))
}

// ---------------------------------------------------------------------------------------
// A connection.

/// What one connection is doing.
#[derive(Default)]
struct Session {
    language: Option<String>,
    format: Option<(u32, u16, u16)>,
    audio: Vec<u8>,
}

/// Answer one connection until it closes.
pub async fn serve<S: tokio::io::AsyncRead + AsyncWrite + Unpin>(state: &AppState, stream: S) -> std::io::Result<()> {
    let (r, mut w) = tokio::io::split(stream);
    let mut r = BufReader::new(r);
    let mut s = Session::default();
    loop {
        let Ok(next) = tokio::time::timeout(IDLE, read_event(&mut r)).await else {
            return Ok(());
        };
        let Some(e) = next? else { return Ok(()) };
        match e.kind.as_str() {
            "describe" => {
                let voices = crate::routes::voice::voice_ids().await;
                let agent = agent_name(state).await;
                write(&mut w, Event::new("info", info(&voices, agent.as_deref()))).await?;
            }
            // What was said to the conversation agent (never sent to speech-to-text).
            "transcript" => {
                let text = e.str("text").unwrap_or_default().trim().to_string();
                // Which device it came from: its own person, else the default.
                let context = e.data.get("context");
                let from = ["satellite_id", "device_id"]
                    .iter()
                    .find_map(|k| context.and_then(|c| c.get(*k)).and_then(Value::as_str).filter(|v| !v.is_empty() && v.len() <= 200))
                    .map(str::to_string);
                let linked = match &from {
                    Some(id) => seen(state, id).await.and_then(|d| d.link),
                    None => None,
                };
                let reply = match linked.or(settings(state).await.agent) {
                    None => Err("Outpost isn't set up to answer here: choose a persona in its Home Assistant voice settings.".to_string()),
                    Some(_) if text.is_empty() => Err("I didn't catch that.".to_string()),
                    Some(agent) => answer(state, &agent, &text, &HOME_ASSISTANT).await,
                };
                match reply {
                    Ok((said, asking)) => {
                        let mut data = json!({ "text": said });
                        if asking {
                            // Keep the device listening for the yes or no.
                            data["continue_conversation"] = json!(true);
                        }
                        if let Some(c) = e.data.get("context") {
                            data["context"] = c.clone();
                        }
                        write(&mut w, Event::new("handled", data)).await?;
                    }
                    Err(why) => write(&mut w, Event::new("not-handled", json!({ "text": why }))).await?,
                }
            }
            "ping" => write(&mut w, Event::new("pong", json!({}))).await?,
            "transcribe" => {
                // "en-US" → "en": what Whisper takes.
                s.language = e.str("language").map(|l| l.split(['-', '_']).next().unwrap_or(l).to_lowercase());
            }
            "audio-start" => {
                s.format = Some((
                    e.num("rate").unwrap_or(16000) as u32,
                    e.num("width").unwrap_or(2) as u16,
                    e.num("channels").unwrap_or(1) as u16,
                ));
                s.audio.clear();
            }
            "audio-chunk" => {
                if s.format.is_none() {
                    s.format = Some((e.num("rate").unwrap_or(16000) as u32, e.num("width").unwrap_or(2) as u16, e.num("channels").unwrap_or(1) as u16));
                }
                if s.audio.len() + e.payload.len() > crate::routes::voice::MAX_AUDIO_BYTES {
                    s.audio.clear();
                    s.format = None;
                    write(&mut w, error("That was too long to listen to.", "too_long")).await?;
                    continue;
                }
                s.audio.extend(&e.payload);
            }
            "audio-stop" => {
                let Some((rate, width, channels)) = s.format.take() else { continue };
                let wav = wav_from_pcm(&std::mem::take(&mut s.audio), rate, width, channels);
                match crate::routes::voice::transcribe_bytes(state, wav, s.language.clone(), "").await {
                    Ok(t) => {
                        let mut data = json!({ "text": t.text });
                        if !t.language.is_empty() {
                            data["language"] = json!(t.language);
                        }
                        write(&mut w, Event::new("transcript", data)).await?;
                    }
                    Err(err) => write(&mut w, error(&err.message, "stt_failed")).await?,
                }
            }
            "synthesize" => {
                let text = e.str("text").unwrap_or_default().to_string();
                let voice = e
                    .data
                    .get("voice")
                    .and_then(|v| v.get("name"))
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(crate::routes::voice::DEFAULT_VOICE)
                    .to_string();
                let text: String = text.chars().take(crate::routes::voice::MAX_SPEAK_CHARS).collect();
                match crate::routes::voice::speak_bytes(state, &text, &voice, 1.0).await {
                    Ok(wav) => match pcm_from_wav(&wav) {
                        Some((rate, width, channels, pcm)) => {
                            let fmt = json!({ "rate": rate, "width": width, "channels": channels });
                            write(&mut w, Event::new("audio-start", fmt.clone())).await?;
                            let step = CHUNK_SAMPLES * usize::from(width.max(1)) * usize::from(channels.max(1));
                            for chunk in pcm.chunks(step) {
                                write(&mut w, Event::new("audio-chunk", fmt.clone()).with_payload(chunk.to_vec())).await?;
                            }
                            write(&mut w, Event::new("audio-stop", json!({}))).await?;
                        }
                        None => write(&mut w, error("The voice came back in a form Outpost couldn't read.", "tts_failed")).await?,
                    },
                    Err(err) => write(&mut w, error(&err.message, "tts_failed")).await?,
                }
            }
            _ => {} // Not ours (wake words, satellites...).
        }
        w.flush().await?;
    }
}

// ---------------------------------------------------------------------------------------
// The conversation agent.

/// How long Home Assistant waits for a spoken reply before being told it's still going.
const ANSWER_WITHIN: Duration = Duration::from_secs(90);

/// Chats with a voice reply running, and when each started.
static REPLYING: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

fn replying(chat_id: &str) -> Option<i64> {
    REPLYING.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|m| m.get(chat_id).copied())
}

fn set_replying(chat_id: &str, started: Option<i64>) {
    let mut g = REPLYING.lock().unwrap_or_else(|p| p.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    match started {
        Some(t) => m.insert(chat_id.to_string(), t),
        None => m.remove(chat_id),
    };
}

/// The persona answering, by name (for Home Assistant's list).
async fn agent_name(state: &AppState) -> Option<String> {
    let a = settings(state).await.agent?;
    let p = outpost_core::personas::get_usable(&state.pool, &a.account_id, &a.persona_id).await.ok()?;
    Some(p.name)
}

/// A spoken yes ("yes", "yeah, go ahead", "do it"). Anything else is a no.
pub fn is_yes(text: &str) -> bool {
    let t: String = text.to_lowercase().chars().map(|c| if c.is_ascii_lowercase() || c == '\'' { c } else { ' ' }).collect();
    let words: Vec<&str> = t.split_whitespace().collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    let phrase = |p: &str| format!(" {} ", words.join(" ")).contains(&format!(" {p} "));
    if has(&["no", "nope", "don't", "dont", "stop", "cancel", "wait", "not"]) {
        return false;
    }
    has(&["yes", "yeah", "yea", "yep", "yup", "ya", "yah", "aye", "sure", "ok", "okay", "please", "correct", "right", "affirmative", "absolutely", "definitely", "confirm", "confirmed"])
        || ["go ahead", "do it", "go for it"].iter().any(|p| phrase(p))
}

/// Where spoken talk is kept: a chat per person and persona, for each way in.
pub struct Place {
    key: &'static str,
    /// The chat's title (and the label its replies run under).
    pub title: &'static str,
}

/// Home Assistant's voice devices.
pub const HOME_ASSISTANT: Place = Place { key: "voice.wyoming.chat", title: "Home Assistant voice" };
/// The phone's wake word.
pub const PHONE: Place = Place { key: "voice.phone.chat", title: "Phone voice" };

/// The chat that talk goes in (one per person and persona, made when first needed).
async fn voice_chat(state: &AppState, agent: &VoiceAgent, place: &Place) -> Result<String, String> {
    let key = format!("{}.{}.{}", place.key, agent.account_id, agent.persona_id);
    if let Some(id) = outpost_core::settings::get(&state.pool, &key).await.ok().flatten() {
        if outpost_core::chats::get(&state.pool, &agent.account_id, &id).await.is_ok() {
            return Ok(id);
        }
    }
    let c = outpost_core::chats::create(&state.pool, &agent.account_id, Some(&agent.persona_id), Some(place.title))
        .await
        .map_err(|e| format!("Couldn't make the voice chat: {e}"))?;
    let _ = outpost_core::settings::set(&state.pool, &key, &c.id).await;
    Ok(c.id)
}

/// Answer something said to a voice device: the reply to speak, and whether it's a question
/// waiting for a yes or no (Allow or Skip, said aloud). The persona works as it does in a typed
/// chat, as the chosen person: their tools, allowed actions, ask-first and tool hours.
pub async fn answer(state: &AppState, agent: &VoiceAgent, text: &str, place: &Place) -> Result<(String, bool), String> {
    let account = outpost_core::accounts::get(&state.pool, &agent.account_id).await.map_err(|_| "Its person isn't there any more.".to_string())?;
    let chat_id = voice_chat(state, agent, place).await?;
    // An answer to the question it asked: yes or no, and the reply carries on.
    let mut answered: Option<String> = None;
    if let Some((id, _)) = state.file_relay.question_in(&chat_id) {
        state.file_relay.answer(&chat_id, &id, json!({ "ok": is_yes(text) }));
        answered = Some(id);
    } else if replying(&chat_id).is_none() {
        let auth = crate::extract::Auth {
            session: outpost_core::sessions::Session {
                id: format!("voice-{}", account.id),
                account_id: account.id.clone(),
                kind: outpost_core::sessions::SessionKind::App,
                label: place.title.into(),
                created_at: outpost_core::now(),
                last_used_at: outpost_core::now(),
            },
            account: account.clone(),
        };
        let clock = crate::llm::UserClock::load(state, &account.id).await;
        let req = crate::routes::chat::SendRequest {
            content: text.chars().take(4000).collect(),
            time_zone: clock.as_ref().and_then(|c| c.zone.clone()),
            utc_offset_minutes: clock.as_ref().map(|c| c.offset_minutes),
            use_pc_tools: true,
            ..Default::default()
        };
        crate::live::take_failure(&chat_id);
        let started = outpost_core::now();
        let mut rx = crate::routes::chat::start_reply(state, &auth, &chat_id, req).await.map_err(|e| e.message)?;
        set_replying(&chat_id, Some(started));
        let id = chat_id.clone();
        tokio::spawn(async move {
            while rx.recv().await.is_some() {}
            set_replying(&id, None);
        });
    }
    let started = replying(&chat_id).unwrap_or_else(outpost_core::now);
    let until = std::time::Instant::now() + ANSWER_WITHIN;
    loop {
        // It needs a yes or no: ask aloud, and keep the device listening.
        if let Some((id, q)) = state.file_relay.question_in(&chat_id) {
            if answered.as_deref() != Some(id.as_str()) {
                return Ok((format!("{} Say yes or no.", crate::routes::voice::speakable(&q)), true));
            }
        }
        if replying(&chat_id).is_none() {
            break;
        }
        if std::time::Instant::now() > until {
            return Ok((format!("That's taking a while. I'll carry on, and the answer will be in the {} chat.", place.title), false));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let messages = outpost_core::chats::messages(&state.pool, &chat_id).await.map_err(|e| e.to_string())?;
    match messages.iter().rev().find(|m| m.role == "assistant" && m.created_at >= started) {
        Some(m) if !m.content.trim().is_empty() => Ok((crate::routes::voice::speakable(&m.content), false)),
        _ => Err(crate::live::take_failure(&chat_id).unwrap_or_else(|| "I couldn't answer that.".into())),
    }
}

// ---------------------------------------------------------------------------------------
// Who's speaking: each voice device linked to a person.

/// Where the voice devices are kept.
const DEVICES: &str = "voice.wyoming.devices";
/// At most this many are remembered (the oldest unlinked go first).
const MAX_DEVICES: usize = 50;

/// A voice device Home Assistant has asked from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VoiceDevice {
    /// Home Assistant's id for it (its satellite's, else its device's).
    pub id: String,
    /// What the owner calls it ("Dan's phone", "Kids' room").
    #[serde(default)]
    pub label: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    /// Who answers it (their tools and rules). None: the default.
    #[serde(default)]
    pub link: Option<VoiceAgent>,
}

pub async fn devices(state: &AppState) -> Vec<VoiceDevice> {
    outpost_core::settings::get(&state.pool, DEVICES)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

async fn save_devices(state: &AppState, list: &[VoiceDevice]) {
    if let Ok(v) = serde_json::to_string(list) {
        let _ = outpost_core::settings::set(&state.pool, DEVICES, &v).await;
    }
}

/// Note that a device spoke (remembering it if it's new); what's known of it.
async fn seen(state: &AppState, id: &str) -> Option<VoiceDevice> {
    let mut list = devices(state).await;
    let now = outpost_core::now();
    let found = remember(&mut list, id, now);
    save_devices(state, &list).await;
    found
}

/// Note a device in the list: seen now, added when new (forgetting the oldest unlinked past
/// [`MAX_DEVICES`]).
fn remember(list: &mut Vec<VoiceDevice>, id: &str, now: i64) -> Option<VoiceDevice> {
    if let Some(d) = list.iter_mut().find(|d| d.id == id) {
        d.last_seen = now;
        return Some(d.clone());
    }
    list.push(VoiceDevice { id: id.to_string(), label: None, first_seen: now, last_seen: now, link: None });
    while list.len() > MAX_DEVICES {
        match list.iter().enumerate().filter(|(_, d)| d.link.is_none()).min_by_key(|(_, d)| d.last_seen).map(|(i, _)| i) {
            Some(i) => {
                list.remove(i);
            }
            None => break,
        }
    }
    list.iter().find(|d| d.id == id).cloned()
}

/// Someone a device can be linked to, and their personas.
#[derive(Debug, Serialize, ToSchema)]
pub struct VoicePerson {
    pub account_id: String,
    pub name: String,
    /// owner, member or restricted.
    pub role: String,
    pub personas: Vec<VoicePersona>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VoicePersona {
    pub id: String,
    pub name: String,
}

/// The voice devices, and who they can be linked to.
#[derive(Debug, Serialize, ToSchema)]
pub struct VoiceDevices {
    pub devices: Vec<VoiceDevice>,
    pub people: Vec<VoicePerson>,
}

async fn devices_now(state: &AppState) -> VoiceDevices {
    let mut list = devices(state).await;
    list.sort_by_key(|d| std::cmp::Reverse(d.last_seen));
    let mut people = Vec::new();
    for a in outpost_core::accounts::list(&state.pool).await.unwrap_or_default() {
        if a.role == outpost_core::accounts::Role::Service {
            continue;
        }
        let mut personas: Vec<VoicePersona> = Vec::new();
        for p in outpost_core::personas::list_existing(&state.pool, &a.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .chain(outpost_core::personas::shared_with_me(&state.pool, &a.id).await.unwrap_or_default())
        {
            if !personas.iter().any(|x| x.id == p.id) {
                personas.push(VoicePersona { id: p.id, name: p.name });
            }
        }
        people.push(VoicePerson { account_id: a.id, name: a.name, role: a.role.as_str().into(), personas });
    }
    VoiceDevices { devices: list, people }
}

/// Voice devices Home Assistant has asked from, and the people they can be linked to.
#[utoipa::path(get, path = "/api/voice/wyoming/devices", tag = "voice", security(("bearer" = [])),
    responses((status = 200, description = "Devices and people", body = VoiceDevices), (status = 403, description = "Owners only", body = crate::error::ErrorBody)))]
pub async fn get_devices(axum::extract::State(state): axum::extract::State<AppState>, auth: crate::extract::Auth) -> crate::error::ApiResult<axum::Json<VoiceDevices>> {
    auth.require_owner()?;
    Ok(axum::Json(devices_now(&state).await))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeviceInput {
    /// What to call it (empty: no name).
    #[serde(default)]
    pub label: Option<String>,
    /// Who answers it. None: the default.
    #[serde(default)]
    pub link: Option<VoiceAgent>,
}

/// Name a voice device, and choose who answers it (their tools and rules apply).
#[utoipa::path(put, path = "/api/voice/wyoming/devices/{id}", tag = "voice", security(("bearer" = [])),
    params(("id" = String, Path, description = "Home Assistant's id for the device")), request_body = DeviceInput,
    responses((status = 200, description = "Saved", body = VoiceDevices), (status = 400, description = "Bad choice", body = crate::error::ErrorBody)))]
pub async fn set_device(
    axum::extract::State(state): axum::extract::State<AppState>,
    auth: crate::extract::Auth,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::Json(input): axum::Json<DeviceInput>,
) -> crate::error::ApiResult<axum::Json<VoiceDevices>> {
    auth.require_owner()?;
    auth.require_person("Linking a voice device")?;
    let bad = |why: &str| crate::error::ApiError::new(axum::http::StatusCode::BAD_REQUEST, "invalid", why.to_string());
    if let Some(l) = &input.link {
        let a = outpost_core::accounts::get(&state.pool, &l.account_id).await.map_err(|_| bad("That person isn't here any more."))?;
        if a.role == outpost_core::accounts::Role::Service {
            return Err(bad("A voice device can't answer as an app account."));
        }
        outpost_core::personas::get_usable(&state.pool, &l.account_id, &l.persona_id)
            .await
            .map_err(|_| bad("That persona isn't one they can use."))?;
    }
    let mut list = devices(&state).await;
    let Some(d) = list.iter_mut().find(|d| d.id == id) else {
        return Err(crate::error::ApiError::new(axum::http::StatusCode::NOT_FOUND, "not_found", "Outpost hasn't heard from that device."));
    };
    d.label = input.label.map(|l| l.trim().chars().take(60).collect::<String>()).filter(|l| !l.is_empty());
    d.link = input.link;
    save_devices(&state, &list).await;
    Ok(axum::Json(devices_now(&state).await))
}

/// Forget a voice device (it's listed again, unlinked, if it speaks again).
#[utoipa::path(delete, path = "/api/voice/wyoming/devices/{id}", tag = "voice", security(("bearer" = [])),
    params(("id" = String, Path, description = "Home Assistant's id for the device")),
    responses((status = 200, description = "Forgotten", body = VoiceDevices)))]
pub async fn forget_device(
    axum::extract::State(state): axum::extract::State<AppState>,
    auth: crate::extract::Auth,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> crate::error::ApiResult<axum::Json<VoiceDevices>> {
    auth.require_owner()?;
    let mut list = devices(&state).await;
    list.retain(|d| d.id != id);
    save_devices(&state, &list).await;
    Ok(axum::Json(devices_now(&state).await))
}

// ---------------------------------------------------------------------------------------
// The listener.

/// The running listener, and its announcement.
struct Running {
    task: tokio::task::JoinHandle<()>,
    announce: Option<mdns_sd::ServiceDaemon>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
/// Why it couldn't start, if it couldn't.
static PROBLEM: Mutex<Option<String>> = Mutex::new(None);

/// Whether it's listening now, and why not if it should be.
pub fn running() -> (bool, Option<String>) {
    let on = RUNNING.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|r| !r.task.is_finished());
    (on, PROBLEM.lock().unwrap_or_else(|p| p.into_inner()).clone())
}

fn stop() {
    if let Some(r) = RUNNING.lock().unwrap_or_else(|p| p.into_inner()).take() {
        r.task.abort();
        if let Some(d) = r.announce {
            let _ = d.shutdown();
        }
    }
}

/// Start (or restart, or stop) as the settings say.
pub async fn apply(state: &AppState) {
    stop();
    *PROBLEM.lock().unwrap_or_else(|p| p.into_inner()) = None;
    let s = settings(state).await;
    if !s.on {
        return;
    }
    let listener = match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], s.port))).await {
        Ok(l) => l,
        Err(e) => {
            let why = format!("Port {} is in use or not allowed ({e}). Pick another.", s.port);
            tracing::warn!(port = s.port, error = %e, "couldn't start the Home Assistant voice service");
            *PROBLEM.lock().unwrap_or_else(|p| p.into_inner()) = Some(why);
            return;
        }
    };
    tracing::info!(port = s.port, "Home Assistant voice service (Wyoming) listening");
    let announce = crate::routes::network::lan_ip().and_then(|ip| announce(ip, s.port).ok());
    let state2 = state.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else { continue };
            if !s.allows(peer.ip()) {
                tracing::info!(%peer, "Wyoming connection refused (not an allowed address)");
                continue;
            }
            let _ = stream.set_nodelay(true);
            let state = state2.clone();
            tokio::spawn(async move {
                if let Err(e) = serve(&state, stream).await {
                    tracing::debug!(%peer, error = %e, "Wyoming connection ended");
                }
            });
        }
    });
    *RUNNING.lock().unwrap_or_else(|p| p.into_inner()) = Some(Running { task, announce });
}

/// Tell Home Assistant it's here (it finds Wyoming services on the network).
fn announce(ip: IpAddr, port: u16) -> anyhow::Result<mdns_sd::ServiceDaemon> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    let name = format!("{} voice", crate::routes::health::machine_name());
    let host = format!("{}-voice.local.", crate::discovery::host_label(&crate::routes::health::machine_name()));
    let info = mdns_sd::ServiceInfo::new(SERVICE_TYPE, &name, &host, ip.to_string().as_str(), port, std::collections::HashMap::<String, String>::new())?;
    daemon.register(info)?;
    Ok(daemon)
}

// ---------------------------------------------------------------------------------------
// Settings, for owners.

/// Wyoming's settings, whether it's running, and the address to give Home Assistant.
#[derive(Debug, Serialize, ToSchema)]
pub struct WyomingStatus {
    #[serde(flatten)]
    pub settings: WyomingSettings,
    /// Listening now.
    pub running: bool,
    /// Why it couldn't start.
    pub problem: Option<String>,
    /// This server's address on the home network, for Home Assistant.
    pub host: Option<String>,
}

async fn status_now(state: &AppState) -> WyomingStatus {
    let (running, problem) = running();
    WyomingStatus { settings: settings(state).await, running, problem, host: crate::routes::network::lan_ip().map(|i| i.to_string()) }
}

/// Home Assistant voice (Wyoming): its settings and whether it's running.
#[utoipa::path(get, path = "/api/voice/wyoming", tag = "voice", security(("bearer" = [])),
    responses((status = 200, description = "Settings and state", body = WyomingStatus), (status = 403, description = "Owners only", body = crate::error::ErrorBody)))]
pub async fn get_settings(axum::extract::State(state): axum::extract::State<AppState>, auth: crate::extract::Auth) -> crate::error::ApiResult<axum::Json<WyomingStatus>> {
    auth.require_owner()?;
    Ok(axum::Json(status_now(&state).await))
}

/// Turn Home Assistant voice on or off, or change its port or allowed addresses.
#[utoipa::path(put, path = "/api/voice/wyoming", tag = "voice", security(("bearer" = [])), request_body = WyomingSettings,
    responses((status = 200, description = "Saved and applied", body = WyomingStatus), (status = 400, description = "Bad settings", body = crate::error::ErrorBody)))]
pub async fn set_settings(
    axum::extract::State(state): axum::extract::State<AppState>,
    auth: crate::extract::Auth,
    axum::Json(input): axum::Json<WyomingSettings>,
) -> crate::error::ApiResult<axum::Json<WyomingStatus>> {
    auth.require_owner()?;
    auth.require_person("Changing Home Assistant voice")?;
    // A persona answers as the owner who chose it.
    let mut input = input;
    if let Some(a) = input.agent.as_mut() {
        a.account_id = auth.account.id.clone();
    }
    save(&state, &input).await.map_err(|why| crate::error::ApiError::new(axum::http::StatusCode::BAD_REQUEST, "invalid", why))?;
    apply(&state).await;
    Ok(axum::Json(status_now(&state).await))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_go_out_and_come_back_the_same() {
        let e = Event::new("audio-chunk", json!({ "rate": 16000, "width": 2, "channels": 1 })).with_payload(vec![1, 2, 3, 4]);
        let bytes = encode(&e);
        let mut r = BufReader::new(&bytes[..]);
        assert_eq!(read_event(&mut r).await.unwrap(), Some(e));
        assert_eq!(read_event(&mut r).await.unwrap(), None, "then the end");
        // Data in the header line is read too, and data after it is merged over it.
        let raw = b"{\"type\":\"transcribe\",\"data\":{\"language\":\"en\",\"name\":\"x\"},\"data_length\":17}\n{\"language\":\"de\"}";
        let e = read_event(&mut BufReader::new(&raw[..])).await.unwrap().unwrap();
        assert_eq!((e.str("language"), e.str("name")), (Some("de"), Some("x")));
        // Rubbish is refused, not trusted.
        assert!(read_event(&mut BufReader::new(&b"not json\n"[..])).await.is_err());
        assert!(read_event(&mut BufReader::new(&b"{\"type\":\"x\",\"payload_length\":999999999}\n"[..])).await.is_err());
    }

    #[test]
    fn audio_goes_into_a_wav_and_back() {
        let pcm: Vec<u8> = (0..200u8).collect();
        let wav = wav_from_pcm(&pcm, 22050, 2, 1);
        let (rate, width, channels, back) = pcm_from_wav(&wav).unwrap();
        assert_eq!((rate, width, channels, back), (22050, 2, 1, &pcm[..]));
        assert!(pcm_from_wav(b"nope").is_none());
    }

    #[test]
    fn only_the_home_network_or_the_listed_addresses_get_in() {
        let open = WyomingSettings { on: true, ..Default::default() };
        assert!(open.allows("192.168.1.50".parse().unwrap()));
        assert!(open.allows("::ffff:10.0.0.5".parse().unwrap()));
        assert!(!open.allows("8.8.8.8".parse().unwrap()));
        let listed = WyomingSettings { allow: vec!["192.168.1.50".into()], ..open };
        assert!(listed.allows("192.168.1.50".parse().unwrap()));
        assert!(!listed.allows("192.168.1.51".parse().unwrap()));
        assert!(WyomingSettings { allow: vec!["ha.local".into()], ..Default::default() }.check().is_err());
        assert!(WyomingSettings { port: 80, ..Default::default() }.check().is_err());
    }

    #[test]
    fn info_lists_listening_and_the_voices() {
        let i = info(&["af_heart".into(), "bm_george".into()], None);
        assert_eq!(i["asr"][0]["models"][0]["installed"], true);
        assert_eq!(i["tts"][0]["voices"][0]["name"], "af_heart");
        assert_eq!(i["tts"][0]["voices"][1]["languages"][0], "en_GB");
        assert!(i["handle"].as_array().unwrap().is_empty());
        assert!(info(&[], None)["tts"].as_array().unwrap().is_empty(), "no voices: none offered");
        let with = info(&[], Some("Assistant"));
        assert_eq!(with["handle"][0]["description"], "Outpost (Assistant)");
        assert_eq!(with["handle"][0]["supports_home_control"], true);
    }

    #[test]
    fn devices_are_remembered_and_the_oldest_unlinked_forgotten() {
        let mut list = Vec::new();
        assert_eq!(remember(&mut list, "phone", 100).unwrap().first_seen, 100);
        assert_eq!(remember(&mut list, "phone", 200).unwrap().last_seen, 200);
        list[0].link = Some(VoiceAgent { account_id: "kid".into(), persona_id: "p".into() });
        for i in 0..MAX_DEVICES {
            remember(&mut list, &format!("d{i}"), 300 + i as i64);
        }
        assert_eq!(list.len(), MAX_DEVICES);
        assert!(list.iter().any(|d| d.id == "phone"), "a linked one is kept");
        assert!(!list.iter().any(|d| d.id == "d0"), "the oldest unlinked goes");
    }

    #[test]
    fn a_spoken_yes_is_a_yes_and_anything_else_a_no() {
        for yes in ["Yes.", "yeah go ahead", "Yep!", "do it", "Okay", "sure, please"] {
            assert!(is_yes(yes), "{yes}");
        }
        for no in ["No.", "no, don't", "wait", "not now", "turn on the lounge lamp", ""] {
            assert!(!is_yes(no), "{no}");
        }
        // A persona answering needs Home Assistant's address.
        let agent = Some(VoiceAgent { account_id: "a".into(), persona_id: "p".into() });
        assert!(WyomingSettings { on: true, agent: agent.clone(), ..Default::default() }.check().is_err());
        assert!(WyomingSettings { on: true, agent, allow: vec!["192.168.1.20".into()], ..Default::default() }.check().is_ok());
    }
}
