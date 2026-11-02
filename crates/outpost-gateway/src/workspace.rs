//! Files in chats, the model's side: the tools it's given for a chat's folders (list, read,
//! search, write, edit), and running them.
//!
//! Server folders are read and written here. A PC folder is read and written by the desktop
//! app on that PC, while it streams the reply: a `file_request` event goes to the app, which
//! does it and posts the answer back (see [`Relay`]). In `ask` mode a change is only proposed
//! (the person applies or skips it later); in `auto` it's made at once, with Undo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::response::sse::Event;
use outpost_core::chat_files::{self, ChatFolder, FileAction};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use crate::AppState;

/// Most characters of a file the model is shown at once.
const READ_LIMIT: usize = 12_000;
/// Largest file the model may write.
const WRITE_LIMIT: usize = 1024 * 1024;
/// Most entries a listing shows.
const LIST_LIMIT: usize = 250;
/// Most search hits.
const SEARCH_LIMIT: usize = 40;
/// Folders not worth looking in.
pub const SKIP_DIRS: [&str; 11] =
    [".git", "node_modules", "target", "dist", "build", ".next", ".venv", "venv", "__pycache__", ".idea", ".vs"];

/// Requests waiting for a desktop app to answer (PC folders).
#[derive(Clone, Default)]
pub struct Relay {
    waiting: Arc<Mutex<HashMap<String, (String, oneshot::Sender<Value>)>>>,
}

impl Relay {
    /// The app answered request `id` (for chat `chat_id`). False if nothing was waiting.
    pub fn answer(&self, chat_id: &str, id: &str, value: Value) -> bool {
        let mut w = self.waiting.lock().unwrap_or_else(|p| p.into_inner());
        match w.remove(id) {
            Some((chat, tx)) if chat == chat_id => tx.send(value).is_ok(),
            Some(other) => {
                w.insert(id.to_string(), other);
                false
            }
            None => false,
        }
    }

    /// Requests still waiting for an answer (for tests).
    #[cfg(test)]
    pub(crate) fn pending_ids(&self) -> Vec<String> {
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).keys().cloned().collect()
    }

    async fn ask(&self, events: &mpsc::Sender<Event>, chat_id: &str, request: Value) -> Result<Value, String> {
        self.ask_within(events, chat_id, request, Duration::from_secs(60)).await
    }

    /// Ask the app streaming this reply to do something (a `file_request` event) and wait for
    /// its answer; an `error` in the answer comes back as the error.
    pub(crate) async fn ask_within(
        &self,
        events: &mpsc::Sender<Event>,
        chat_id: &str,
        request: Value,
        within: Duration,
    ) -> Result<Value, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), (chat_id.to_string(), tx));
        let mut body = request;
        body["id"] = json!(id);
        let sent = events.send(Event::default().event("file_request").data(body.to_string())).await;
        let answer = if sent.is_err() {
            Err("The app closed the chat.".to_string())
        } else {
            match tokio::time::timeout(within, rx).await {
                Ok(Ok(v)) => Ok(v),
                _ => Err("The desktop app didn't answer in time (closed, or what it was asked for isn't there any more).".to_string()),
            }
        };
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
        let v = answer?;
        match v.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None => Ok(v),
        }
    }
}

/// A chat's folders, for one reply.
pub struct Workspace {
    pub state: AppState,
    pub account_id: String,
    pub chat_id: String,
    pub folders: Vec<ChatFolder>,
    /// `ask` or `auto`.
    pub mode: String,
    /// PC folders the app streaming this reply can reach (their ids).
    pub reachable: Vec<String>,
    pub events: mpsc::Sender<Event>,
}

/// What a tool call came to: the model's answer, and what to show under the reply.
pub struct Outcome {
    pub reply: String,
    pub action: FileAction,
}

impl Workspace {
    /// The system prompt's part about the folders.
    pub fn instructions(&self) -> String {
        let list: Vec<String> = self
            .folders
            .iter()
            .map(|f| match f.kind.as_str() {
                "pc" if !self.reachable.contains(&f.id) => format!(
                    "- {} (on {}; not reachable now: it can only be used from the Outpost desktop app on that PC)",
                    f.name,
                    f.device.as_deref().unwrap_or("another PC")
                ),
                "pc" => format!("- {} (a folder on {})", f.name, f.device.as_deref().unwrap_or("the user's PC")),
                _ => format!("- {} (a folder on the Outpost server)", f.name),
            })
            .collect();
        let example = self.folders.first().map(|f| f.name.as_str()).unwrap_or("Project");
        let changes = if self.mode == "auto" {
            "Changes you make are saved straight away (the user can undo them)."
        } else {
            "Changes you make are shown to the user, who applies or skips each one: say what you changed and why."
        };
        format!(
            "## Files\nYou can work with files in these folders, using your tools:\n{}\nEvery path starts with the folder's \
             name, e.g. `{example}/src/main.ts`. Look before you change anything: list_files to see what's there, \
             read_file to read, search_files to find text. Change files with edit_file (replace an exact piece of text) \
             or write_file (a new file, or a whole new version). Keep changes to what was asked. {changes} Never say a \
             change was made unless a tool said so.",
            list.join("\n")
        )
    }

    /// The tool definitions for Ollama.
    pub fn tools() -> Value {
        let path = json!({ "type": "string", "description": "Path starting with the folder's name, e.g. Shop/src/app.ts" });
        json!([
            { "type": "function", "function": {
                "name": "list_files",
                "description": "List the files and folders in a folder (two levels deep).",
                "parameters": { "type": "object", "properties": { "path": path }, "required": ["path"] } } },
            { "type": "function", "function": {
                "name": "read_file",
                "description": "Read a text file, with line numbers. Long files come in parts: ask for more with from_line.",
                "parameters": { "type": "object", "properties": {
                    "path": path,
                    "from_line": { "type": "integer", "description": "First line to show (from 1)." } }, "required": ["path"] } } },
            { "type": "function", "function": {
                "name": "search_files",
                "description": "Find text in the files of a folder (not case-sensitive). Gives each matching line.",
                "parameters": { "type": "object", "properties": {
                    "text": { "type": "string", "description": "What to find." },
                    "path": { "type": "string", "description": "Folder to look in, e.g. Shop or Shop/src." } }, "required": ["text", "path"] } } },
            { "type": "function", "function": {
                "name": "write_file",
                "description": "Create a file, or replace a file's whole content.",
                "parameters": { "type": "object", "properties": {
                    "path": path,
                    "content": { "type": "string", "description": "The complete new content." } }, "required": ["path", "content"] } } },
            { "type": "function", "function": {
                "name": "edit_file",
                "description": "Change part of a file: replace an exact piece of its text (it must appear exactly once) with new text.",
                "parameters": { "type": "object", "properties": {
                    "path": path,
                    "find": { "type": "string", "description": "The exact text now in the file (copy it, with enough lines to be unique)." },
                    "replace": { "type": "string", "description": "What it becomes." } }, "required": ["path", "find", "replace"] } } }
        ])
    }

    /// Run one tool call.
    pub async fn run(&self, name: &str, args: &Value) -> Outcome {
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let full = s("path");
        let op = match name {
            "list_files" => "list",
            "read_file" => "read",
            "search_files" => "search",
            "write_file" => "write",
            "edit_file" => "edit",
            _ => {
                return fail("tool", &full, format!("There's no tool called {name}."));
            }
        };
        let (folder, inner) = match self.locate(&full) {
            Ok(x) => x,
            Err(e) => return fail(op, &full, e),
        };
        let shown = join(&folder.name, &inner);
        if folder.kind == "pc" && !self.reachable.contains(&folder.id) {
            return fail(
                op,
                &shown,
                format!(
                    "{} is on {}: it can only be used from the Outpost desktop app there.",
                    folder.name,
                    folder.device.as_deref().unwrap_or("another PC")
                ),
            );
        }
        let result = match op {
            "list" => self.list(folder, &inner).await.map(|(text, n)| (text, format!("{n} entries"), None)),
            "read" => {
                let from = args.get("from_line").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
                self.read_text(folder, &inner).await.and_then(|t| match t {
                    Some(t) => Ok(numbered(&shown, &t, from)),
                    None => Err("There's no such file.".to_string()),
                })
            }
            "search" => self.search(folder, &inner, &s("text")).await.map(|(text, n)| (text, format!("{n} matches"), None)),
            "write" => self.change(folder, &inner, |_| Ok(s("content"))).await,
            _ => {
                let (find, replace) = (s("find"), s("replace"));
                self.change(folder, &inner, |before| {
                    let before = before.ok_or("There's no such file. Use write_file to create it.")?;
                    if find.is_empty() {
                        return Err("Say which text to replace (find).".into());
                    }
                    match before.matches(&find).count() {
                        1 => Ok(before.replacen(&find, &replace, 1)),
                        0 => Err("That text isn't in the file (it must match exactly, spaces and all). Read the file again.".into()),
                        n => Err(format!("That text is in the file {n} times. Include more lines around it so it's unique.")),
                    }
                })
                .await
            }
        };
        match result {
            Ok((reply, note, change_id)) => Outcome {
                reply,
                action: FileAction { op: op.into(), path: shown, note: Some(note), change_id, failed: false },
            },
            Err(e) => fail(op, &shown, e),
        }
    }

    /// Which folder a path is in, and the rest of it.
    fn locate(&self, full: &str) -> Result<(&ChatFolder, PathBuf), String> {
        let path = full.trim().trim_start_matches(['/', '\\']).replace('\\', "/");
        let (first, rest) = path.split_once('/').unwrap_or((path.as_str(), ""));
        let named = self.folders.iter().find(|f| f.name.eq_ignore_ascii_case(first));
        let (folder, rest) = match named {
            Some(f) => (f, rest),
            // One folder: a path without its name means inside it.
            None if self.folders.len() == 1 => (&self.folders[0], path.as_str()),
            None => {
                let names: Vec<&str> = self.folders.iter().map(|f| f.name.as_str()).collect();
                return Err(format!("Start the path with one of the folders: {}.", names.join(", ")));
            }
        };
        let inner = chat_files::safe_path(rest).ok_or("That path leaves the folder.")?;
        Ok((folder, inner))
    }

    fn server_root(&self, folder: &ChatFolder) -> PathBuf {
        chat_files::folder_dir(&self.state.config.files_dir(), &self.account_id, &folder.id)
    }

    async fn pc(&self, folder: &ChatFolder, op: &str, inner: &Path, extra: Value) -> Result<Value, String> {
        let mut req = json!({ "folder": folder.id, "op": op, "path": slash(inner) });
        if let (Some(o), Some(e)) = (req.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        self.state.file_relay.ask(&self.events, &self.chat_id, req).await
    }

    async fn list(&self, folder: &ChatFolder, inner: &Path) -> Result<(String, usize), String> {
        let entries: Vec<(String, bool)> = if folder.kind == "pc" {
            let v = self.pc(folder, "list", inner, json!({})).await?;
            v.get("entries")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|e| Some((e.get("path")?.as_str()?.to_string(), e.get("dir").and_then(Value::as_bool).unwrap_or(false))))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            let root = self.server_root(folder);
            let start = within(&root, inner)?;
            tokio::task::spawn_blocking(move || list_dir(&start, 2, LIST_LIMIT))
                .await
                .map_err(|e| e.to_string())??
        };
        let n = entries.len();
        let base = join(&folder.name, inner);
        if n == 0 {
            return Ok((format!("{base} is empty."), 0));
        }
        let lines: Vec<String> = entries.iter().map(|(p, dir)| if *dir { format!("{p}/") } else { p.clone() }).collect();
        let more = if n >= LIST_LIMIT { "\n(and more: list a sub-folder to see the rest)" } else { "" };
        Ok((format!("In {base}:\n{}{more}", lines.join("\n")), n))
    }

    /// A file's text (`None`: no such file).
    async fn read_text(&self, folder: &ChatFolder, inner: &Path) -> Result<Option<String>, String> {
        if inner.as_os_str().is_empty() {
            return Err("That's the folder itself: give a file's path.".into());
        }
        if folder.kind == "pc" {
            let v = self.pc(folder, "read", inner, json!({})).await?;
            if v.get("missing").and_then(Value::as_bool) == Some(true) {
                return Ok(None);
            }
            return v.get("text").and_then(Value::as_str).map(|t| Some(t.to_string())).ok_or("The app sent no text.".into());
        }
        let file = within(&self.server_root(folder), inner)?;
        match tokio::fs::read(&file).await {
            Ok(bytes) => text_of(bytes).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::IsADirectory || file.is_dir() => Err("That's a folder: use list_files.".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn search(&self, folder: &ChatFolder, inner: &Path, text: &str) -> Result<(String, usize), String> {
        if text.trim().is_empty() {
            return Err("Say what text to find.".into());
        }
        let hits: Vec<(String, usize, String)> = if folder.kind == "pc" {
            let v = self.pc(folder, "search", inner, json!({ "text": text })).await?;
            v.get("hits")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|h| {
                            Some((
                                h.get("path")?.as_str()?.to_string(),
                                h.get("line")?.as_u64()? as usize,
                                h.get("text")?.as_str()?.to_string(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            let start = within(&self.server_root(folder), inner)?;
            let needle = text.to_lowercase();
            tokio::task::spawn_blocking(move || search_dir(&start, &needle, SEARCH_LIMIT))
                .await
                .map_err(|e| e.to_string())?
        };
        let base = join(&folder.name, inner);
        if hits.is_empty() {
            return Ok((format!("No matches for \"{text}\" in {base}."), 0));
        }
        let lines: Vec<String> = hits
            .iter()
            .map(|(p, n, t)| format!("{}:{n}: {}", join(&base, Path::new(p)), t.trim().chars().take(200).collect::<String>()))
            .collect();
        let n = hits.len();
        let more = if n >= SEARCH_LIMIT { "\n(more matches: search a smaller folder or more exact text)" } else { "" };
        Ok((format!("{}{more}", lines.join("\n")), n))
    }

    /// Make (or propose) a change. `make` gets what's there now and returns the new text.
    async fn change(
        &self,
        folder: &ChatFolder,
        inner: &Path,
        make: impl FnOnce(Option<String>) -> Result<String, String>,
    ) -> Result<(String, String, Option<String>), String> {
        let before = self.read_text(folder, inner).await?;
        let after = make(before.clone())?;
        if after.len() > WRITE_LIMIT {
            return Err("That's too big for one file (1 MB at most).".into());
        }
        let shown = join(&folder.name, inner);
        if before.as_deref() == Some(after.as_str()) {
            return Ok((format!("{shown} already says that: nothing to change."), "no change".into(), None));
        }
        let (added, removed) = line_counts(before.as_deref(), &after);
        let rel = slash(inner);
        if self.mode == "auto" {
            if folder.kind == "pc" {
                self.pc(folder, "write", inner, json!({ "content": after })).await?;
            } else {
                write_server(&self.server_root(folder), inner, &after).await?;
                let _ = chat_files::touch_folder(&self.state.pool, &folder.id).await;
            }
            let c = chat_files::add_change(&self.state.pool, &self.chat_id, folder, &rel, before.as_deref(), &after, "applied")
                .await
                .map_err(|e| e.to_string())?;
            let what = if before.is_none() { "Created" } else { "Saved" };
            Ok((format!("{what} {shown} (+{added} −{removed} lines)."), format!("+{added} −{removed}"), Some(c.id)))
        } else {
            let c = chat_files::add_change(&self.state.pool, &self.chat_id, folder, &rel, before.as_deref(), &after, "proposed")
                .await
                .map_err(|e| e.to_string())?;
            Ok((
                format!(
                    "Proposed a change to {shown} (+{added} −{removed} lines). The user will apply or skip it; the file \
                     stays as it was until then, so don't read it back expecting the change."
                ),
                format!("+{added} −{removed}"),
                Some(c.id),
            ))
        }
    }
}

pub(crate) fn fail(op: &str, path: &str, why: String) -> Outcome {
    Outcome {
        reply: format!("That didn't work: {why}"),
        action: FileAction { op: op.into(), path: path.into(), note: Some(why), change_id: None, failed: true },
    }
}

fn slash(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/")
}

fn join(base: &str, p: &Path) -> String {
    let rest = slash(p);
    if rest.is_empty() {
        base.to_string()
    } else {
        format!("{base}/{rest}")
    }
}

/// `root/inner`, refusing anything that would end up outside `root` (through a link, say).
pub fn within(root: &Path, inner: &Path) -> Result<PathBuf, String> {
    // A new server folder has no files yet.
    let _ = std::fs::create_dir_all(root);
    let full = root.join(inner);
    // The deepest part that exists must still be inside the folder.
    let mut probe = full.clone();
    while !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    if probe.exists() {
        let real_root = root.canonicalize().map_err(|_| "The folder is missing.".to_string())?;
        let real = probe.canonicalize().map_err(|e| e.to_string())?;
        if !real.starts_with(&real_root) {
            return Err("That path leaves the folder.".into());
        }
    }
    Ok(full)
}

/// Text from a file's bytes, or why it isn't text.
pub fn text_of(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err("That's not a text file.".into());
    }
    String::from_utf8(bytes).map_err(|_| "That's not a UTF-8 text file.".into())
}

/// A file's text with line numbers, from `from` (1-based), cut at [`READ_LIMIT`].
pub fn numbered(shown: &str, text: &str, from: usize) -> (String, String, Option<String>) {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return (format!("{shown} is empty."), "empty".into(), None);
    }
    let start = from.min(total);
    let mut out = String::new();
    let mut last = start - 1;
    for (i, l) in lines.iter().enumerate().skip(start - 1) {
        let line = format!("{:>5}  {l}\n", i + 1);
        if out.len() + line.len() > READ_LIMIT && i + 1 > start {
            break;
        }
        out.push_str(&line);
        last = i + 1;
    }
    let more = if last < total {
        format!("\n(lines {start}–{last} of {total}; read again with from_line {} for more)", last + 1)
    } else {
        String::new()
    };
    (format!("{shown} ({total} lines):\n{out}{more}"), format!("lines {start}–{last} of {total}"), None)
}

/// Lines added and removed (roughly: by counting lines in each, ignoring order).
pub fn line_counts(before: Option<&str>, after: &str) -> (usize, usize) {
    let mut old: HashMap<&str, isize> = HashMap::new();
    for l in before.unwrap_or_default().lines() {
        *old.entry(l).or_default() += 1;
    }
    let mut added = 0;
    for l in after.lines() {
        match old.get_mut(l) {
            Some(n) if *n > 0 => *n -= 1,
            _ => added += 1,
        }
    }
    let removed = old.values().filter(|n| **n > 0).map(|n| *n as usize).sum();
    (added, removed)
}

/// Write a server folder's file (via a temporary name, so a cut-off write leaves the old one).
pub async fn write_server(root: &Path, inner: &Path, text: &str) -> Result<(), String> {
    tokio::fs::create_dir_all(root).await.map_err(|e| e.to_string())?;
    let file = within(root, inner)?;
    if let Some(dir) = file.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    let part = file.with_extension(format!(
        "{}outpost-part",
        file.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()
    ));
    tokio::fs::write(&part, text).await.map_err(|e| e.to_string())?;
    tokio::fs::rename(&part, &file).await.map_err(|e| e.to_string())
}

/// Entries under `start`, `depth` levels down, as paths relative to it (folders marked).
pub fn list_dir(start: &Path, depth: usize, limit: usize) -> Result<Vec<(String, bool)>, String> {
    if !start.exists() {
        return Err("There's no such folder.".into());
    }
    if start.is_file() {
        return Err("That's a file: use read_file.".into());
    }
    let mut out = Vec::new();
    fn walk(dir: &Path, rel: &str, depth: usize, limit: usize, out: &mut Vec<(String, bool)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));
        for e in entries {
            if out.len() >= limit {
                return;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.path().is_dir();
            if is_dir && SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            out.push((path.clone(), is_dir));
            if is_dir && depth > 1 {
                walk(&e.path(), &path, depth - 1, limit, out);
            }
        }
    }
    walk(start, "", depth, limit, &mut out);
    Ok(out)
}

/// Lines containing `needle` (lower case) in text files under `start`.
pub fn search_dir(start: &Path, needle: &str, limit: usize) -> Vec<(String, usize, String)> {
    let mut hits = Vec::new();
    let mut seen = 0usize;
    fn walk(dir: &Path, rel: &str, needle: &str, limit: usize, seen: &mut usize, hits: &mut Vec<(String, usize, String)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if hits.len() >= limit || *seen > 5000 {
                return;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let p = e.path();
            if p.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    walk(&p, &path, needle, limit, seen, hits);
                }
                continue;
            }
            *seen += 1;
            if e.metadata().map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read(&p).map_err(|_| ()).and_then(|b| text_of(b).map_err(|_| ())) else { continue };
            for (i, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(needle) {
                    hits.push((path.clone(), i + 1, line.to_string()));
                    if hits.len() >= limit {
                        return;
                    }
                }
            }
        }
    }
    if start.is_file() {
        if let Some(name) = start.file_name() {
            let name = name.to_string_lossy().to_string();
            if let Ok(text) = std::fs::read(start).map_err(|_| ()).and_then(|b| text_of(b).map_err(|_| ())) {
                for (i, line) in text.lines().enumerate() {
                    if line.to_lowercase().contains(needle) && hits.len() < limit {
                        hits.push((name.clone(), i + 1, line.to_string()));
                    }
                }
            }
        }
        return hits;
    }
    walk(start, "", needle, limit, &mut seen, &mut hits);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_shows_numbered_lines_in_parts() {
        let text: String = (1..=5000).map(|i| format!("line number {i}\n")).collect();
        let (out, note, _) = numbered("Shop/a.txt", &text, 1);
        assert!(out.starts_with("Shop/a.txt (5000 lines):\n    1  line number 1\n"));
        assert!(out.contains("read again with from_line"));
        assert!(out.len() < READ_LIMIT + 300);
        assert!(note.starts_with("lines 1–"));
        let (out, _, _) = numbered("Shop/a.txt", &text, 4999);
        assert!(out.contains(" 5000  line number 5000") && !out.contains("read again"));
    }

    #[test]
    fn changes_are_counted_by_line() {
        assert_eq!(line_counts(None, "a\nb\n"), (2, 0));
        assert_eq!(line_counts(Some("a\nb\nc\n"), "a\nB\nc\nd\n"), (2, 1));
        assert_eq!(text_of(vec![0, 1, 2]).unwrap_err(), "That's not a text file.");
    }

    #[tokio::test]
    async fn server_folders_are_listed_searched_and_written_inside_themselves() {
        let root = std::env::temp_dir().join(format!("outpost-ws-{}", uuid::Uuid::new_v4()));
        write_server(&root, Path::new("src/app.ts"), "export const hello = 1;\n").await.unwrap();
        write_server(&root, Path::new("node_modules/x/index.js"), "hello\n").await.unwrap();
        let listed = list_dir(&root, 2, 100).unwrap();
        assert!(listed.contains(&("src".into(), true)));
        assert!(listed.contains(&("src/app.ts".into(), false)));
        assert!(!listed.iter().any(|(p, _)| p.starts_with("node_modules/")), "skips node_modules: {listed:?}");
        let hits = search_dir(&root, "hello", 10);
        assert_eq!(hits, vec![("src/app.ts".into(), 1, "export const hello = 1;".into())]);
        assert!(within(&root, Path::new("src/new.ts")).is_ok());
        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join(format!("outpost-outside-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
            assert!(within(&root, Path::new("escape/x.txt")).is_err(), "a link out of the folder is refused");
        }
    }
}
