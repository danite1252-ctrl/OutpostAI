//! Folders on this PC that chats can use. The chat keeps only each folder's name and an id;
//! where it is stays here, in the app's settings. While a reply streams, the server asks this
//! app to list, read, search or write in one (`file_request` events), and the app does it
//! here, never outside the folder.

use std::path::{Path, PathBuf};

use outpost_gateway::workspace;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// A folder on this PC that chats may use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PcFolder {
    pub id: String,
    pub name: String,
    pub path: String,
}

fn all() -> Vec<PcFolder> {
    serde_json::from_value(crate::read_settings()["pc_folders"].clone()).unwrap_or_default()
}

fn save(list: &[PcFolder]) -> Result<(), String> {
    crate::save_setting_value("pc_folders", serde_json::to_value(list).map_err(|e| e.to_string())?)
}

fn find(id: &str) -> Result<PcFolder, String> {
    all().into_iter().find(|f| f.id == id).ok_or_else(|| "That folder isn't set up on this PC any more.".into())
}

/// This PC's name (shown on chats that use its folders: "On DESKTOP-DAN").
#[tauri::command]
pub fn pc_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "this PC".into())
}

/// The folders chats may use on this PC.
#[tauri::command]
pub fn pc_folders() -> Vec<PcFolder> {
    all()
}

/// Pick a folder (Windows' folder picker); it's added, or the one already there returned.
/// `None` if the picker was closed.
#[tauri::command]
pub async fn pc_folder_pick(app: AppHandle) -> Result<Option<PcFolder>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title("Choose a folder for the chat").pick_folder(move |p| {
        let _ = tx.send(p);
    });
    let Some(picked) = rx.await.map_err(|e| e.to_string())? else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let path_text = path.to_string_lossy().to_string();
    let mut list = all();
    if let Some(f) = list.iter().find(|f| Path::new(&f.path) == path) {
        return Ok(Some(f.clone()));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Folder".into());
    let f = PcFolder { id: format!("pc-{}", uuid_like()), name, path: path_text };
    list.push(f.clone());
    save(&list)?;
    Ok(Some(f))
}

/// Stop offering a folder to chats (nothing in it changes).
#[tauri::command]
pub fn pc_folder_forget(id: String) -> Result<(), String> {
    let list: Vec<PcFolder> = all().into_iter().filter(|f| f.id != id).collect();
    save(&list)
}

fn uuid_like() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}{:x}", t, std::process::id())
}

/// Do what a reply asked in one of this PC's folders: `list`, `read`, `search`, `write`,
/// `delete`, `run` a command (or `cancel` it), or a long-running process: `start`, `output`,
/// `input`, `stop`. Answers the way the server expects (`error` when it didn't work).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn pc_folder_op(
    id: String,
    op: String,
    path: String,
    text: Option<String>,
    content: Option<String>,
    command: Option<String>,
    timeout: Option<u64>,
    run_id: Option<String>,
    name: Option<String>,
    lines: Option<usize>,
) -> Value {
    use outpost_gateway::runner;
    // A process is known by its folder and name.
    let proc_name = format!("{id}:{}", name.clone().unwrap_or_default());
    let result = match op.as_str() {
        "run" => match spec(&id, &path, command.unwrap_or_default()) {
            Ok(s) => runner::run(&s, timeout.unwrap_or(600), run_id).await,
            Err(e) => Err(e),
        },
        "cancel" => {
            // Stop a running command (the person pressed Stop).
            if let Some(r) = run_id {
                runner::cancel(&r);
            }
            Ok(json!({ "ok": true }))
        }
        "start" => match spec(&id, &path, command.unwrap_or_default()) {
            Ok(s) => runner::start(&proc_name, &s).await,
            Err(e) => Err(e),
        },
        "output" => runner::output(&proc_name, lines.unwrap_or(60), None),
        "input" => runner::input(&proc_name, &text.unwrap_or_default()).await,
        "stop" => runner::stop(&proc_name),
        "processes" => Ok(json!({ "processes": runner::list(&format!("{id}:")).into_iter()
            .map(|(n, running)| json!({ "name": n.split_once(':').map(|x| x.1.to_string()).unwrap_or(n), "running": running }))
            .collect::<Vec<_>>() })),
        _ => run(&id, &op, &path, text, content).await,
    };
    match result {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    }
}

/// What to run, where: a command line in the folder (or a sub-folder of it), as you, with
/// Windows' command prompt (`sh` elsewhere), no window.
fn spec(id: &str, path: &str, command: String) -> Result<outpost_gateway::runner::Spec, String> {
    let folder = find(id)?;
    let root = PathBuf::from(&folder.path);
    if !root.is_dir() {
        return Err(format!("{} isn't on this PC any more ({}).", folder.name, folder.path));
    }
    let inner = outpost_core::chat_files::safe_path(path).ok_or("That path leaves the folder.")?;
    let dir = workspace::within(&root, &inner)?;
    if !dir.is_dir() {
        return Err("That isn't a folder: give the folder to run in.".into());
    }
    if command.trim().is_empty() {
        return Err("Say which command to run.".into());
    }
    Ok(outpost_gateway::runner::Spec { dir, command, env: Vec::new(), sandbox: None })
}

async fn run(id: &str, op: &str, path: &str, text: Option<String>, content: Option<String>) -> Result<Value, String> {
    let folder = find(id)?;
    let root = PathBuf::from(&folder.path);
    if !root.is_dir() {
        return Err(format!("{} isn't on this PC any more ({}).", folder.name, folder.path));
    }
    let inner = outpost_core::chat_files::safe_path(path).ok_or("That path leaves the folder.")?;
    let full = workspace::within(&root, &inner)?;
    match op {
        "list" => {
            let start = full.clone();
            let entries = tokio::task::spawn_blocking(move || workspace::list_dir(&start, 2, 250))
                .await
                .map_err(|e| e.to_string())??;
            Ok(json!({ "entries": entries.into_iter().map(|(p, dir)| json!({ "path": p, "dir": dir })).collect::<Vec<_>>() }))
        }
        "read" => match tokio::fs::read(&full).await {
            Ok(bytes) => Ok(json!({ "text": workspace::text_of(bytes)? })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({ "missing": true })),
            Err(_) if full.is_dir() => Err("That's a folder: use list_files.".into()),
            Err(e) => Err(e.to_string()),
        },
        "search" => {
            let needle = text.unwrap_or_default().to_lowercase();
            let start = full.clone();
            let hits = tokio::task::spawn_blocking(move || workspace::search_dir(&start, &needle, 40))
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "hits": hits.into_iter().map(|(p, line, t)| json!({ "path": p, "line": line, "text": t })).collect::<Vec<_>>() }))
        }
        "write" => {
            if inner.as_os_str().is_empty() {
                return Err("Give a file's path.".into());
            }
            workspace::write_server(&root, &inner, &content.unwrap_or_default()).await?;
            Ok(json!({ "ok": true }))
        }
        "delete" => {
            if full.is_file() {
                tokio::fs::remove_file(&full).await.map_err(|e| e.to_string())?;
            }
            Ok(json!({ "ok": true }))
        }
        _ => Err(format!("Unknown request {op}.")),
    }
}
