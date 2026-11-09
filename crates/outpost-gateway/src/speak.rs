//! Outpost saying things out loud without being asked: a routine's result on Home Assistant
//! speakers, and on the phone (which plays it when the notification arrives).
//!
//! Speakers go through Home Assistant's own text-to-speech (`tts.speak`), so anything Home
//! Assistant can play on works (Google / Nest speakers, Sonos...). When Outpost is Home
//! Assistant's voice service (Settings › Voice › Home Assistant), it speaks in the persona's
//! voice. Home Assistant is reached with the address and token of the person's Home Assistant
//! tool (its MCP server), so there's nothing more to set up.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts::{self, Role};
use outpost_core::activity::{self, NewEntry};
use outpost_core::routines::Routine;
use outpost_core::{personas, tools};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// About as long as a spoken reply goes (a minute or so).
const SPOKEN_MAX: usize = 450;

/// The person's Home Assistant: its address and token, from their Home Assistant tool.
#[derive(Debug, Clone)]
pub struct HomeAssistant {
    /// Its tool's name, for the activity log ("Home Assistant").
    pub name: String,
    pub base: String,
    token: String,
}

/// Home Assistant's own address, from its MCP server's (`http://ha:8123/api/mcp`).
pub fn ha_base(url: &str) -> Option<String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let i = url.find("/api/mcp").or_else(|| url.find("/mcp_server"))?;
    Some(url[..i].trim_end_matches('/').to_string()).filter(|b| b.contains("://") && b.len() > 8)
}

/// The Home Assistant this person may use (theirs or one shared with them), if any.
pub async fn home_assistant(state: &AppState, account_id: &str) -> Option<HomeAssistant> {
    let account = accounts::get(&state.pool, account_id).await.ok()?;
    let restricted = account.role == Role::Restricted;
    let list = tools::usable(&state.pool, account_id, restricted).await.ok()?;
    list.into_iter().filter(|t| t.enabled && t.transport == "http").find_map(|t| {
        let base = ha_base(&t.url)?;
        let secrets = crate::server_tools::secrets(state, &t);
        let auth = secrets.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("authorization")).map(|(_, v)| v.trim().to_string())?;
        let token = auth.strip_prefix("Bearer ").unwrap_or(&auth).trim().to_string();
        (!token.is_empty()).then(|| HomeAssistant { name: t.name.clone(), base, token })
    })
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("Outpost/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("HTTP client")
}

impl HomeAssistant {
    /// Its speakers, by name.
    pub async fn speakers(&self) -> Result<Vec<Speaker>, String> {
        Ok(speakers_in(&self.states().await?))
    }

    /// A camera's picture now (JPEG, as a `data:` URL for the model).
    pub async fn snapshot(&self, camera: &str) -> Result<String, String> {
        use base64::Engine;
        let r = client()
            .get(format!("{}/api/camera_proxy/{camera}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| "Home Assistant didn't answer.".to_string())?;
        match r.status().as_u16() {
            200 => {}
            401 | 403 => return Err("Home Assistant turned down the token in your Home Assistant tool.".into()),
            404 => return Err(format!("Home Assistant has no camera {camera}.")),
            code => return Err(format!("Home Assistant couldn't get the camera's picture ({code}).")),
        }
        let mime = r
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim().to_string())
            .filter(|m| matches!(m.as_str(), "image/jpeg" | "image/png" | "image/webp" | "image/gif"))
            .unwrap_or_else(|| "image/jpeg".into());
        let bytes = r.bytes().await.map_err(|_| "The camera's picture was cut off.".to_string())?;
        if bytes.is_empty() {
            return Err("The camera sent an empty picture.".into());
        }
        if bytes.len() > 10 * 1024 * 1024 {
            return Err("The camera's picture is over 10 MB.".into());
        }
        Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
    }

    /// A picture Home Assistant has from an event (a Nest doorbell's clip preview,
    /// `/api/nest/event_media/<device>/<event>/thumbnail`), as up to three still frames
    /// (`data:` URLs): first, middle and last of an animation.
    pub async fn event_pictures(&self, path: &str) -> Result<Vec<String>, String> {
        let path = path.trim();
        let path = path.strip_prefix(&self.base).unwrap_or(path);
        if !path.starts_with("/api/") || path.contains("..") || path.len() > 500 {
            return Err("That isn't a picture in Home Assistant.".into());
        }
        let r = client()
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| "Home Assistant didn't answer.".to_string())?;
        match r.status().as_u16() {
            200 => {}
            401 | 403 => return Err("Home Assistant turned down the token in your Home Assistant tool.".into()),
            404 => return Err("Home Assistant doesn't have that picture (any more).".into()),
            code => return Err(format!("Home Assistant couldn't get the picture ({code}).")),
        }
        let bytes = r.bytes().await.map_err(|_| "The picture was cut off.".to_string())?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err("The picture is over 20 MB.".into());
        }
        tokio::task::spawn_blocking(move || stills(&bytes)).await.map_err(|_| "The picture couldn't be read.".to_string())?
    }

    async fn states(&self) -> Result<Vec<Value>, String> {
        let r = client()
            .get(format!("{}/api/states", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| "Home Assistant didn't answer.".to_string())?;
        match r.status().as_u16() {
            200 => r.json().await.map_err(|_| "Home Assistant's answer couldn't be read.".to_string()),
            401 | 403 => Err("Home Assistant turned down the token in your Home Assistant tool.".into()),
            code => Err(format!("Home Assistant said no ({code}).")),
        }
    }

    /// Say `text` on `speaker` (a media player's entity id), in `voice` when Outpost is Home
    /// Assistant's voice service.
    async fn say(&self, speaker: &str, text: &str, voice: &str) -> Result<(), String> {
        let states = self.states().await?;
        let (engine, ours) = tts_engine(&states).ok_or(
            "Home Assistant has no text-to-speech. Add Outpost as its voice service (Settings › Devices › Add › Wyoming), or another text-to-speech.",
        )?;
        let mut body = json!({ "entity_id": engine, "media_player_entity_id": speaker, "message": text, "cache": false });
        if ours {
            body["options"] = json!({ "voice": voice });
        }
        let r = client()
            .post(format!("{}/api/services/tts/speak", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|_| "Home Assistant didn't answer.".to_string())?;
        if r.status().is_success() {
            return Ok(());
        }
        let code = r.status().as_u16();
        let why: String = r.text().await.unwrap_or_default().chars().take(200).collect();
        Err(match code {
            401 | 403 => "Home Assistant turned down the token in your Home Assistant tool.".into(),
            _ if why.trim().is_empty() => format!("Home Assistant said no ({code})."),
            _ => format!("Home Assistant said no ({code}): {}", why.trim()),
        })
    }
}

/// Home Assistant's text-to-speech to use: Outpost's own (from its Wyoming service) when it's
/// there, else the first one it has. And whether it's Outpost's.
fn tts_engine(states: &[Value]) -> Option<(String, bool)> {
    let engines: Vec<(&str, String)> = states
        .iter()
        .filter_map(|s| {
            let id = s["entity_id"].as_str()?;
            id.starts_with("tts.").then(|| (id, format!("{id} {}", s["attributes"]["friendly_name"].as_str().unwrap_or("")).to_lowercase()))
        })
        .collect();
    if let Some((id, _)) = engines.iter().find(|(_, words)| words.contains("outpost")) {
        return Some((id.to_string(), true));
    }
    engines.first().map(|(id, _)| (id.to_string(), false))
}

/// A picture as up to three JPEG stills (`data:` URLs): an animation's first, middle and last
/// frames, or the picture itself. Big ones are scaled down to 1280 pixels across.
pub fn stills(bytes: &[u8]) -> Result<Vec<String>, String> {
    use base64::Engine;
    use image::AnimationDecoder;
    let format = image::guess_format(bytes).map_err(|_| "That isn't a picture (a video, maybe: send its thumbnail).".to_string())?;
    let frames: Vec<image::DynamicImage> = if format == image::ImageFormat::Gif {
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| format!("The picture couldn't be read: {e}"))?;
        let all: Vec<image::Frame> = decoder.into_frames().take(400).filter_map(Result::ok).collect();
        if all.is_empty() {
            return Err("The picture has no frames.".into());
        }
        let n = all.len();
        let mut picks = vec![0, n / 2, n - 1];
        picks.dedup();
        picks.into_iter().map(|i| image::DynamicImage::ImageRgba8(all[i].buffer().clone())).collect()
    } else {
        vec![image::load_from_memory_with_format(bytes, format).map_err(|e| format!("The picture couldn't be read: {e}"))?]
    };
    frames
        .into_iter()
        .map(|f| {
            let f = if f.width() > 1280 { f.resize(1280, 1280 * f.height() / f.width().max(1), image::imageops::FilterType::Triangle) } else { f };
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(f.to_rgb8())
                .write_to(&mut out, image::ImageFormat::Jpeg)
                .map_err(|e| format!("The picture couldn't be made into a still: {e}"))?;
            Ok(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(out.into_inner())))
        })
        .collect()
}

/// A speaker Home Assistant can play on.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Speaker {
    /// Its entity id ("media_player.kitchen").
    pub id: String,
    pub name: String,
}

/// Home Assistant's speakers (media players that are there now), by name.
fn speakers_in(states: &[Value]) -> Vec<Speaker> {
    entities_in(states, "media_player.")
}

/// Home Assistant's things of one kind ("camera.") that are there now, by name.
fn entities_in(states: &[Value], kind: &str) -> Vec<Speaker> {
    let mut out: Vec<Speaker> = states
        .iter()
        .filter_map(|s| {
            let id = s["entity_id"].as_str()?;
            if !id.starts_with(kind) || s["state"].as_str() == Some("unavailable") {
                return None;
            }
            let name = s["attributes"]["friendly_name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(id);
            Some(Speaker { id: id.to_string(), name: name.to_string() })
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// Where Outpost can speak for me.
#[derive(Debug, Serialize, ToSchema)]
pub struct Speakers {
    /// Home Assistant's speakers.
    pub speakers: Vec<Speaker>,
    /// Home Assistant's cameras, for a routine to look at.
    pub cameras: Vec<Speaker>,
    /// There's a Home Assistant tool to speak through.
    pub home_assistant: bool,
    /// Why the speakers couldn't be listed.
    pub problem: Option<String>,
}

/// The speakers Outpost can say things on (through my Home Assistant tool).
#[utoipa::path(get, path = "/api/speakers", tag = "routines", security(("bearer" = [])),
    responses((status = 200, description = "Speakers", body = Speakers), (status = 401, description = "Not signed in", body = ErrorBody)))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Speakers>> {
    let Some(ha) = home_assistant(&state, &auth.account.id).await else {
        return Ok(Json(Speakers { speakers: vec![], cameras: vec![], home_assistant: false, problem: None }));
    };
    Ok(Json(match ha.states().await {
        Ok(states) => Speakers {
            speakers: speakers_in(&states),
            cameras: entities_in(&states, "camera."),
            home_assistant: true,
            problem: None,
        },
        Err(e) => Speakers { speakers: vec![], cameras: vec![], home_assistant: true, problem: Some(e) },
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TrySpeaker {
    /// The speaker's entity id.
    pub speaker: String,
    /// Say it in this persona's voice.
    pub persona_id: Option<String>,
}

/// Say a short test line on a speaker.
#[utoipa::path(post, path = "/api/speakers/try", tag = "routines", security(("bearer" = [])), request_body = TrySpeaker,
    responses((status = 204, description = "Said"), (status = 400, description = "Couldn't", body = ErrorBody)))]
pub async fn try_one(State(state): State<AppState>, auth: Auth, Json(input): Json<TrySpeaker>) -> ApiResult<StatusCode> {
    let bad = |m: String| ApiError::new(StatusCode::BAD_REQUEST, "invalid", m);
    outpost_core::routines::clean_say_on(vec![input.speaker.clone()]).map_err(|e| bad(e.to_string()))?;
    let ha = home_assistant(&state, &auth.account.id)
        .await
        .ok_or_else(|| bad("Add Home Assistant as a tool first (Tools).".into()))?;
    let voice = voice_for(&state, &auth.account.id, input.persona_id.as_deref()).await;
    ha.say(&input.speaker, "Hi, this is Outpost. This is where I'll tell you things.", &voice).await.map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn voice_for(state: &AppState, account_id: &str, persona_id: Option<&str>) -> String {
    let persona = match persona_id {
        Some(id) => personas::get_usable(&state.pool, account_id, id).await.ok().and_then(|p| p.voice),
        None => None,
    };
    persona.filter(|v| !v.trim().is_empty()).unwrap_or_else(|| crate::routes::voice::DEFAULT_VOICE.to_string())
}

/// What a routine says out loud: its result, in a minute or less.
pub fn spoken(text: &str) -> String {
    crate::routes::voice::spoken_cap(&crate::routes::voice::speakable(text), SPOKEN_MAX)
}

/// A routine's result, said on its Home Assistant speakers (in the background; each try is in
/// the activity log). Not in the person's tools-off hours.
pub fn routine_said(state: &AppState, r: &Routine, text: &str) {
    let speakers: Vec<String> = r.say_on.iter().filter(|s| s.starts_with("media_player.")).cloned().collect();
    let text = spoken(text);
    if speakers.is_empty() || text.trim().is_empty() {
        return;
    }
    let (state, r) = (state.clone(), r.clone());
    tokio::spawn(async move {
        let off = activity::tools_off(&state.pool, &r.account_id).await.ok().flatten();
        let now = outpost_core::now();
        let in_off_hours = off.as_ref().is_some_and(|q| q.ends(&r.zone(), now).is_some());
        let ha = home_assistant(&state, &r.account_id).await;
        let voice = voice_for(&state, &r.account_id, r.persona_id.as_deref()).await;
        for speaker in speakers {
            let (outcome, result) = match (&ha, in_off_hours) {
                (_, true) => ("off_hours", "Not said: it's in your tools-off hours.".to_string()),
                (None, _) => ("failed", "Not said: there's no Home Assistant tool to speak through.".to_string()),
                (Some(ha), _) => match ha.say(&speaker, &text, &voice).await {
                    Ok(()) => ("done", "Said.".to_string()),
                    Err(e) => {
                        tracing::warn!(routine = %r.id, speaker, error = %e, "couldn't say a routine's result");
                        ("failed", e)
                    }
                },
            };
            activity::record(
                &state.pool,
                NewEntry {
                    account_id: r.account_id.clone(),
                    chat_id: r.chat_id.clone(),
                    routine_id: Some(r.id.clone()),
                    source: "routine".into(),
                    server: ha.as_ref().map(|h| h.name.clone()).unwrap_or_else(|| "Home Assistant".into()),
                    tool: "Say out loud".into(),
                    arguments: json!({ "speaker": speaker, "message": text }).to_string(),
                    outcome: outcome.into(),
                    result,
                },
            )
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_assistants_address_comes_from_its_mcp_server() {
        assert_eq!(ha_base("http://192.168.1.27:8123/api/mcp").as_deref(), Some("http://192.168.1.27:8123"));
        assert_eq!(ha_base("https://ha.example.com/mcp_server/sse").as_deref(), Some("https://ha.example.com"));
        assert_eq!(ha_base("https://mcp.example.com/mcp"), None);
        assert_eq!(ha_base("/api/mcp"), None);
    }

    #[test]
    fn outposts_own_voice_is_used_when_home_assistant_has_it() {
        let states = vec![
            json!({"entity_id": "tts.google_translate_en_com", "attributes": {"friendly_name": "Google Translate"}}),
            json!({"entity_id": "tts.outpost_voices", "attributes": {"friendly_name": "Outpost voices"}}),
            json!({"entity_id": "light.kitchen", "attributes": {}}),
        ];
        assert_eq!(tts_engine(&states), Some(("tts.outpost_voices".into(), true)));
        assert_eq!(tts_engine(&states[..1]), Some(("tts.google_translate_en_com".into(), false)));
        assert_eq!(tts_engine(&states[2..]), None);
    }

    #[test]
    fn speakers_are_media_players_that_are_there() {
        let states = vec![
            json!({"entity_id": "media_player.lounge", "state": "idle", "attributes": {"friendly_name": "Lounge Nest"}}),
            json!({"entity_id": "media_player.kitchen", "state": "off", "attributes": {"friendly_name": "Kitchen speaker"}}),
            json!({"entity_id": "media_player.old", "state": "unavailable", "attributes": {"friendly_name": "Old"}}),
            json!({"entity_id": "light.lounge", "state": "on", "attributes": {}}),
        ];
        let names: Vec<String> = speakers_in(&states).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["Kitchen speaker", "Lounge Nest"]);
    }

    #[test]
    fn a_doorbell_clip_becomes_three_stills() {
        use image::codecs::gif::GifEncoder;
        use image::{Delay, Frame, Rgba, RgbaImage};
        let mut gif = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut gif);
            for shade in [0u8, 60, 120, 180, 240] {
                let img = RgbaImage::from_pixel(8, 6, Rgba([shade, shade, shade, 255]));
                enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(100, 1))).unwrap();
            }
        }
        let stills = stills(&gif).unwrap();
        assert_eq!(stills.len(), 3);
        assert!(stills.iter().all(|s| s.starts_with("data:image/jpeg;base64,")));
        assert!(super::stills(b"not a picture").is_err());
    }
}
