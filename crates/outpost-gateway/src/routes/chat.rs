//! Personas, chats (with streamed replies) and memory. Everything here is scoped to
//! the signed-in account.

use std::convert::Infallible;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Response;
use axum::Json;
use futures_util::{Stream, StreamExt};
use outpost_core::attachments::{self, MAX_IMAGES_PER_MESSAGE};
use outpost_core::chats::{self, Chat, Message};
use outpost_core::memory::{self, Memory};
use outpost_core::personas::{self, Persona, PersonaInput};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::{llm, AppState};

// ---------------------------------------------------------------- personas

/// My personas (every account starts with "Assistant"), then ones others shared with
/// everyone (`owner_name` set: theirs to edit, mine to chat with or copy).
#[utoipa::path(get, path = "/api/personas", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "Personas", body = [Persona]), (status = 401, description = "Not signed in", body = ErrorBody)))]
pub async fn list_personas(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<Persona>>> {
    let mut all = personas::list(&state.pool, &auth.account.id).await?;
    all.extend(personas::shared_with_me(&state.pool, &auth.account.id).await?);
    Ok(Json(all))
}

/// Create a persona.
#[utoipa::path(post, path = "/api/personas", tag = "chat", security(("bearer" = [])),
    request_body = PersonaInput,
    responses((status = 201, description = "Created", body = Persona), (status = 400, description = "Not accepted", body = ErrorBody)))]
pub async fn create_persona(
    State(state): State<AppState>,
    auth: Auth,
    Json(input): Json<PersonaInput>,
) -> ApiResult<(StatusCode, Json<Persona>)> {
    let p = personas::create(&state.pool, &auth.account.id, input).await?;
    Ok((StatusCode::CREATED, Json(p)))
}

/// Change a persona. Its `slug` (and so `persona:<slug>`) never changes.
#[utoipa::path(patch, path = "/api/personas/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Persona id")),
    request_body = PersonaInput,
    responses((status = 200, description = "Updated", body = Persona), (status = 400, description = "Not accepted", body = ErrorBody), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn update_persona(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(input): Json<PersonaInput>,
) -> ApiResult<Json<Persona>> {
    Ok(Json(personas::update(&state.pool, &auth.account.id, &id, input).await?))
}

/// Delete a persona (not the last one). Its chats are kept.
#[utoipa::path(delete, path = "/api/personas/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Persona id")),
    responses((status = 204, description = "Deleted"), (status = 403, description = "Last persona", body = ErrorBody), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_persona(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    personas::delete(&state.pool, &auth.account.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Who else uses this Outpost (names only), e.g. to send someone a persona.
#[utoipa::path(get, path = "/api/people", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "People", body = [super::auth::DirectoryEntry])))]
pub async fn people(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<super::auth::DirectoryEntry>>> {
    auth.require_person("The list of people")?;
    Ok(Json(
        outpost_core::accounts::list(&state.pool)
            .await?
            .into_iter()
            .filter(|a| a.role != outpost_core::accounts::Role::Service)
            .map(|a| super::auth::DirectoryEntry { id: a.id, name: a.name, role: a.role, avatar_color: a.avatar_color })
            .collect(),
    ))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SendPersonaRequest {
    /// Who gets a copy.
    pub account_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SentPersona {
    pub account_name: String,
    /// The copy's name in their list (with "from ..." if they had one called the same).
    pub persona_name: String,
}

/// Send copies of one of my personas: its prompt, model, voice and settings, but none of
/// my memories. Each copy is theirs to change. (`{name}` in the prompt becomes theirs.)
#[utoipa::path(post, path = "/api/personas/{id}/send", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Persona id")),
    request_body = SendPersonaRequest,
    responses((status = 200, description = "Sent", body = [SentPersona]), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn send_persona(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<SendPersonaRequest>,
) -> ApiResult<Json<Vec<SentPersona>>> {
    auth.require_person("Sending a persona")?;
    let p = personas::get(&state.pool, &auth.account.id, &id).await?;
    let mut sent = Vec::new();
    for target in req.account_ids.iter().filter(|t| **t != auth.account.id) {
        let account = outpost_core::accounts::get(&state.pool, target).await?;
        if account.role == outpost_core::accounts::Role::Service {
            continue;
        }
        let copy = copy_persona(&state, &p, &account.id, &auth.account.name).await?;
        outpost_core::audit::log_quiet(
            &state.pool,
            Some(&auth.account.id),
            Some(&auth.session.id),
            "persona_sent",
            Some(&format!("{} to {}", p.name, account.name)),
        )
        .await;
        sent.push(SentPersona { account_name: account.name, persona_name: copy.name });
    }
    Ok(Json(sent))
}

/// A copy of `p` in `account_id`'s personas: settings and prompt, no memories, not shared.
/// "(from <who>)" is added when they already have one with that name.
async fn copy_persona(state: &AppState, p: &Persona, account_id: &str, from: &str) -> ApiResult<Persona> {
    let theirs = personas::list(&state.pool, account_id).await?;
    let taken = |n: &str| theirs.iter().any(|q| q.name.eq_ignore_ascii_case(n));
    let mut name = p.name.clone();
    if taken(&name) {
        name = format!("{} (from {from})", p.name);
        let mut n = 2;
        while taken(&name) {
            name = format!("{} (from {from}) {n}", p.name);
            n += 1;
        }
    }
    Ok(personas::create(
        &state.pool,
        account_id,
        personas::PersonaInput {
            name: Some(name),
            system_prompt: Some(p.system_prompt.clone()),
            model: p.model.clone(),
            temperature: p.temperature,
            think: Some(p.think),
            use_memory: Some(p.use_memory),
            voice: p.voice.clone(),
            speak_replies: Some(p.speak_replies),
            shared: Some(false),
            avatar_color: Some(p.avatar_color.clone()),
            slug: None,
        },
    )
    .await?)
}

/// Make my own copy of a persona someone shared (or of one of mine), to change as I like.
#[utoipa::path(post, path = "/api/personas/{id}/copy", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Persona id")),
    responses((status = 201, description = "My copy", body = Persona), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn copy_persona_for_me(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Persona>)> {
    let p = personas::get_usable(&state.pool, &auth.account.id, &id).await?;
    let from = p.owner_name.clone().unwrap_or_else(|| auth.account.name.clone());
    let copy = copy_persona(&state, &p, &auth.account.id, &from).await?;
    Ok((StatusCode::CREATED, Json(copy)))
}

// ---------------------------------------------------------------- chats

/// My chats, newest first.
#[utoipa::path(get, path = "/api/chats", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "Chats", body = [Chat])))]
pub async fn list_chats(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<Chat>>> {
    Ok(Json(chats::list(&state.pool, &auth.account.id).await?))
}

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct NewChatRequest {
    /// Persona to talk to; defaults to your first persona.
    pub persona_id: Option<String>,
    pub title: Option<String>,
}

/// Start a chat.
#[utoipa::path(post, path = "/api/chats", tag = "chat", security(("bearer" = [])),
    request_body = NewChatRequest,
    responses((status = 201, description = "Created", body = Chat), (status = 404, description = "No such persona", body = ErrorBody)))]
pub async fn create_chat(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<NewChatRequest>,
) -> ApiResult<(StatusCode, Json<Chat>)> {
    let persona_id = match req.persona_id {
        Some(id) => id,
        None => personas::list(&state.pool, &auth.account.id)
            .await?
            .into_iter()
            .next()
            .map(|p| p.id)
            .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "No persona found."))?,
    };
    let chat = chats::create(&state.pool, &auth.account.id, Some(&persona_id), req.title.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(chat)))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChatWithMessages {
    pub chat: Chat,
    pub messages: Vec<Message>,
}

/// A chat and all of its messages.
#[utoipa::path(get, path = "/api/chats/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    responses((status = 200, description = "Chat", body = ChatWithMessages), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_chat(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<ChatWithMessages>> {
    let chat = chats::get(&state.pool, &auth.account.id, &id).await?;
    let messages = chats::messages(&state.pool, &chat.id).await?;
    Ok(Json(ChatWithMessages { chat, messages }))
}

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdateChatRequest {
    pub title: Option<String>,
    pub persona_id: Option<String>,
    /// Model override for this chat; empty string to clear.
    pub model: Option<String>,
}

/// Rename a chat, switch its persona, or override its model.
#[utoipa::path(patch, path = "/api/chats/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    request_body = UpdateChatRequest,
    responses((status = 200, description = "Updated", body = Chat), (status = 400, description = "Not accepted", body = ErrorBody), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn update_chat(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<UpdateChatRequest>,
) -> ApiResult<Json<Chat>> {
    Ok(Json(
        chats::update(
            &state.pool,
            &auth.account.id,
            &id,
            req.title.as_deref(),
            req.persona_id.as_deref(),
            req.model.as_deref(),
        )
        .await?,
    ))
}

/// Delete a chat and its messages.
#[utoipa::path(delete, path = "/api/chats/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_chat(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let chat = chats::get(&state.pool, &auth.account.id, &id).await?;
    let files = attachments::paths_for_chat(&state.pool, &state.config.files_dir(), &chat.id).await?;
    // Deleting a chat forgets what was learned in it.
    memory::forget_chat(&state.pool, &auth.account.id, &chat.id).await?;
    chats::delete(&state.pool, &auth.account.id, &chat.id).await?;
    attachments::remove_files(&files).await;
    Ok(StatusCode::NO_CONTENT)
}

/// One of my attached images, as the image itself.
#[utoipa::path(get, path = "/api/files/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Attachment id")),
    responses((status = 200, description = "The image (PNG, JPEG, WebP or GIF)"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_file(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    let (a, path) = attachments::get(&state.pool, &state.config.files_dir(), &auth.account.id, &id).await?;
    let data = attachments::read(&path).await?;
    Response::builder()
        .header(header::CONTENT_TYPE, a.mime)
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(data))
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Could not send the file.",
            )
        })
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SendRequest {
    #[serde(default)]
    pub content: String,
    /// Up to 4 images (PNG, JPEG, WebP or GIF, 10 MB each) as base64 or
    /// `data:image/...;base64,` URLs. The chat's model must have vision.
    #[serde(default)]
    pub images: Vec<String>,
    /// The app's clock: minutes ahead of UTC (Sydney in summer: 660), so the model knows
    /// the user's date and time. Remembered for the account.
    #[serde(default)]
    pub utc_offset_minutes: Option<i32>,
    /// Its time zone's name, e.g. "Australia/Sydney".
    #[serde(default)]
    pub time_zone: Option<String>,
}

#[derive(Deserialize)]
struct OllamaLine {
    #[serde(default)]
    message: Option<OllamaMessage>,
    #[serde(default)]
    done: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct OllamaMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    thinking: Option<String>,
}

fn event(name: &str, data: serde_json::Value) -> Event {
    Event::default().event(name).data(data.to_string())
}

/// Send a message and stream the reply as Server-Sent Events:
/// `start` (the saved user message and model), `thinking` and `token` chunks
/// (`{"text": ...}`), then `done` (the saved reply) or `error` (`{"message": ...}`).
/// The reply is saved even if the client disconnects mid-stream.
#[utoipa::path(post, path = "/api/chats/{id}/messages", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    request_body = SendRequest,
    responses(
        (status = 200, description = "Event stream", body = String, content_type = "text/event-stream"),
        (status = 400, description = "Empty message, bad image, no model, or a model without vision", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
        (status = 503, description = "Ollama isn't running", body = ErrorBody),
    ))]
pub async fn send_message(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<SendRequest>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let content = req.content.trim().to_string();
    if content.is_empty() && req.images.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid",
            "Write a message or attach an image first.",
        ));
    }
    if req.images.len() > MAX_IMAGES_PER_MESSAGE {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid",
            format!("Attach up to {MAX_IMAGES_PER_MESSAGE} images at a time."),
        ));
    }
    // Check every image before anything is saved.
    let images = req
        .images
        .iter()
        .map(|i| attachments::decode_image(i))
        .collect::<Result<Vec<_>, _>>()?;
    let account_id = auth.account.id.clone();
    let chat = chats::get(&state.pool, &account_id, &id).await?;
    // "/draw ..." (or "draw <a character> ...") makes a picture instead of a reply.
    let my_characters = outpost_core::training::characters(&state.pool, Some(account_id.as_str())).await?;
    if let Some(prompt) = draw_request(&content, &my_characters) {
        let prompt = prompt.as_str();
        if !images.is_empty() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid",
                "Send /draw without pictures. To change a picture, use Create › Edit.",
            ));
        }
        let rx = draw(&state, &auth, &chat, &content, prompt).await?;
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<Event, Infallible>);
        return Ok(Sse::new(stream).keep_alive(KeepAlive::default()));
    }
    let persona = match &chat.persona_id {
        Some(pid) => personas::get_usable(&state.pool, &account_id, pid).await.ok(),
        None => None,
    };
    let resolved = llm::resolve_model(&state, chat.model.as_deref(), persona.as_ref()).await?;
    if !images.is_empty() && !resolved.can_see {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "model_no_vision",
            format!(
                "{} can't see images. Pick a model with vision (the eye badge) for this persona, or send text only.",
                resolved.model
            ),
        ));
    }
    let model = resolved.model.clone();
    if let Some(clock) = llm::UserClock::from_app(req.utc_offset_minutes, req.time_zone.as_deref()) {
        clock.save(&state, &account_id).await;
    }
    // Chat has the graphics card for now (describing gallery pictures waits).
    crate::gallery_index::chat_active();
    let mut system = llm::system_prompt(&state, &account_id, persona.as_ref(), Some(&content)).await?;

    // First message names the chat.
    let earlier = chats::messages(&state.pool, &chat.id).await?;
    // Passages from the chat's (and its persona's) documents that fit this message.
    let previous = earlier.iter().rev().find(|m| m.role == "user").map(|m| m.content.clone());
    let docs = crate::documents::context(&state, &chat.id, persona.as_ref(), &content, previous.as_deref()).await;
    if let Some(d) = &docs {
        system = format!("{system}\n\n{}", d.section);
    }
    let doc_sources = docs.map(|d| d.sources).unwrap_or_default();
    if earlier.is_empty() && chat.title == "New chat" {
        let title = if content.is_empty() {
            "Image".to_string()
        } else {
            chats::title_from(&content)
        };
        let _ = chats::update(&state.pool, &account_id, &chat.id, Some(title.as_str()), None, None).await;
    }
    let mut user_msg = chats::add_message(&state.pool, &chat.id, "user", &content, None, None).await?;
    let files_dir = state.config.files_dir();
    for image in &images {
        match attachments::save_image(&state.pool, &files_dir, &account_id, &user_msg.id, image).await {
            Ok(a) => user_msg.attachments.push(a),
            Err(e) => {
                // Undo the half-sent message so a retry doesn't duplicate it.
                let saved = attachments::paths_for_message(&state.pool, &files_dir, &user_msg.id)
                    .await
                    .unwrap_or_default();
                let _ = chats::delete_message(&state.pool, &chat.id, &user_msg.id).await;
                attachments::remove_files(&saved).await;
                return Err(e.into());
            }
        }
    }
    let mut history = earlier;
    history.push(user_msg.clone());
    // Earlier images go back too when the model can see (newest first, capped).
    let history_images = if resolved.can_see {
        llm::history_images(&state, &account_id, &history).await
    } else {
        Default::default()
    };
    // The reply's request is built in the task below: a long chat may first need its
    // oldest messages summarised (one extra call), and the user shouldn't wait for "start".
    let body_state = state.clone();
    let body_persona = persona.clone();

    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(256);
    let url = format!("{}/api/chat", state.config.ollama_url.trim_end_matches('/'));
    let client = state.llm.clone();
    let pool = state.pool.clone();
    let chat_id = chat.id.clone();
    // For learning memories once the reply is done.
    let learn_state = state.clone();
    let learn_persona = persona.clone();
    let learn_account = account_id.clone();
    let user_text = content.clone();
    let user_message_id = user_msg.id.clone();
    let can_think = resolved.can_think;
    let model_for_body = model.clone();
    let _ = tx
        .send(event(
            "start",
            serde_json::json!({ "user_message": user_msg, "model": model }),
        ))
        .await;

    tokio::spawn(async move {
        let (summary, recent) =
            llm::fit_history(&body_state, &chat_id, &model_for_body, can_think, &system, history).await;
        let body = llm::ollama_chat_body(
            &model_for_body,
            &system,
            summary.as_deref(),
            &recent,
            &history_images,
            can_think,
            body_persona.as_ref(),
        );
        let mut content = String::new();
        let mut thinking = String::new();
        let mut failure: Option<String> = None;
        match client.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                let mut stream = resp.bytes_stream();
                let mut buf: Vec<u8> = Vec::new();
                'read: while let Some(chunk) = stream.next().await {
                    let Ok(chunk) = chunk else {
                        failure = Some("The model stopped responding.".into());
                        break;
                    };
                    buf.extend_from_slice(&chunk);
                    while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=pos).collect();
                        let Ok(update) = serde_json::from_slice::<OllamaLine>(&line) else {
                            continue;
                        };
                        if let Some(err) = update.error {
                            failure = Some(err);
                            break 'read;
                        }
                        if let Some(m) = update.message {
                            if let Some(t) = m.thinking.filter(|t| !t.is_empty()) {
                                thinking.push_str(&t);
                                // A closed client just stops receiving; keep collecting the reply.
                                let _ = tx.send(event("thinking", serde_json::json!({ "text": t }))).await;
                            }
                            if !m.content.is_empty() {
                                content.push_str(&m.content);
                                let _ = tx.send(event("token", serde_json::json!({ "text": m.content }))).await;
                            }
                        }
                        if update.done {
                            break 'read;
                        }
                    }
                }
            }
            Ok(resp) => {
                let text = resp.text().await.unwrap_or_default();
                failure = Some(format!("Ollama refused the request: {text}"));
            }
            Err(_) => failure = Some("Ollama isn't running. Start it in Settings → Engines.".into()),
        }

        if content.is_empty() && failure.is_some() {
            let message = failure.unwrap_or_default();
            let _ = tx.send(event("error", serde_json::json!({ "message": message }))).await;
            return;
        }
        let thinking = Some(thinking.as_str()).filter(|t| !t.is_empty());
        let saved = chats::add_message(&pool, &chat_id, "assistant", &content, thinking, Some(model.as_str())).await;
        match saved {
            Ok(mut saved) => {
                if !doc_sources.is_empty() {
                    saved.sources = crate::documents::cited(&content, &doc_sources);
                    let _ = outpost_core::documents::set_message_sources(&pool, &saved.id, &saved.sources).await;
                }
                let complete = failure.is_none();
                let _ = tx
                    .send(event("done", serde_json::json!({ "message": saved, "error": failure })))
                    .await;
                // Close the stream first: learning happens after the user has the reply.
                drop(tx);
                if complete {
                    crate::memory::learn(
                        learn_state,
                        crate::memory::Exchange {
                            account_id: learn_account,
                            persona: learn_persona,
                            chat_id,
                            user_message_id,
                            model,
                            can_think,
                            user_text,
                            reply: content,
                        },
                    )
                    .await;
                }
            }
            Err(e) => {
                let _ = tx
                    .send(event(
                        "error",
                        serde_json::json!({ "message": format!("Could not save the reply: {e}") }),
                    ))
                    .await;
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<Event, Infallible>);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RemovedMessages {
    /// The messages deleted: the one asked for, and the reply to it if it was yours.
    pub removed: Vec<String>,
    /// Memories learned from them that were forgotten.
    pub forgotten: u64,
}

/// Delete a message from one of my chats. A message I sent takes its reply with it,
/// and anything the persona learned from it is forgotten. Pictures attached go too.
#[utoipa::path(delete, path = "/api/chats/{id}/messages/{message_id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id"), ("message_id" = String, Path, description = "Message id")),
    responses((status = 200, description = "Deleted", body = RemovedMessages), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_message(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, message_id)): Path<(String, String)>,
) -> ApiResult<Json<RemovedMessages>> {
    let account_id = &auth.account.id;
    chats::get(&state.pool, account_id, &id).await?;
    let files_dir = state.config.files_dir();
    // Find the files before their rows go.
    let mut files = attachments::paths_for_message(&state.pool, &files_dir, &message_id).await?;
    let removed = chats::remove_message(&state.pool, account_id, &id, &message_id).await?;
    for other in removed.iter().skip(1) {
        files.extend(attachments::paths_for_message(&state.pool, &files_dir, other).await.unwrap_or_default());
    }
    attachments::remove_files(&files).await;
    let forgotten = memory::forget_messages(&state.pool, account_id, &removed).await?;
    Ok(Json(RemovedMessages { removed, forgotten }))
}

// ---------------------------------------------------------------- drawing from chat

/// Whether a message asks for a picture, and if so what to draw.
///
/// - `/draw ...` (or "slash draw ..." when spoken) always draws.
/// - "draw me ...", "draw a ...", "draw a picture of ...", "make a picture of ...",
///   "picture of ..." draw anything.
/// - "draw ..." alone draws when it names one of your characters ("draw Kat at the beach"),
///   so "draw conclusions" stays a chat message.
///
/// Speech is allowed for: a polite lead-in ("can you", "please", "hey Kat,") is skipped, a
/// full stop at the end is dropped, and a character's name heard as a sound-alike word
/// ("Cat" for "Kat") is put back.
pub fn draw_request(content: &str, characters: &[outpost_core::training::Character]) -> Option<String> {
    let mut text = content.trim().trim_end_matches(['.', '!', '?']).trim().to_string();
    // Spoken lead-ins, possibly several ("okay, can you please ...").
    const LEADS: [&str; 14] = [
        "okay", "ok", "hey", "hi", "please", "can you", "could you", "would you", "will you", "i want you to",
        "i'd like you to", "go ahead and", "now", "and",
    ];
    loop {
        let lower = text.to_lowercase();
        let Some(lead) = LEADS.iter().find(|l| {
            lower.starts_with(*l) && lower[l.len()..].starts_with(|c: char| c == ' ' || c == ',')
        }) else {
            break;
        };
        text = text.get(lead.len()..).unwrap_or("").trim_start_matches([' ', ',']).to_string();
    }
    let lower = text.to_lowercase();
    for slash in ["/draw", "slash draw", "slash-draw"] {
        if lower == slash || lower.starts_with(&format!("{slash} ")) {
            return Some(with_names(text.get(slash.len()..).unwrap_or("").trim(), characters));
        }
    }
    for lead in [
        "draw me a picture of ",
        "draw a picture of ",
        "draw me ",
        "make me a picture of ",
        "make a picture of ",
        "picture of ",
        "draw a ",
        "draw an ",
    ] {
        if lower.starts_with(lead) {
            // "draw a dog" keeps its "a"; "draw me a dog" and "picture of a dog" don't need to.
            let keep = if lead == "draw a " || lead == "draw an " { "draw ".len() } else { lead.len() };
            let rest = text.get(keep..).unwrap_or("").trim();
            if !rest.is_empty() {
                return Some(with_names(rest, characters));
            }
        }
    }
    if lower.starts_with("draw ") {
        let rest = with_names(text.get("draw ".len()..).unwrap_or("").trim(), characters);
        if outpost_core::training::mentioned(&rest, characters).is_some() {
            return Some(rest);
        }
    }
    None
}

/// Put back character names that speech recognition wrote as a sound-alike word
/// ("Cat" for "Kat", "Kaht"…): compares how the words sound, roughly.
fn with_names(text: &str, characters: &[outpost_core::training::Character]) -> String {
    fn sound(word: &str) -> String {
        let w = word.to_lowercase();
        let w = w.replace("ph", "f").replace("ck", "k").replace("qu", "kw").replace("ah", "a");
        let mut out = String::new();
        for ch in w.chars().filter(|c| c.is_alphanumeric()) {
            let ch = match ch {
                'c' | 'q' => 'k',
                'z' => 's',
                'y' => 'i',
                other => other,
            };
            // Doubled letters sound like one ("Katt").
            if out.ends_with(ch) {
                continue;
            }
            out.push(ch);
        }
        out
    }
    if outpost_core::training::mentioned(text, characters).is_some() {
        return text.to_string();
    }
    let names: Vec<(String, &str)> = characters
        .iter()
        .filter(|c| c.name.chars().count() >= 2 && !c.name.contains(' '))
        .map(|c| (sound(&c.name), c.name.as_str()))
        .collect();
    let mut out: Vec<String> = Vec::new();
    for word in text.split(' ') {
        let core = word.trim_matches(|c: char| !c.is_alphanumeric());
        // "a cat" is any cat; "Cat at the beach" (a name, no article) is the character.
        let article = out
            .last()
            .is_some_and(|w| ["a", "an", "the", "my", "your", "some", "this", "that"].contains(&w.to_lowercase().as_str()));
        let replaced = names
            .iter()
            .find(|(key, _)| !article && !core.is_empty() && sound(core) == *key)
            .map(|(_, name)| word.replace(core, name));
        out.push(replaced.unwrap_or_else(|| word.to_string()));
    }
    out.join(" ")
}

/// Most pictures one chat message may ask for.
pub const MAX_DRAWINGS: usize = 8;

/// The pictures one draw message asks for. Scenes go on separate lines or between
/// `;` ("Kat at the beach; in the snow"), and a count asks for several of each, with
/// different seeds: "4 pictures of Kat at the beach", "Kat at the beach x4". At most
/// [`MAX_DRAWINGS`] in all.
pub fn plan_drawings(prompt: &str) -> Vec<String> {
    const WORDS: [&str; 8] = ["one", "two", "three", "four", "five", "six", "seven", "eight"];
    const NOUNS: [&str; 12] = [
        "pictures", "picture", "photos", "photo", "images", "image", "pics", "pic", "versions", "variations",
        "drawings", "shots",
    ];
    let number = |w: &str| -> Option<usize> {
        let w = w.to_lowercase();
        w.parse::<usize>().ok().or_else(|| WORDS.iter().position(|x| *x == w).map(|i| i + 1))
    };
    let mut text = prompt.trim().to_string();
    let mut count = 1;
    // Leading "4 pictures of ..." / "three photos of ...".
    let words: Vec<&str> = text.split_whitespace().collect();
    if let (Some(n), Some(noun)) = (words.first().and_then(|w| number(w)), words.get(1)) {
        if NOUNS.contains(&noun.to_lowercase().as_str()) && (1..=MAX_DRAWINGS).contains(&n) {
            let skip = if words.get(2).is_some_and(|w| w.eq_ignore_ascii_case("of")) { 3 } else { 2 };
            count = n;
            text = words[skip..].join(" ");
        }
    }
    // Trailing "x4" / "×4".
    if let Some((rest, last)) = text.rsplit_once(char::is_whitespace) {
        let n = last
            .strip_prefix(['x', 'X', '×'])
            .and_then(|d| d.parse::<usize>().ok())
            .filter(|n| (1..=MAX_DRAWINGS).contains(n));
        if let Some(n) = n {
            count = n;
            text = rest.trim().to_string();
        }
    }
    let scenes: Vec<String> = text
        .split(['\n', ';'])
        .map(|line| {
            let t = line.trim().trim_start_matches(['-', '*', '•']).trim_start();
            // "1. at the beach" / "2) in the snow"
            let t = match t.split_once(['.', ')']) {
                Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => rest.trim_start(),
                _ => t,
            };
            t.trim().trim_end_matches(['.', ',']).to_string()
        })
        .filter(|t| !t.is_empty())
        .collect();
    scenes
        .iter()
        .flat_map(|s| std::iter::repeat(s.clone()).take(count))
        .take(MAX_DRAWINGS)
        .collect()
}

/// One picture to queue: the workflow, its inputs, and who or what it shows.
pub struct PlannedPicture {
    pub pack: outpost_engines::packs::Pack,
    pub inputs: serde_json::Map<String, serde_json::Value>,
    /// The character's name, or "that".
    pub what: String,
}

impl PlannedPicture {
    /// The exact prompt it sends.
    pub fn prompt(&self) -> &str {
        self.inputs.get("prompt").and_then(serde_json::Value::as_str).unwrap_or_default()
    }
}

/// Plan the pictures a draw request asks for (see [`plan_drawings`]), with the right
/// workflow for each: a named character uses its LoRA, anything else Outpost's pick of
/// the picture workflows. Checks everything before anything is queued.
pub async fn plan_pictures(state: &AppState, account_id: &str, prompt: &str) -> ApiResult<Vec<PlannedPicture>> {
    use outpost_engines::packs;
    let characters = outpost_core::training::characters(&state.pool, Some(account_id)).await?;
    let characters = &characters[..];
    let scenes = plan_drawings(prompt);
    if scenes.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid",
            "Say what to draw, e.g. \"/draw a lighthouse at dusk\".",
        ));
    }
    let waiting = outpost_core::jobs::waiting_for(&state.pool, account_id).await?;
    let room = (outpost_core::jobs::MAX_QUEUED_PER_ACCOUNT - waiting).max(0) as usize;
    if scenes.len() > room {
        return Err(ApiError::conflict(format!(
            "You have {waiting} pictures waiting already, so there's room for {room} more. Let some finish first."
        )));
    }
    let loras = crate::routes::create::visible_loras(state, account_id).await;
    // The picture workflow for scenes without a character: Outpost's pick among ready ones.
    let picture_pack = {
        let hw = crate::routes::models::measure(state).await;
        let ready: Vec<packs::Pack> = packs::built_in()
            .into_iter()
            .filter(|p| p.task() == packs::Task::Picture && state.assets.status(p).ready)
            .collect();
        packs::pick(&ready, packs::Task::Picture, |p| crate::routes::create::pack_fit(p.vram_gb, &hw))
            .or(ready.first())
            .cloned()
    };
    // The same for characters (a full-size and an 8 GB version share the LoRAs).
    let character_pack = {
        let hw = crate::routes::models::measure(state).await;
        let ready: Vec<packs::Pack> = packs::built_in()
            .into_iter()
            .filter(|p| p.task() == packs::Task::Character && state.assets.status(p).ready)
            .collect();
        packs::pick(&ready, packs::Task::Character, |p| crate::routes::create::pack_fit(p.vram_gb, &hw))
            .or(ready.first())
            .cloned()
    };
    // Plan every picture first, so a problem stops them all before any is queued.
    let mut planned: Vec<PlannedPicture> = Vec::new();
    let mut carried: Option<outpost_core::training::Character> = None;
    for scene in &scenes {
        // A later scene without a name keeps the character of the one before.
        let named = outpost_core::training::mentioned(scene, characters).cloned();
        let character = named.or_else(|| carried.clone());
        let (pack, inputs, what) = if let Some(mut c) = character {
            // Older characters learn what they are from their captions now.
            outpost_core::training::fill_subject(&state.pool, &mut c).await?;
            let pack = character_pack
                .clone()
                .ok_or_else(|| {
                    ApiError::conflict("Z-Image Turbo (for characters) isn't downloaded yet. Get it in Create › Characters first.")
                })?;
            if !loras.contains(&c.lora_file) {
                return Err(ApiError::conflict(format!(
                    "{}'s LoRA ({}) is missing from the LoRA folder.",
                    c.name, c.lora_file
                )));
            }
            let inputs = serde_json::json!({
                "prompt": outpost_core::training::with_trigger(scene, &c),
                "lora": c.lora_file,
            });
            let what = c.name.clone();
            carried = Some(c);
            (pack, inputs, what)
        } else {
            let pack = picture_pack
                .clone()
                .ok_or_else(|| ApiError::conflict("No picture workflow is downloaded yet. Get one in Create › Picture first."))?;
            (pack, serde_json::json!({ "prompt": scene }), "that".to_string())
        };
        let inputs = inputs.as_object().cloned().unwrap_or_default();
        pack.build(&inputs, || 0, &std::collections::HashMap::new(), &loras)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid", e.to_string()))?;
        planned.push(PlannedPicture { pack, inputs, what });
    }
    Ok(planned)
}

/// Queue planned pictures; returns their job ids in order.
pub async fn queue_pictures(
    state: &AppState,
    account_id: &str,
    priority: i64,
    planned: &[PlannedPicture],
) -> ApiResult<Vec<String>> {
    let mut ids = Vec::new();
    for p in planned {
        let job = outpost_core::jobs::create(
            &state.pool,
            account_id,
            &p.pack.id,
            &serde_json::Value::Object(p.inputs.clone()),
            priority,
        )
        .await?;
        ids.push(job.id);
    }
    state.queue.wake.notify_one();
    Ok(ids)
}


/// Queue pictures for a chat message and answer with a card per picture that the
/// apps fill in as they draw: `[[outpost-job:<id>]]`.
async fn draw(
    state: &AppState,
    auth: &Auth,
    chat: &Chat,
    content: &str,
    prompt: &str,
) -> ApiResult<tokio::sync::mpsc::Receiver<Event>> {
    let planned = plan_pictures(state, &auth.account.id, prompt).await?;
    let ids = queue_pictures(state, &auth.account.id, outpost_core::jobs::priority_for(auth.session.kind), &planned).await?;
    let lines: Vec<String> = planned.iter().map(|p| p.prompt().to_string()).collect();
    let cards: Vec<String> = ids.iter().map(|id| format!("[[outpost-job:{id}]]")).collect();

    if chat.title == "New chat" && chats::messages(&state.pool, &chat.id).await?.is_empty() {
        let title = chats::title_from(prompt);
        let _ = chats::update(&state.pool, &auth.account.id, &chat.id, Some(title.as_str()), None, None).await;
    }
    let user_msg = chats::add_message(&state.pool, &chat.id, "user", content, None, None).await?;
    let first_what = &planned[0].what;
    let pack_name = &planned[0].pack.name;
    let reply = if planned.len() == 1 {
        format!("Drawing {first_what} with {pack_name}: \"{}\"\n\n{}", lines[0], cards[0])
    } else {
        let mut distinct = lines.clone();
        distinct.dedup();
        let list = if distinct.len() == 1 {
            format!("\"{}\"", distinct[0])
        } else {
            distinct.iter().enumerate().map(|(i, l)| format!("{}. \"{l}\"", i + 1)).collect::<Vec<_>>().join("\n")
        };
        format!(
            "Drawing {} pictures of {first_what} with {pack_name}:\n{list}\n\n{}",
            planned.len(),
            cards.join("\n")
        )
    };
    let saved = chats::add_message(&state.pool, &chat.id, "assistant", &reply, None, Some(pack_name.as_str())).await?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(4);
    let _ = tx
        .send(event("start", serde_json::json!({ "user_message": user_msg, "model": pack_name })))
        .await;
    let _ = tx.send(event("token", serde_json::json!({ "text": reply }))).await;
    let _ = tx
        .send(event("done", serde_json::json!({ "message": saved, "error": null })))
        .await;
    Ok(rx)
}

#[cfg(test)]
mod draw_tests {
    use super::{draw_request, plan_drawings};
    use outpost_core::training::Character;

    fn kat() -> Vec<Character> {
        vec![Character {
            id: "c1".into(),
            account_id: "a1".into(),
            name: "Kat".into(),
            trigger_word: "zk4t".into(),
            lora_file: "kat.safetensors".into(),
            training_id: None,
            step: None,
            created_at: 0,
            subject: Some("woman".into()),
            style: None,
        }]
    }

    #[test]
    fn spoken_draw_requests_are_understood() {
        let c = kat();
        let d = |t: &str| draw_request(t, &c);
        assert_eq!(d("/draw a lighthouse").as_deref(), Some("a lighthouse"));
        assert_eq!(d("Slash draw a lighthouse.").as_deref(), Some("a lighthouse"));
        // Whisper writes the name as it sounds.
        assert_eq!(d("Draw Cat at the beach.").as_deref(), Some("Kat at the beach"));
        assert_eq!(d("Can you please draw Kat at the beach?").as_deref(), Some("Kat at the beach"));
        assert_eq!(d("Okay, draw me a dog in the snow").as_deref(), Some("a dog in the snow"));
        assert_eq!(d("Draw a picture of a fox").as_deref(), Some("a fox"));
        // An article means any cat, not the character.
        assert_eq!(d("Draw a cat at the beach").as_deref(), Some("a cat at the beach"));
        assert_eq!(d("draw zk4t smiling").as_deref(), Some("zk4t smiling"));
        // Ordinary chat stays chat.
        assert_eq!(d("draw conclusions from this"), None);
        assert_eq!(d("How do I draw better?"), None);
        assert_eq!(d("Hello, can you hear me?"), None);
    }

    #[test]
    fn one_message_can_ask_for_several_pictures() {
        assert_eq!(plan_drawings("Kat at the beach"), vec!["Kat at the beach"]);
        assert_eq!(plan_drawings("4 pictures of Kat at the beach").len(), 4);
        assert_eq!(plan_drawings("three photos of Kat")[2], "Kat");
        assert_eq!(plan_drawings("Kat at the beach x3"), vec!["Kat at the beach"; 3]);
        assert_eq!(
            plan_drawings("Kat at the beach; in the snow\n- reading in a café."),
            vec!["Kat at the beach", "in the snow", "reading in a café"]
        );
        assert_eq!(plan_drawings("1. a fox\n2) a hen").len(), 2);
        assert_eq!(plan_drawings("2 pictures of Kat at the beach; in the snow").len(), 4);
        assert_eq!(plan_drawings("20 pictures of a cat").len(), 1, "too many isn't a count");
        assert_eq!(plan_drawings("a; b; c; d; e x3").len(), 8, "capped");
        // Numbers that aren't counts stay in the prompt.
        assert_eq!(plan_drawings("2 cats on a sofa"), vec!["2 cats on a sofa"]);
        assert_eq!(plan_drawings("Kat and her dog at the beach"), vec!["Kat and her dog at the beach"]);
        assert!(plan_drawings("  ").is_empty());
    }
}

// ---------------------------------------------------------------- memory

/// Everything I've asked Outpost to remember.
#[utoipa::path(get, path = "/api/memory", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "Memories", body = [Memory])))]
pub async fn list_memory(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<Memory>>> {
    Ok(Json(memory::list(&state.pool, &auth.account.id).await?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NewMemoryRequest {
    pub text: String,
    /// Only this persona sees it; leave out for all personas.
    pub persona_id: Option<String>,
}

/// Remember something. Personas with memory on include it in every chat.
#[utoipa::path(post, path = "/api/memory", tag = "chat", security(("bearer" = [])),
    request_body = NewMemoryRequest,
    responses((status = 201, description = "Saved", body = Memory), (status = 400, description = "Not accepted", body = ErrorBody)))]
pub async fn add_memory(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<NewMemoryRequest>,
) -> ApiResult<(StatusCode, Json<Memory>)> {
    let m = memory::add(&state.pool, &auth.account.id, req.persona_id.as_deref(), &req.text).await?;
    Ok((StatusCode::CREATED, Json(m)))
}

/// Forget something.
#[utoipa::path(delete, path = "/api/memory/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Memory id")),
    responses((status = 204, description = "Forgotten"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete_memory(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    memory::delete(&state.pool, &auth.account.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MemorySettings {
    /// Learn memories from my chats automatically.
    pub auto: bool,
    /// The embedding model used to recall relevant memories, if one is installed.
    /// Without one, the newest learned memories are used.
    pub embedding_model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MemorySettingsRequest {
    pub auto: bool,
}

/// My memory settings.
#[utoipa::path(get, path = "/api/memory/settings", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "Settings", body = MemorySettings)))]
pub async fn memory_settings(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<MemorySettings>> {
    Ok(Json(MemorySettings {
        auto: memory::auto_enabled(&state.pool, &auth.account.id).await?,
        embedding_model: crate::memory::embedding_model(&state).await,
    }))
}

/// Turn automatic memory on or off for my account.
#[utoipa::path(put, path = "/api/memory/settings", tag = "chat", security(("bearer" = [])),
    request_body = MemorySettingsRequest,
    responses((status = 200, description = "Saved", body = MemorySettings)))]
pub async fn set_memory_settings(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<MemorySettingsRequest>,
) -> ApiResult<Json<MemorySettings>> {
    memory::set_auto_enabled(&state.pool, &auth.account.id, req.auto).await?;
    Ok(Json(MemorySettings {
        auto: req.auto,
        embedding_model: crate::memory::embedding_model(&state).await,
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Cleared {
    pub removed: u64,
}

/// Forget everything learned automatically. Memories I added myself stay.
#[utoipa::path(delete, path = "/api/memory/auto", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "Cleared", body = Cleared)))]
pub async fn clear_auto_memory(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Cleared>> {
    Ok(Json(Cleared {
        removed: memory::clear_auto(&state.pool, &auth.account.id).await?,
    }))
}
