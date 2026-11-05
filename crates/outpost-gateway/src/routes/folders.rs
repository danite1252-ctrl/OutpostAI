//! Files in chats, the apps' side: server folders (make, upload into, download, delete), the
//! folders a chat uses and whether it asks before changing files, the desktop app's answers
//! for PC folders, and Apply / Skip / Undo for changes.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use outpost_core::chat_files::{self, ChatFolder, FileChange, Folder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::workspace;
use crate::AppState;

/// Largest file uploaded into a server folder.
pub const MAX_UPLOAD: usize = 50 * 1024 * 1024;

fn root(state: &AppState, account_id: &str, id: &str) -> std::path::PathBuf {
    chat_files::folder_dir(&state.config.files_dir(), account_id, id)
}

fn inner(path: &str) -> ApiResult<std::path::PathBuf> {
    let p = chat_files::safe_path(path).ok_or_else(|| ApiError::bad_request("That path leaves the folder."))?;
    if p.as_os_str().is_empty() {
        return Err(ApiError::bad_request("Give a file's path."));
    }
    Ok(p)
}

// ---------------------------------------------------------------- server folders

/// A server folder, with how much space it takes and how many chats use it.
#[derive(Debug, Serialize, ToSchema)]
pub struct FolderInfo {
    #[serde(flatten)]
    pub folder: Folder,
    /// Its files' total size.
    pub bytes: u64,
    /// Chats it's added to.
    pub chats: i64,
}

/// Total size of the files under a folder (not following links).
pub fn size_of(dir: &std::path::Path) -> u64 {
    let mut total = 0;
    let mut look = vec![dir.to_path_buf()];
    while let Some(d) = look.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                look.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

async fn info(state: &AppState, f: Folder) -> ApiResult<FolderInfo> {
    let chats: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_folders WHERE kind = 'server' AND ref = ?")
        .bind(&f.id)
        .fetch_one(&state.pool)
        .await
        .map_err(outpost_core::CoreError::from)?;
    let dir = root(state, &f.account_id, &f.id);
    let bytes = tokio::task::spawn_blocking(move || size_of(&dir)).await.unwrap_or(0);
    Ok(FolderInfo { folder: f, bytes, chats })
}

/// My server folders, with their sizes and how many chats use each.
#[utoipa::path(get, path = "/api/folders", tag = "files", security(("bearer" = [])),
    responses((status = 200, description = "Folders", body = [FolderInfo])))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<FolderInfo>>> {
    let mut out = Vec::new();
    for f in chat_files::folders(&state.pool, &auth.account.id).await? {
        out.push(info(&state, f).await?);
    }
    Ok(Json(out))
}

/// The server folders only this chat uses (what deleting the chat could clean up too).
#[utoipa::path(get, path = "/api/chats/{id}/own-folders", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    responses((status = 200, description = "Folders", body = [FolderInfo])))]
pub async fn own_folders(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<FolderInfo>>> {
    outpost_core::chats::get(&state.pool, &auth.account.id, &id).await?;
    let refs: Vec<String> = sqlx::query_scalar(
        "SELECT ref FROM chat_folders cf WHERE chat_id = ? AND kind = 'server' \
         AND NOT EXISTS (SELECT 1 FROM chat_folders o WHERE o.kind = 'server' AND o.ref = cf.ref AND o.chat_id <> cf.chat_id)",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await
    .map_err(outpost_core::CoreError::from)?;
    let mut out = Vec::new();
    for r in refs {
        if let Ok(f) = chat_files::folder(&state.pool, &auth.account.id, &r).await {
            out.push(info(&state, f).await?);
        }
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewFolder {
    pub name: String,
}

/// Make a server folder (then upload files into it).
#[utoipa::path(post, path = "/api/folders", tag = "files", security(("bearer" = [])),
    request_body = NewFolder,
    responses((status = 201, description = "Made", body = Folder), (status = 400, description = "Bad name", body = ErrorBody)))]
pub async fn create(State(state): State<AppState>, auth: Auth, Json(req): Json<NewFolder>) -> ApiResult<(StatusCode, Json<Folder>)> {
    auth.require_person("Making folders")?;
    let f = chat_files::create_folder(&state.pool, &auth.account.id, &req.name).await?;
    let _ = tokio::fs::create_dir_all(root(&state, &f.account_id, &f.id)).await;
    Ok((StatusCode::CREATED, Json(f)))
}

/// Rename one of my server folders.
#[utoipa::path(patch, path = "/api/folders/{id}", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id")), request_body = NewFolder,
    responses((status = 200, description = "Renamed", body = Folder), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn rename(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(req): Json<NewFolder>) -> ApiResult<Json<Folder>> {
    Ok(Json(chat_files::rename_folder(&state.pool, &auth.account.id, &id, &req.name).await?))
}

/// Delete one of my server folders and its files.
#[utoipa::path(delete, path = "/api/folders/{id}", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    // Anything still running in it (a test server) stops first.
    let f = chat_files::folder(&state.pool, &auth.account.id, &id).await?;
    let prefix = format!("{}:", f.id);
    for (name, running) in crate::runner::list(&prefix) {
        if running {
            let _ = crate::runner::stop(&name);
        }
    }
    if !crate::runner::list(&prefix).is_empty() {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    chat_files::delete_folder(&state.pool, &state.config.files_dir(), &auth.account.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FolderFile {
    pub path: String,
    pub bytes: u64,
}

/// Every file in one of my server folders (up to 5,000; tool folders like node_modules skipped).
#[utoipa::path(get, path = "/api/folders/{id}/files", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id")),
    responses((status = 200, description = "Files", body = [FolderFile])))]
pub async fn files(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<FolderFile>>> {
    let f = chat_files::folder(&state.pool, &auth.account.id, &id).await?;
    let dir = root(&state, &f.account_id, &f.id);
    let out = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        if let Ok(list) = workspace::list_dir(&dir, 64, 5000) {
            for (p, is_dir) in list {
                if !is_dir {
                    let bytes = std::fs::metadata(dir.join(&p)).map(|m| m.len()).unwrap_or(0);
                    out.push(FolderFile { path: p, bytes });
                }
            }
        }
        out
    })
    .await
    .map_err(|_| ApiError::conflict("Couldn't list the folder."))?;
    Ok(Json(out))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PathQuery {
    /// Inside the folder, e.g. `src/app.ts`.
    pub path: String,
}

/// Put a file into one of my server folders (the body is the file; it replaces one there).
#[utoipa::path(put, path = "/api/folders/{id}/file", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id"), PathQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses((status = 204, description = "Saved"), (status = 400, description = "Bad path or too big", body = ErrorBody)))]
pub async fn put_file(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<PathQuery>,
    body: Body,
) -> ApiResult<StatusCode> {
    let f = chat_files::folder(&state.pool, &auth.account.id, &id).await?;
    let inner = inner(&q.path)?;
    let data = axum::body::to_bytes(body, MAX_UPLOAD)
        .await
        .map_err(|_| ApiError::bad_request("That file is too big (50 MB at most), or the upload was cut off."))?;
    let dir = root(&state, &f.account_id, &f.id);
    let file = workspace::within(&dir, &inner).map_err(ApiError::bad_request)?;
    if let Some(parent) = file.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| ApiError::conflict(e.to_string()))?;
    }
    tokio::fs::write(&file, &data).await.map_err(|e| ApiError::conflict(e.to_string()))?;
    let _ = chat_files::touch_folder(&state.pool, &f.id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// One file from one of my server folders.
#[utoipa::path(get, path = "/api/folders/{id}/file", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id"), PathQuery),
    responses((status = 200, description = "The file"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn get_file(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Query(q): Query<PathQuery>) -> ApiResult<Response> {
    let f = chat_files::folder(&state.pool, &auth.account.id, &id).await?;
    let file = workspace::within(&root(&state, &f.account_id, &f.id), &inner(&q.path)?).map_err(ApiError::bad_request)?;
    let data = tokio::fs::read(&file).await.map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "There's no such file."))?;
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(data))
        .map_err(|_| ApiError::conflict("Couldn't send the file."))
}

/// Delete a file from one of my server folders.
#[utoipa::path(delete, path = "/api/folders/{id}/file", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id"), PathQuery),
    responses((status = 204, description = "Deleted")))]
pub async fn delete_file(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Query(q): Query<PathQuery>) -> ApiResult<StatusCode> {
    let f = chat_files::folder(&state.pool, &auth.account.id, &id).await?;
    let file = workspace::within(&root(&state, &f.account_id, &f.id), &inner(&q.path)?).map_err(ApiError::bad_request)?;
    let _ = tokio::fs::remove_file(&file).await;
    Ok(StatusCode::NO_CONTENT)
}

/// What's been started in one of my server folders (test servers...), running or ended.
#[utoipa::path(get, path = "/api/folders/{id}/processes", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id")),
    responses((status = 200, description = "Processes: name, command, running, minutes")))]
pub async fn processes(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let f = chat_files::folder(&state.pool, &auth.account.id, &id).await?;
    Ok(Json(crate::runner::describe(&format!("{}:", f.id))))
}

/// Stop something started in one of my server folders.
#[utoipa::path(post, path = "/api/folders/{id}/processes/{name}/stop", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id"), ("name" = String, Path, description = "Its name")),
    responses((status = 204, description = "Stopped"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn stop_process(State(state): State<AppState>, auth: Auth, Path((id, name)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let f = chat_files::folder(&state.pool, &auth.account.id, &id).await?;
    crate::runner::stop(&format!("{}:{name}", f.id)).map_err(|e| ApiError::new(StatusCode::NOT_FOUND, "not_found", e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// One of my server folders as a zip.
#[utoipa::path(get, path = "/api/folders/{id}/zip", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Folder id")),
    responses((status = 200, description = "A zip of its files")))]
pub async fn zip(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    let f = chat_files::folder(&state.pool, &auth.account.id, &id).await?;
    let dir = root(&state, &f.account_id, &f.id);
    let data = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (p, is_dir) in workspace::list_dir(&dir, 64, 20_000).unwrap_or_default() {
            if is_dir {
                continue;
            }
            let bytes = std::fs::read(dir.join(&p)).map_err(|e| e.to_string())?;
            zip.start_file(p, opts).map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
    })
    .await
    .map_err(|_| ApiError::conflict("Couldn't pack the folder."))?
    .map_err(ApiError::conflict)?;
    let name: String = f.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}.zip\""))
        .body(Body::from(data))
        .map_err(|_| ApiError::conflict("Couldn't send the zip."))
}

// ---------------------------------------------------------------- a chat's folders

/// The folders a chat of mine works with.
#[utoipa::path(get, path = "/api/chats/{id}/folders", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    responses((status = 200, description = "Folders", body = [ChatFolder])))]
pub async fn chat_folders(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<ChatFolder>>> {
    outpost_core::chats::get(&state.pool, &auth.account.id, &id).await?;
    Ok(Json(chat_files::chat_folders(&state.pool, &id).await?))
}

/// Set the folders a chat of mine works with (server folders of mine, or folders on a PC).
#[utoipa::path(put, path = "/api/chats/{id}/folders", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")), request_body = [ChatFolder],
    responses((status = 200, description = "Saved", body = [ChatFolder]), (status = 400, description = "Not usable", body = ErrorBody)))]
pub async fn set_chat_folders(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(list): Json<Vec<ChatFolder>>,
) -> ApiResult<Json<Vec<ChatFolder>>> {
    Ok(Json(chat_files::set_chat_folders(&state.pool, &auth.account.id, &id, &list).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FileMode {
    /// `ask` (each change waits for Apply) or `auto` (made straight away, with Undo).
    pub mode: String,
}

/// Whether a chat of mine asks before changing files.
#[utoipa::path(put, path = "/api/chats/{id}/file-mode", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")), request_body = FileMode,
    responses((status = 204, description = "Saved")))]
pub async fn set_file_mode(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(req): Json<FileMode>) -> ApiResult<StatusCode> {
    chat_files::set_file_mode(&state.pool, &auth.account.id, &id, &req.mode).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The desktop app's answer to a reply's `file_request` (for a PC folder).
#[utoipa::path(post, path = "/api/chats/{id}/file-answers/{request}", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id"), ("request" = String, Path, description = "The request's id")),
    request_body = Object,
    responses((status = 204, description = "Taken"), (status = 404, description = "Nothing waiting", body = ErrorBody)))]
pub async fn answer(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, request)): Path<(String, String)>,
    Json(value): Json<Value>,
) -> ApiResult<StatusCode> {
    outpost_core::chats::get(&state.pool, &auth.account.id, &id).await?;
    if state.file_relay.answer(&id, &request, value) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "Nothing is waiting for that (it took too long?)."))
    }
}

// ---------------------------------------------------------------- changes

/// Every change replies in a chat of mine made or proposed, oldest first.
#[utoipa::path(get, path = "/api/chats/{id}/changes", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id")),
    responses((status = 200, description = "Changes", body = [FileChange])))]
pub async fn changes(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<Vec<FileChange>>> {
    outpost_core::chats::get(&state.pool, &auth.account.id, &id).await?;
    Ok(Json(chat_files::changes(&state.pool, &id).await?))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ChangeAction {
    /// For a PC folder: the desktop app has already done it there (Outpost only records it).
    #[serde(default)]
    pub done_on_pc: bool,
    /// Do it even though the file changed since.
    #[serde(default)]
    pub force: bool,
}

/// Apply a proposed change, skip it, or undo one that was made: `action` is `apply`, `skip`
/// or `undo`. A server folder's file is changed here; a PC folder's by its desktop app (which
/// then says so with `done_on_pc`).
#[utoipa::path(post, path = "/api/chats/{id}/changes/{change}/{action}", tag = "files", security(("bearer" = [])),
    params(("id" = String, Path, description = "Chat id"), ("change" = String, Path, description = "Change id"),
        ("action" = String, Path, description = "apply, skip or undo")),
    request_body = ChangeAction,
    responses((status = 200, description = "Done", body = FileChange),
        (status = 409, description = "The file changed since (send force to go ahead)", body = ErrorBody)))]
pub async fn act(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, change, action)): Path<(String, String, String)>,
    body: Option<Json<ChangeAction>>,
) -> ApiResult<Json<FileChange>> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    outpost_core::chats::get(&state.pool, &auth.account.id, &id).await?;
    let c = chat_files::change(&state.pool, &id, &change).await?;
    let to = match action.as_str() {
        "apply" => "applied",
        "skip" => "skipped",
        "undo" => "undone",
        _ => return Err(ApiError::bad_request("Apply, skip or undo.")),
    };
    if to != "skipped" {
        if c.kind == "pc" {
            if !req.done_on_pc {
                return Err(ApiError::conflict("That file is on a PC: its desktop app makes the change."));
            }
        } else {
            let dir = root(&state, &auth.account.id, &c.folder_id);
            chat_files::folder(&state.pool, &auth.account.id, &c.folder_id).await?;
            let path = inner(&c.path)?;
            let file = workspace::within(&dir, &path).map_err(ApiError::bad_request)?;
            let now = tokio::fs::read(&file).await.ok().and_then(|b| workspace::text_of(b).ok());
            // What the file should hold now, and what it becomes.
            let (expect, becomes) = if to == "applied" { (c.before.clone(), Some(c.after.clone())) } else { (Some(c.after.clone()), c.before.clone()) };
            if now != expect && !req.force {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "changed_since",
                    format!("{} has changed since. Do it anyway?", c.path),
                ));
            }
            match becomes {
                Some(text) => workspace::write_server(&dir, &path, &text).await.map_err(ApiError::conflict)?,
                None => {
                    let _ = tokio::fs::remove_file(&file).await;
                }
            }
            let _ = chat_files::touch_folder(&state.pool, &c.folder_id).await;
        }
    }
    Ok(Json(chat_files::set_change_status(&state.pool, &id, &change, to).await?))
}
