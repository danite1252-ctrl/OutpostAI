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

/// My personas. Every account starts with "Assistant".
#[utoipa::path(get, path = "/api/personas", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "Personas", body = [Persona]), (status = 401, description = "Not signed in", body = ErrorBody)))]
pub async fn list_personas(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<Persona>>> {
    Ok(Json(personas::list(&state.pool, &auth.account.id).await?))
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
    let persona = match &chat.persona_id {
        Some(pid) => personas::get(&state.pool, &account_id, pid).await.ok(),
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
    let system = llm::system_prompt(&state, &account_id, persona.as_ref(), Some(&content)).await?;

    // First message names the chat.
    let earlier = chats::messages(&state.pool, &chat.id).await?;
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
    let body = llm::ollama_chat_body(
        &model,
        &system,
        &history,
        &history_images,
        resolved.can_think,
        persona.as_ref(),
    );

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
    let can_think = resolved.can_think;
    let _ = tx
        .send(event(
            "start",
            serde_json::json!({ "user_message": user_msg, "model": model }),
        ))
        .await;

    tokio::spawn(async move {
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
            Ok(saved) => {
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
