//! Documents to chat with: my library, and which chats and personas use which documents.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use outpost_core::documents::{self, Document};
use outpost_core::{chats, personas};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// My documents, newest first.
#[utoipa::path(get, path = "/api/documents", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "Documents", body = [Document])))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<Document>>> {
    Ok(Json(documents::list(&state.pool, &auth.account.id).await?))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct AddQuery {
    /// The file's name, for its type: .pdf, .docx, .txt, .md, .csv or .html.
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Added {
    pub document: Document,
    /// False when the same file was added before (that one is returned).
    pub added: bool,
}

/// Add a document (the file as the body). It's read in the background: poll
/// `GET /api/documents` until its status is `ready` (or `failed`, with why).
#[utoipa::path(post, path = "/api/documents", tag = "chat", security(("bearer" = [])),
    params(AddQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "The document"),
    responses((status = 201, description = "Added", body = Added), (status = 400, description = "Not a document Outpost reads", body = ErrorBody)))]
pub async fn add(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<AddQuery>,
    body: Body,
) -> ApiResult<(StatusCode, Json<Added>)> {
    let data = axum::body::to_bytes(body, documents::MAX_BYTES)
        .await
        .map_err(|_| ApiError::bad_request(format!("Documents can be up to {} MB.", documents::MAX_BYTES / 1024 / 1024)))?;
    let (document, added) = documents::add(&state.pool, &state.config.files_dir(), &auth.account.id, &q.name, &data).await?;
    if added {
        tokio::spawn(crate::documents::read(state.clone(), document.id.clone()));
    }
    Ok((if added { StatusCode::CREATED } else { StatusCode::OK }, Json(Added { document, added })))
}

/// One of my documents' files.
#[utoipa::path(get, path = "/api/documents/{id}/file", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Document id")),
    responses((status = 200, description = "The file"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn file(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    let (d, path) = documents::get(&state.pool, &state.config.files_dir(), &auth.account.id, &id).await?;
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "That file is missing."))?;
    let safe: String = d.name.chars().map(|c| if c.is_ascii_alphanumeric() || ".-_ ".contains(c) { c } else { '_' }).collect();
    Response::builder()
        .header(header::CONTENT_TYPE, d.mime)
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{safe}\""))
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(Body::from(data))
        .map_err(|_| ApiError::conflict("Could not send the file."))
}

/// Delete one of my documents (it leaves every chat and persona that used it).
#[utoipa::path(delete, path = "/api/documents/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Document id")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    documents::delete(&state.pool, &state.config.files_dir(), &auth.account.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct Attached {
    /// Document ids (my own), all of them: this replaces the list.
    pub document_ids: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatDocuments {
    /// The chat's own documents.
    pub documents: Vec<Document>,
    /// Its persona's, used too.
    pub persona_documents: Vec<Document>,
}

/// The documents a chat uses: its own, and its persona's.
#[utoipa::path(get, path = "/api/chats/{id}/documents", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    responses((status = 200, description = "Documents", body = ChatDocuments)))]
pub async fn for_chat(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<ChatDocuments>> {
    let chat = chats::get(&state.pool, &auth.account.id, &id).await?;
    let persona_documents = match &chat.persona_id {
        Some(p) if personas::get_usable(&state.pool, &auth.account.id, p).await.is_ok() => {
            documents::for_persona(&state.pool, p).await?
        }
        _ => Vec::new(),
    };
    Ok(Json(ChatDocuments { documents: documents::for_chat(&state.pool, &chat.id).await?, persona_documents }))
}

/// Set which of my documents a chat uses.
#[utoipa::path(put, path = "/api/chats/{id}/documents", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    request_body = Attached,
    responses((status = 200, description = "Documents", body = ChatDocuments), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn set_for_chat(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<Attached>,
) -> ApiResult<Json<ChatDocuments>> {
    let chat = chats::get(&state.pool, &auth.account.id, &id).await?;
    documents::set_for_chat(&state.pool, &auth.account.id, &chat.id, &req.document_ids).await?;
    for_chat(State(state), auth, Path(id)).await
}

/// The documents a persona uses (in every chat with it, including other people's, if it's shared).
#[utoipa::path(get, path = "/api/personas/{id}/documents", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Persona id")),
    responses((status = 200, description = "Documents", body = [Document])))]
pub async fn for_persona(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<Document>>> {
    let p = personas::get_usable(&state.pool, &auth.account.id, &id).await?;
    Ok(Json(documents::for_persona(&state.pool, &p.id).await?))
}

/// Set which of my documents one of my personas uses.
#[utoipa::path(put, path = "/api/personas/{id}/documents", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Persona id")),
    request_body = Attached,
    responses((status = 200, description = "Documents", body = [Document]), (status = 404, description = "Not mine", body = ErrorBody)))]
pub async fn set_for_persona(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(req): Json<Attached>,
) -> ApiResult<Json<Vec<Document>>> {
    let p = personas::get(&state.pool, &auth.account.id, &id).await?;
    documents::set_for_persona(&state.pool, &auth.account.id, &p.id, &req.document_ids).await?;
    Ok(Json(documents::for_persona(&state.pool, &p.id).await?))
}
