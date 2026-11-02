//! Files in chats: folders a chat's model can read and change.
//!
//! - **Server folders** are kept by Outpost (under `files/<account>/folders/<id>`): made by
//!   uploading files, downloadable as a zip, usable from any device.
//! - **PC folders** stay on one PC: the chat records only their name and which PC; that PC's
//!   desktop app reads and writes them for the chat while it's open.
//!
//! A reply's file work is kept on the message (what it read, searched, changed), and each
//! change it made or proposed is kept whole (before and after), for Apply, Skip and Undo.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

/// A folder kept on the server.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A folder a chat uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct ChatFolder {
    /// `server` or `pc`.
    pub kind: String,
    /// The server folder's id, or the PC folder's id on its PC.
    #[sqlx(rename = "ref")]
    pub id: String,
    /// Its name: the first part of every path the model uses in it.
    pub name: String,
    /// For a PC folder: which PC.
    pub device: Option<String>,
}

/// One thing a reply did with files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FileAction {
    /// `list`, `read`, `search`, `write` or `edit`.
    pub op: String,
    /// The path, starting with the folder's name ("Shop/src/app.ts").
    pub path: String,
    /// A short note: lines read, matches found, why it failed.
    #[serde(default)]
    pub note: Option<String>,
    /// For a change: its record (see [`FileChange`]).
    #[serde(default)]
    pub change_id: Option<String>,
    /// It didn't work (the note says why).
    #[serde(default)]
    pub failed: bool,
}

/// A change to a file, proposed or made.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct FileChange {
    pub id: String,
    pub chat_id: String,
    pub kind: String,
    #[sqlx(rename = "ref")]
    pub folder_id: String,
    /// The folder's name.
    pub folder: String,
    /// Inside the folder.
    pub path: String,
    /// What the file held (`None`: it's a new file).
    pub before: Option<String>,
    pub after: String,
    /// `proposed`, `applied`, `skipped` or `undone`.
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const MAX_NAME: usize = 60;

/// A folder's name: one line, no slashes, at most 60 characters.
pub fn clean_folder_name(name: &str) -> CoreResult<String> {
    let n: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let n: String = n.trim_matches('.').trim().chars().take(MAX_NAME).collect();
    if n.is_empty() {
        return Err(CoreError::Invalid("Give the folder a name.".into()));
    }
    Ok(n)
}

/// A path inside a folder, made safe: relative, no `..`, no drive or root. `None` when it
/// would leave the folder. `""` (or `.`) is the folder itself.
pub fn safe_path(path: &str) -> Option<PathBuf> {
    let path = path.replace('\\', "/");
    if path.starts_with('/') || path.contains(':') || path.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(&path).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

/// Where a server folder's files are.
pub fn folder_dir(files_dir: &Path, account_id: &str, folder_id: &str) -> PathBuf {
    files_dir.join(account_id).join("folders").join(folder_id)
}

// ---------------------------------------------------------------- server folders

pub async fn create_folder(pool: &SqlitePool, account_id: &str, name: &str) -> CoreResult<Folder> {
    let t = now();
    let f = Folder {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.into(),
        name: clean_folder_name(name)?,
        created_at: t,
        updated_at: t,
    };
    sqlx::query("INSERT INTO folders (id, account_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&f.id)
        .bind(&f.account_id)
        .bind(&f.name)
        .bind(t)
        .bind(t)
        .execute(pool)
        .await?;
    Ok(f)
}

pub async fn folders(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Folder>> {
    Ok(sqlx::query_as("SELECT id, account_id, name, created_at, updated_at FROM folders WHERE account_id = ? ORDER BY name")
        .bind(account_id)
        .fetch_all(pool)
        .await?)
}

pub async fn folder(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Folder> {
    sqlx::query_as("SELECT id, account_id, name, created_at, updated_at FROM folders WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

pub async fn touch_folder(pool: &SqlitePool, id: &str) -> CoreResult<()> {
    sqlx::query("UPDATE folders SET updated_at = ? WHERE id = ?").bind(now()).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn rename_folder(pool: &SqlitePool, account_id: &str, id: &str, name: &str) -> CoreResult<Folder> {
    let name = clean_folder_name(name)?;
    let done = sqlx::query("UPDATE folders SET name = ?, updated_at = ? WHERE id = ? AND account_id = ?")
        .bind(&name)
        .bind(now())
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    // Chats show it by its new name too.
    sqlx::query("UPDATE chat_folders SET name = ? WHERE kind = 'server' AND ref = ?").bind(&name).bind(id).execute(pool).await?;
    folder(pool, account_id, id).await
}

/// Delete a server folder and its files; chats stop using it.
pub async fn delete_folder(pool: &SqlitePool, files_dir: &Path, account_id: &str, id: &str) -> CoreResult<()> {
    folder(pool, account_id, id).await?;
    sqlx::query("DELETE FROM chat_folders WHERE kind = 'server' AND ref = ?").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM folders WHERE id = ? AND account_id = ?").bind(id).bind(account_id).execute(pool).await?;
    let _ = tokio::fs::remove_dir_all(folder_dir(files_dir, account_id, id)).await;
    Ok(())
}

// ---------------------------------------------------------------- a chat's folders

pub async fn chat_folders(pool: &SqlitePool, chat_id: &str) -> CoreResult<Vec<ChatFolder>> {
    Ok(sqlx::query_as("SELECT kind, ref, name, device FROM chat_folders WHERE chat_id = ? ORDER BY name")
        .bind(chat_id)
        .fetch_all(pool)
        .await?)
}

/// Replace a chat's folders. Server folders must be mine; names are unique within the chat
/// (a clash gets " 2" added), since they start every path.
pub async fn set_chat_folders(
    pool: &SqlitePool,
    account_id: &str,
    chat_id: &str,
    list: &[ChatFolder],
) -> CoreResult<Vec<ChatFolder>> {
    crate::chats::get(pool, account_id, chat_id).await?;
    if list.len() > 8 {
        return Err(CoreError::Invalid("A chat can use up to 8 folders.".into()));
    }
    let mut taken: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for f in list {
        let name = match f.kind.as_str() {
            "server" => folder(pool, account_id, &f.id).await?.name,
            "pc" => {
                if f.id.is_empty() || f.id.len() > 80 {
                    return Err(CoreError::Invalid("That PC folder's id isn't usable.".into()));
                }
                clean_folder_name(&f.name)?
            }
            _ => return Err(CoreError::Invalid("A folder is on the server or on a PC.".into())),
        };
        let mut unique = name.clone();
        let mut n = 2;
        while taken.iter().any(|t| t.eq_ignore_ascii_case(&unique)) {
            unique = format!("{name} {n}");
            n += 1;
        }
        taken.push(unique.clone());
        out.push(ChatFolder {
            kind: f.kind.clone(),
            id: f.id.clone(),
            name: unique,
            device: f.device.clone().filter(|_| f.kind == "pc").map(|d| d.chars().take(60).collect()),
        });
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM chat_folders WHERE chat_id = ?").bind(chat_id).execute(&mut *tx).await?;
    for f in &out {
        sqlx::query("INSERT INTO chat_folders (chat_id, kind, ref, name, device) VALUES (?, ?, ?, ?, ?)")
            .bind(chat_id)
            .bind(&f.kind)
            .bind(&f.id)
            .bind(&f.name)
            .bind(&f.device)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(out)
}

/// `ask` or `auto`.
pub async fn set_file_mode(pool: &SqlitePool, account_id: &str, chat_id: &str, mode: &str) -> CoreResult<()> {
    if !matches!(mode, "ask" | "auto") {
        return Err(CoreError::Invalid("Changes are either asked about first or made straight away.".into()));
    }
    let done = sqlx::query("UPDATE chats SET file_mode = ? WHERE id = ? AND account_id = ?")
        .bind(mode)
        .bind(chat_id)
        .bind(account_id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

// ---------------------------------------------------------------- what replies did

pub async fn set_actions(pool: &SqlitePool, message_id: &str, actions: &[FileAction]) -> CoreResult<()> {
    if actions.is_empty() {
        return Ok(());
    }
    let json = serde_json::to_string(actions).map_err(|e| CoreError::Invalid(e.to_string()))?;
    sqlx::query("UPDATE messages SET files = ? WHERE id = ?").bind(json).bind(message_id).execute(pool).await?;
    Ok(())
}

pub async fn actions_for_chat(pool: &SqlitePool, chat_id: &str) -> CoreResult<HashMap<String, Vec<FileAction>>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, files FROM messages WHERE chat_id = ? AND files IS NOT NULL")
        .bind(chat_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, json)| serde_json::from_str(&json).ok().map(|a| (id, a)))
        .collect())
}

/// Keep a reply's tool calls and results (Ollama's messages), for the history next time.
pub async fn set_trace(pool: &SqlitePool, message_id: &str, trace: &[serde_json::Value]) -> CoreResult<()> {
    if trace.is_empty() {
        return Ok(());
    }
    let json = serde_json::to_string(trace).map_err(|e| CoreError::Invalid(e.to_string()))?;
    sqlx::query("UPDATE messages SET tool_trace = ? WHERE id = ?").bind(json).bind(message_id).execute(pool).await?;
    Ok(())
}

/// The tool calls and results of a chat's replies, by message.
pub async fn traces_for_chat(pool: &SqlitePool, chat_id: &str) -> CoreResult<HashMap<String, Vec<serde_json::Value>>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, tool_trace FROM messages WHERE chat_id = ? AND tool_trace IS NOT NULL")
            .bind(chat_id)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, json)| serde_json::from_str(&json).ok().map(|t| (id, t)))
        .collect())
}

// ---------------------------------------------------------------- changes

const CHANGE_COLS: &str = "id, chat_id, kind, ref, folder, path, before, after, status, created_at, updated_at";

#[allow(clippy::too_many_arguments)]
pub async fn add_change(
    pool: &SqlitePool,
    chat_id: &str,
    folder: &ChatFolder,
    path: &str,
    before: Option<&str>,
    after: &str,
    status: &str,
) -> CoreResult<FileChange> {
    let t = now();
    let c = FileChange {
        id: uuid::Uuid::new_v4().to_string(),
        chat_id: chat_id.into(),
        kind: folder.kind.clone(),
        folder_id: folder.id.clone(),
        folder: folder.name.clone(),
        path: path.into(),
        before: before.map(Into::into),
        after: after.into(),
        status: status.into(),
        created_at: t,
        updated_at: t,
    };
    sqlx::query(&format!("INSERT INTO file_changes ({CHANGE_COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"))
        .bind(&c.id)
        .bind(&c.chat_id)
        .bind(&c.kind)
        .bind(&c.folder_id)
        .bind(&c.folder)
        .bind(&c.path)
        .bind(&c.before)
        .bind(&c.after)
        .bind(&c.status)
        .bind(t)
        .bind(t)
        .execute(pool)
        .await?;
    Ok(c)
}

pub async fn changes(pool: &SqlitePool, chat_id: &str) -> CoreResult<Vec<FileChange>> {
    Ok(sqlx::query_as(&format!("SELECT {CHANGE_COLS} FROM file_changes WHERE chat_id = ? ORDER BY created_at, rowid"))
        .bind(chat_id)
        .fetch_all(pool)
        .await?)
}

pub async fn change(pool: &SqlitePool, chat_id: &str, id: &str) -> CoreResult<FileChange> {
    sqlx::query_as(&format!("SELECT {CHANGE_COLS} FROM file_changes WHERE id = ? AND chat_id = ?"))
        .bind(id)
        .bind(chat_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

/// Move a change on: proposed → applied | skipped; applied → undone. Anything else is refused.
pub async fn set_change_status(pool: &SqlitePool, chat_id: &str, id: &str, to: &str) -> CoreResult<FileChange> {
    let c = change(pool, chat_id, id).await?;
    let ok = matches!((c.status.as_str(), to), ("proposed", "applied") | ("proposed", "skipped") | ("applied", "undone"));
    if !ok {
        return Err(CoreError::Invalid(format!("That change is {} already.", c.status)));
    }
    sqlx::query("UPDATE file_changes SET status = ?, updated_at = ? WHERE id = ?")
        .bind(to)
        .bind(now())
        .bind(id)
        .execute(pool)
        .await?;
    change(pool, chat_id, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;

    #[test]
    fn paths_stay_inside_the_folder() {
        assert_eq!(safe_path("src/app.ts").unwrap(), PathBuf::from("src/app.ts"));
        assert_eq!(safe_path("./src\\\\lib/x.rs").unwrap(), PathBuf::from("src/lib/x.rs"));
        assert_eq!(safe_path("").unwrap(), PathBuf::new());
        for bad in ["../x", "src/../../x", "/etc/passwd", "C:\\\\Windows", "a\u{0}b"] {
            assert!(safe_path(bad).is_none(), "{bad}");
        }
        assert_eq!(clean_folder_name("  my/shop \n ").unwrap(), "my shop");
        assert!(clean_folder_name(" .. ").is_err());
    }

    #[tokio::test]
    async fn a_chat_uses_my_folders_and_changes_move_on_once() {
        let pool = open_in_memory().await.unwrap();
        let dan = accounts::create(&pool, NewAccount { name: "Dan".into(), role: Role::Member, secret: Some("4821".into()) })
            .await
            .unwrap()
            .id;
        let sam = accounts::create(&pool, NewAccount { name: "Sam".into(), role: Role::Member, secret: Some("4821".into()) })
            .await
            .unwrap()
            .id;
        let chat = crate::chats::create(&pool, &dan, None, None).await.unwrap();
        assert_eq!(chat.file_mode, "ask");
        let shop = create_folder(&pool, &dan, "Shop").await.unwrap();
        let theirs = create_folder(&pool, &sam, "Theirs").await.unwrap();
        let pc = ChatFolder { kind: "pc".into(), id: "pc-1".into(), name: "Shop".into(), device: Some("Dan's PC".into()) };
        let server = ChatFolder { kind: "server".into(), id: shop.id.clone(), name: "ignored".into(), device: None };
        let set = set_chat_folders(&pool, &dan, &chat.id, &[server.clone(), pc.clone()]).await.unwrap();
        assert_eq!(set.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Shop", "Shop 2"], "names stay unique");
        assert_eq!(chat_folders(&pool, &chat.id).await.unwrap().len(), 2);
        // Someone else's server folder isn't mine to use.
        let not_mine = ChatFolder { kind: "server".into(), id: theirs.id.clone(), name: "x".into(), device: None };
        assert!(set_chat_folders(&pool, &dan, &chat.id, &[not_mine]).await.is_err());

        set_file_mode(&pool, &dan, &chat.id, "auto").await.unwrap();
        assert_eq!(crate::chats::get(&pool, &dan, &chat.id).await.unwrap().file_mode, "auto");
        assert!(set_file_mode(&pool, &dan, &chat.id, "sometimes").await.is_err());

        let c = add_change(&pool, &chat.id, &set[0], "src/a.ts", None, "x", "proposed").await.unwrap();
        assert_eq!(set_change_status(&pool, &chat.id, &c.id, "applied").await.unwrap().status, "applied");
        assert!(set_change_status(&pool, &chat.id, &c.id, "skipped").await.is_err(), "applied can only be undone");
        assert_eq!(set_change_status(&pool, &chat.id, &c.id, "undone").await.unwrap().status, "undone");
        assert_eq!(changes(&pool, &chat.id).await.unwrap().len(), 1);

        // Renaming a server folder renames it in chats; deleting it takes it out of them.
        rename_folder(&pool, &dan, &shop.id, "Web shop").await.unwrap();
        assert!(chat_folders(&pool, &chat.id).await.unwrap().iter().any(|f| f.name == "Web shop"));
        let dir = std::env::temp_dir().join(format!("outpost-folders-{}", uuid::Uuid::new_v4()));
        delete_folder(&pool, &dir, &dan, &shop.id).await.unwrap();
        assert_eq!(chat_folders(&pool, &chat.id).await.unwrap().len(), 1);
    }
}
