//! Files in chats, the model's side: the tools it's given for a chat's folders (list, read,
//! search, write, edit), and running them.
//!
//! Server folders are read and written here. A PC folder is read and written by the desktop
//! app on that PC: while it streams the reply, a `file_request` event goes to the app, which
//! does it and posts the answer back (see [`Relay`]); a reply streamed elsewhere (the phone)
//! reaches it through the PC's open app (see [`crate::device_tools`]). In `ask` mode a change
//! is only proposed (the person applies or skips it later); in `auto` it's made at once, with Undo.
//!
//! Commands (`run_command`, e.g. `gradlew build`) run in a PC folder, on that PC. Ones the
//! person hasn't allowed for the folder ask them first (Allow, Always allow, Skip).

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
/// Longest a command may run (the model can ask for less).
const COMMAND_MAX_SECS: u64 = 30 * 60;
/// What a command gets unless the model says.
const COMMAND_SECS: u64 = 10 * 60;
/// Longest to wait for the person to allow a command.
const CONFIRM_WITHIN: Duration = Duration::from_secs(5 * 60);
/// Most of a command's output the model is shown.
const OUTPUT_LIMIT: usize = 6_000;
/// Folders not worth looking in.
pub const SKIP_DIRS: [&str; 12] =
    [".git", "node_modules", "target", "dist", "build", ".next", ".venv", "venv", "__pycache__", ".idea", ".vs", ".gradle"];

/// Chats whose reply the person stopped (Stop): the reply ends at its next step, and a
/// command it's running is stopped on the PC.
static STOPPED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// The person pressed Stop on this chat's reply.
pub fn request_stop(chat_id: &str) {
    STOPPED.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(Default::default).insert(chat_id.to_string());
}

/// Was this chat's reply stopped?
pub fn stopped(chat_id: &str) -> bool {
    STOPPED.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|s| s.contains(chat_id))
}

/// A new reply starts: forget an earlier Stop.
pub fn clear_stop(chat_id: &str) {
    if let Some(s) = STOPPED.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        s.remove(chat_id);
    }
}

async fn until_stopped(chat_id: &str) {
    while !stopped(chat_id) {
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

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

    /// Ask the app streaming this reply to do something (a `file_request` event) and wait for
    /// its answer; an `error` in the answer comes back as the error.
    pub(crate) async fn ask_within(
        &self,
        events: &mpsc::Sender<Event>,
        chat_id: &str,
        request: Value,
        within: Duration,
        event: &str,
    ) -> Result<Value, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), (chat_id.to_string(), tx));
        let mut body = request;
        body["id"] = json!(id);
        let sent = events.send(Event::default().event(event).data(body.to_string())).await;
        let answer = if sent.is_err() {
            Err("The app closed the chat.".to_string())
        } else {
            match tokio::time::timeout(within, rx).await {
                Ok(Ok(v)) => Ok(v),
                _ => Err("The desktop app didn't answer in time (closed, or what it was asked for isn't there any more).".to_string()),
            }
        };
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
        if event == "confirm" {
            // Answered (on one device) or given up on: every device showing it puts it away.
            let _ = events.send(Event::default().event("answered").data(json!({ "id": id }).to_string())).await;
        }
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
    /// Commands may run in server folders for this account (owners; others when sandboxed).
    pub server_commands: bool,
}

/// What a tool call came to: the model's answer, and what to show under the reply.
pub struct Outcome {
    pub reply: String,
    pub action: FileAction,
    /// More to show under the reply (files a build made, to download).
    pub more: Vec<FileAction>,
}

impl Workspace {
    /// The system prompt's part about the folders.
    pub fn instructions(&self) -> String {
        let list: Vec<String> = self
            .folders
            .iter()
            .map(|f| match f.kind.as_str() {
                "pc" if !self.can_reach(f) => format!(
                    "- {} (on {}; not reachable now: that PC needs to be on, with the Outpost app open)",
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
             change was made unless a tool said so.{commands}",
            list.join("\n"),
            commands = self.command_instructions()
        )
    }

    fn command_instructions(&self) -> String {
        if !self.has_commands() {
            return String::new();
        }
        let mut t = String::from(
            "\n\nYou can run commands in a folder with run_command (path = the folder, or a sub-folder to run in), e.g. \
             `gradlew build`, `mvn package` or `npm test`. Use it to build and test your changes: read the errors it \
             shows, fix the code, and run it again until it works (say so if you can't get it working). run_command is \
             for commands that finish by themselves. For something that keeps running (a test Minecraft server, a dev \
             server), use start_process, then process_output to read its log (wait for it to finish starting: read \
             again if it isn't ready), send_to_process to type console commands into it, and stop_process when you're \
             done (always stop what you started). Run one command per call, without && or | (chained commands always \
             need the user's OK each time). The user may be asked to allow a command first; if they skip it, don't run \
             it again. Never say a build passed unless run_command showed it.",
        );
        if self.server_commands && self.folders.iter().any(|f| f.kind != "pc") {
            t.push_str(
                "\n\nFolders on the Outpost server run on Linux: use `./gradlew build` (or `gradlew build`, which \
                 Outpost runs as ./gradlew), and `/` in paths. Java and Gradle are provided (Outpost downloads them the first \
                 time; Java 21, or 25 when the build files ask for it), so you can start a new project in an \
                 empty folder: write settings.gradle.kts and build.gradle.kts (for a Paper plugin, the \
                 papermc repository and paper-api as compileOnly), then run `gradle build`, or `gradle wrapper` \
                 once to add a wrapper. Never try to install software (apt, sudo, curl | sh): it isn't allowed \
                 and isn't needed. Maven isn't installed: use Gradle. What a build makes (the .jar in build/libs \
                 or target) is offered to the user to download.",
            );
        }
        t
    }

    /// The tool definitions for Ollama (with the command tools when commands can run).
    pub fn tools(&self) -> Value {
        let mut all = Self::file_tools();
        if self.has_commands() {
            if let Some(a) = all.as_array_mut() {
                let path = json!({ "type": "string", "description": "The folder it runs in (its name, or name/sub-folder), e.g. MyPlugin/run" });
                let name = json!({ "type": "string", "description": "The name you gave it when starting it, e.g. test-server" });
                a.push(json!({ "type": "function", "function": {
                    "name": "start_process",
                    "description": "Start something that keeps running (a test Minecraft server, a dev server) in a folder. It runs in the background: read what it prints with process_output, type into its console with send_to_process, and stop it with stop_process when done.",
                    "parameters": { "type": "object", "properties": {
                        "path": path,
                        "command": { "type": "string", "description": "The command line, e.g. java -Xmx2G -jar paper.jar nogui" },
                        "name": { "type": "string", "description": "A short name for it, e.g. test-server" } },
                        "required": ["path", "command", "name"] } } }));
                a.push(json!({ "type": "function", "function": {
                    "name": "process_output",
                    "description": "The latest lines a started process printed (its log), and whether it's still running.",
                    "parameters": { "type": "object", "properties": {
                        "path": path, "name": name,
                        "lines": { "type": "integer", "description": "How many of the last lines (default 60)." } },
                        "required": ["path", "name"] } } }));
                a.push(json!({ "type": "function", "function": {
                    "name": "send_to_process",
                    "description": "Type a line into a started process's console (e.g. a server command like `plugins` or `stop`), and see what it printed after.",
                    "parameters": { "type": "object", "properties": {
                        "path": path, "name": name,
                        "text": { "type": "string", "description": "The line to type." } },
                        "required": ["path", "name", "text"] } } }));
                a.push(json!({ "type": "function", "function": {
                    "name": "stop_process",
                    "description": "Stop a started process (and everything it started).",
                    "parameters": { "type": "object", "properties": { "path": path, "name": name }, "required": ["path", "name"] } } }));
                a.push(json!({ "type": "function", "function": {
                    "name": "run_command",
                    "description": "Run a command in a folder and get its exit code and output (errors first when it's long). For builds and tests, e.g. gradlew build. It must finish by itself.",
                    "parameters": { "type": "object", "properties": {
                        "path": { "type": "string", "description": "The folder to run in (its name, or name/sub-folder), e.g. MyPlugin" },
                        "command": { "type": "string", "description": "The command line, e.g. gradlew build" },
                        "timeout_minutes": { "type": "integer", "description": "Stop it after this many minutes (default 10, at most 30)." } },
                        "required": ["path", "command"] } } }));
            }
        }
        all
    }

    fn file_tools() -> Value {
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
                "description": "Create a file, or replace a file's whole content. Folders are made by themselves when you write a file inside them (write src/main/java/App.java directly): never write a file just to make a folder.",
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
            "run_command" => "run",
            "start_process" => "start",
            "process_output" => "output",
            "send_to_process" => "input",
            "stop_process" => "stop",
            _ => {
                return fail("tool", &full, format!("There's no tool called {name}."));
            }
        };
        let (folder, inner) = match self.locate(&full) {
            Ok(x) => x,
            Err(e) => return fail(op, &full, e),
        };
        let shown = join(&folder.name, &inner);
        if !self.can_reach(folder) {
            return fail(
                op,
                &shown,
                format!(
                    "{} is on {}, which isn't reachable now: it needs that PC on, with the Outpost app open.",
                    folder.name,
                    folder.device.as_deref().unwrap_or("another PC")
                ),
            );
        }
        if matches!(op, "run" | "start" | "output" | "input" | "stop") {
            if folder.kind != "pc" && !self.server_commands {
                return fail(op, &shown, "Commands can't run in server folders from this account.".into());
            }
            // Asking first, with changes waiting: a build would use the old files. (Saving
            // straight away, older waiting changes don't hold anything up.)
            if matches!(op, "run" | "start") && self.mode != "auto" {
                let waiting = self.waiting_in(folder).await;
                if !waiting.is_empty() {
                    let n = waiting.len();
                    return Outcome {
                        reply: format!(
                            "Not run: {n} change{} in {} {} still waiting for the user to apply ({}), so the command \
                             would use the old files. Stop now: tell the user to apply them (with Apply all \
                             in the \"waiting for you\" bar above the message box) and say \"carry on\", then run it. Don't change more files or run anything else until they do.",
                            if n == 1 { "" } else { "s" },
                            folder.name,
                            if n == 1 { "is" } else { "are" },
                            waiting.join(", ")
                        ),
                        action: FileAction {
                            op: op.into(),
                            path: shown.clone(),
                            note: Some(format!("waiting for {n} change{} to be applied", if n == 1 { "" } else { "s" })),
                            change_id: None,
                            failed: true,
                        },
                        more: Vec::new(),
                    };
                }
            }
            if op == "run" {
                return self.run_command(folder, &inner, &shown, args).await;
            }
            return self.process(op, folder, &inner, &shown, args).await;
        }
        let result = match op {
            "list" => self.list(folder, &inner).await.map(|(text, n)| (text, format!("{n} entries"), None)),
            "read" => {
                let from = args.get("from_line").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
                // Asking first: the model sees its own changes still waiting (saving straight away,
                // the file is what's saved).
                let waiting = if self.mode == "auto" { None } else { self.waiting_change(folder, &slash(&inner)).await };
                let text = match &waiting {
                    Some(c) => Ok(Some(c.after.clone())),
                    None => self.read_text(folder, &inner).await,
                };
                text.and_then(|t| match t {
                    Some(t) => Ok(numbered(&shown, &t, from)),
                    None => Err("There's no such file.".to_string()),
                })
                .map(|(text, note, id)| match waiting {
                    Some(_) => (format!("(This includes your changes still waiting for the user to apply.)\n{text}"), note, id),
                    None => (text, note, id),
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
                more: Vec::new(),
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

    /// A PC folder is usable when the app streaming this reply is on that PC, or that PC's
    /// app is open (it's reached through it).
    fn can_reach(&self, folder: &ChatFolder) -> bool {
        folder.kind != "pc" || self.reachable.contains(&folder.id) || self.state.device_tools.for_folder(&self.account_id, &folder.id).is_some()
    }

    /// Commands can run: a folder on a PC, or a server folder this account may run them in.
    pub fn has_commands(&self) -> bool {
        self.folders.iter().any(|f| f.kind == "pc" || self.server_commands)
    }

    fn server_root(&self, folder: &ChatFolder) -> PathBuf {
        chat_files::folder_dir(&self.state.config.files_dir(), &self.account_id, &folder.id)
    }

    async fn pc(&self, folder: &ChatFolder, op: &str, inner: &Path, extra: Value) -> Result<Value, String> {
        self.pc_within(folder, op, inner, extra, Duration::from_secs(60)).await
    }

    async fn pc_within(&self, folder: &ChatFolder, op: &str, inner: &Path, extra: Value, within: Duration) -> Result<Value, String> {
        let mut req = json!({ "folder": folder.id, "op": op, "path": slash(inner) });
        if let (Some(o), Some(e)) = (req.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        if self.reachable.contains(&folder.id) {
            return self.state.file_relay.ask_within(&self.events, &self.chat_id, req, within, "file_request").await;
        }
        // From elsewhere: through that PC's open app.
        match self.state.device_tools.for_folder(&self.account_id, &folder.id) {
            Some((session, _)) => {
                req["kind"] = json!("folder");
                self.state.device_tools.call(&session, req, within).await
            }
            None => Err(format!(
                "{} is on {}, which isn't reachable now (is it on, with the Outpost app open?).",
                folder.name,
                folder.device.as_deref().unwrap_or("another PC")
            )),
        }
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
    /// This chat's change to that file still waiting for the user, if any.
    async fn waiting_change(&self, folder: &ChatFolder, rel: &str) -> Option<chat_files::FileChange> {
        chat_files::changes(&self.state.pool, &self.chat_id)
            .await
            .ok()?
            .into_iter()
            .rev()
            .find(|c| c.status == "proposed" && c.folder_id == folder.id && c.kind == folder.kind && c.path == rel)
    }

    /// Changes in this folder still waiting for the user: a command would run on the old files.
    async fn waiting_in(&self, folder: &ChatFolder) -> Vec<String> {
        chat_files::changes(&self.state.pool, &self.chat_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.status == "proposed" && c.folder_id == folder.id && c.kind == folder.kind)
            .map(|c| c.path)
            .collect()
    }

    /// A file can't go where a folder is (or will be), or inside something that's a file: the
    /// folder as it is, and the changes still waiting in this chat.
    async fn check_shape(&self, folder: &ChatFolder, inner: &Path, rel: &str) -> Result<(), String> {
        let shown = join(&folder.name, inner);
        let folder_msg = format!(
            "{shown} is a folder, so it can't be written as a file. Folders are made by themselves when you write a file \
             inside one: write each file at its full path (e.g. {shown}/src/Main.java)."
        );
        if folder.kind != "pc" {
            let root = self.server_root(folder);
            let target = within(&root, inner)?;
            if target.is_dir() {
                return Err(folder_msg);
            }
            let mut up = target.parent();
            while let Some(p) = up {
                if p == root || !p.starts_with(&root) {
                    break;
                }
                if p.is_file() {
                    let name = p.strip_prefix(&root).map(slash).unwrap_or_default();
                    return Err(format!(
                        "{} is a file, so nothing can go inside it. If it was meant to be a folder, delete that file first.",
                        join(&folder.name, Path::new(&name))
                    ));
                }
                up = p.parent();
            }
        }
        let waiting = chat_files::changes(&self.state.pool, &self.chat_id).await.map_err(|e| e.to_string())?;
        for c in waiting.iter().filter(|c| c.status == "proposed" && c.folder_id == folder.id && c.kind == folder.kind) {
            if c.path.starts_with(&format!("{rel}/")) {
                return Err(folder_msg);
            }
            if rel.starts_with(&format!("{}/", c.path)) {
                return Err(format!(
                    "A change waiting for the user writes {} as a file, so {shown} can't go inside it. Folders are made by \
                     themselves: don't write a file to make a folder.",
                    join(&folder.name, Path::new(&c.path))
                ));
            }
        }
        Ok(())
    }

    async fn change(
        &self,
        folder: &ChatFolder,
        inner: &Path,
        make: impl FnOnce(Option<String>) -> Result<String, String>,
    ) -> Result<(String, String, Option<String>), String> {
        let rel = slash(inner);
        self.check_shape(folder, inner, &rel).await?;
        // A change to this file is already waiting: build on it, and keep one change per file.
        let waiting = if self.mode == "auto" { None } else { self.waiting_change(folder, &rel).await };
        if let Some(w) = waiting {
            let shown = join(&folder.name, inner);
            let after = make(Some(w.after.clone()))?;
            if after.len() > WRITE_LIMIT {
                return Err("That's too big for one file (1 MB at most).".into());
            }
            if after == w.after {
                return Ok((format!("{shown} already says that: nothing to change."), "no change".into(), None));
            }
            let c = chat_files::update_proposed(&self.state.pool, &self.chat_id, &w.id, &after).await.map_err(|e| e.to_string())?;
            let (added, removed) = line_counts(c.before.as_deref(), &after);
            return Ok((
                format!(
                    "Updated your waiting change to {shown} (now +{added} −{removed} lines from the original). It's still \
                     waiting for the user to apply it; reading the file shows your version."
                ),
                format!("+{added} −{removed}"),
                Some(c.id),
            ));
        }
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
                    "Proposed a change to {shown} (+{added} −{removed} lines). It's waiting for the user to apply it: \
                     reading the file shows your version, but commands (builds) can't run in this folder until the \
                     user has applied or skipped the waiting changes."
                ),
                format!("+{added} −{removed}"),
                Some(c.id),
            ))
        }
    }
}

impl Workspace {
    /// May this command run? Asks the person unless it's allowed in the folder. `Err` is the
    /// outcome to give the model (skipped, no answer...).
    async fn permit(&self, folder: &ChatFolder, shown: &str, op: &str, command: &str, verb: &str) -> Result<(), Outcome> {
        let note = |n: &str| FileAction { op: op.into(), path: shown.to_string(), note: Some(n.into()), change_id: None, failed: true };
        if let Some(why) = refused(command) {
            return Err(fail(op, shown, format!("That command isn't allowed ({why}).")));
        }
        let rules = CommandRules::load(&self.state, &folder.id).await;
        if rules.allows(command) {
            return Ok(());
        }
        let always = always_label(command);
        let question = format!("{verb} `{command}` in {shown}?");
        let asked = json!({ "kind": "command", "question": question, "always": always, "command": command, "folder": folder.name });
        let answer = self.state.file_relay.ask_within(&self.events, &self.chat_id, asked, CONFIRM_WITHIN, "confirm").await;
        let ok = answer.as_ref().is_ok_and(|v| v.get("ok").and_then(Value::as_bool) == Some(true));
        if !ok {
            let cannot = answer.as_ref().is_ok_and(|v| v.get("cannot_ask").and_then(Value::as_bool) == Some(true));
            let (reply, why) = match (&answer, cannot) {
                (_, true) => (
                    format!("`{command}` didn't run: the app the user is chatting in can't show the request to run commands (it may need updating). Tell the user."),
                    "couldn't ask (update the app)",
                ),
                (Err(_), _) => (
                    format!("`{command}` didn't run: nobody answered the request to run it. Tell the user it's waiting for their OK."),
                    "no answer",
                ),
                _ => (format!("The user chose Skip for `{command}`, so it didn't run. Don't run it again unless they ask."), "skipped"),
            };
            return Err(Outcome { reply, action: note(why), more: Vec::new() });
        }
        if answer.is_ok_and(|v| v.get("always").and_then(Value::as_bool) == Some(true)) {
            if let Some(program) = always {
                let _ = rules.with(&program).save(&self.state, &folder.id).await;
            }
        }
        Ok(())
    }

    /// How a command runs in a server folder: where, with which Java, sandboxed or not.
    async fn server_spec(&self, folder: &ChatFolder, inner: &Path, command: &str) -> Result<crate::runner::Spec, String> {
        let account = outpost_core::accounts::get(&self.state.pool, &self.account_id).await.map_err(|e| e.to_string())?;
        let sandboxed = crate::runner::sandbox_available();
        match account.role {
            outpost_core::accounts::Role::Restricted | outpost_core::accounts::Role::Service => {
                return Err("Commands can't run in server folders from this account.".into())
            }
            outpost_core::accounts::Role::Member if !sandboxed => {
                return Err("Commands in server folders need the server's sandbox (bubblewrap, `sudo apt install bubblewrap`), or an owner's account. Ask an owner.".into())
            }
            _ => {}
        }
        let root = self.server_root(folder);
        let dir = within(&root, inner)?;
        if !dir.is_dir() {
            return Err("That isn't a folder (yet): give the folder to run in.".into());
        }
        let real_root = root.canonicalize().map_err(|e| e.to_string())?;
        // The same place by its real path (what the sandbox shows).
        let dir = real_root.join(inner);
        let home = self.state.config.data_dir.join("build-home").join(&self.account_id);
        tokio::fs::create_dir_all(&home).await.map_err(|e| e.to_string())?;
        let mut command = command.to_string();
        // `gradlew build` on Linux is `./gradlew build` (and the wrapper must be runnable).
        if let Some(program) = program_of(&command) {
            if program == "gradlew" || program == "mvnw" {
                let script = dir.join(&program);
                if script.is_file() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755));
                    }
                    let rest = command.split_once(char::is_whitespace).map(|(_, r)| r.to_string()).unwrap_or_default();
                    command = format!("./{program} {rest}").trim_end().to_string();
                }
            }
        }
        let mut env = vec![
            ("HOME".to_string(), home.to_string_lossy().to_string()),
            ("GRADLE_USER_HOME".to_string(), home.join(".gradle").to_string_lossy().to_string()),
        ];
        let mut tools = Vec::new();
        let engines = self.state.config.data_dir.join("engines");
        // Java and Gradle for Java work: downloaded the first time they're needed, then
        // always there (so a new project in an empty folder can be started with them).
        let java_ready = (17..=40).any(|v| outpost_engines::runtimes::java_home(&engines, v).is_some());
        if java_ready || wants_java(&dir, &command) {
            let mut build = String::new();
            for f in ["build.gradle.kts", "build.gradle", "pom.xml"] {
                if let Ok(t) = tokio::fs::read_to_string(dir.join(f)).await {
                    build.push_str(&t);
                }
            }
            let version = outpost_engines::runtimes::java_for_project(&build);
            let client = reqwest::Client::new();
            let java = outpost_engines::runtimes::ensure_java(&client, &engines, version, |_, _| {})
                .await
                .map_err(|e| format!("Couldn't get Java {version} for the build: {e:#}"))?;
            let gradle = outpost_engines::runtimes::ensure_gradle(&client, &engines, |_, _| {})
                .await
                .map_err(|e| format!("Couldn't get Gradle for the build: {e:#}"))?;
            let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
            env.push(("JAVA_HOME".into(), java.to_string_lossy().to_string()));
            env.push((
                "PATH".into(),
                format!("{}:{}:{path}", java.join("bin").to_string_lossy(), gradle.join("bin").to_string_lossy()),
            ));
            tools.push(java);
            tools.push(gradle);
        }
        let sandbox = sandboxed.then(|| crate::runner::Sandbox { folder: real_root, home: home.clone(), tools, network: true });
        Ok(crate::runner::Spec { dir, command, env, sandbox })
    }

    /// Run a command in a folder (on its PC, or here for a server folder), asking first
    /// unless it's allowed there.
    async fn run_command(&self, folder: &ChatFolder, inner: &Path, shown: &str, args: &Value) -> Outcome {
        let command = args.get("command").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        let note = |n: String| FileAction { op: "run".into(), path: shown.to_string(), note: Some(n), change_id: None, failed: false };
        if command.is_empty() {
            return fail("run", shown, "Say which command to run.".into());
        }
        let mins = args.get("timeout_minutes").and_then(Value::as_u64).unwrap_or(COMMAND_SECS / 60).clamp(1, COMMAND_MAX_SECS / 60);
        if let Err(o) = self.permit(folder, shown, "run", &command, "Run").await {
            return o;
        }
        let secs = mins * 60;
        let started = std::time::Instant::now();
        let since = std::time::SystemTime::now();
        let run_id = uuid::Uuid::new_v4().simple().to_string();
        let got = if folder.kind == "pc" {
            let running =
                self.pc_within(folder, "run", inner, json!({ "command": command, "timeout": secs, "run_id": run_id }), Duration::from_secs(secs + 30));
            tokio::select! {
                got = running => got,
                _ = until_stopped(&self.chat_id) => {
                    // Stop it on the PC (through its open app: the reply's own stream may be gone).
                    if let Some((session, _)) = self.state.device_tools.for_folder(&self.account_id, &folder.id) {
                        let _ = self
                            .state
                            .device_tools
                            .call(&session, json!({ "kind": "folder", "folder": folder.id, "op": "cancel", "path": "", "run_id": run_id }), Duration::from_secs(15))
                            .await;
                    }
                    return Outcome {
                        reply: format!("The user pressed Stop, so `{command}` was stopped."),
                        action: FileAction { failed: true, ..note("stopped".into()) },
                        more: Vec::new(),
                    };
                }
            }
        } else {
            let spec = match self.server_spec(folder, inner, &command).await {
                Ok(s) => s,
                Err(e) => return fail("run", shown, e),
            };
            let running = crate::runner::run(&spec, secs, Some(run_id.clone()));
            tokio::select! {
                got = running => got,
                _ = until_stopped(&self.chat_id) => {
                    crate::runner::cancel(&run_id);
                    return Outcome {
                        reply: format!("The user pressed Stop, so `{command}` was stopped."),
                        action: FileAction { failed: true, ..note("stopped".into()) },
                        more: Vec::new(),
                    };
                }
            }
        };
        let took = started.elapsed().as_secs();
        match got {
            Err(e) => fail("run", shown, e),
            Ok(v) => {
                let output = v.get("output").and_then(Value::as_str).unwrap_or_default();
                let timed_out = v.get("timed_out").and_then(Value::as_bool) == Some(true);
                let code = v.get("exit_code").and_then(Value::as_i64);
                let how = match (timed_out, code) {
                    (true, _) => format!("was stopped after {mins} minutes (still running)"),
                    (false, Some(0)) => format!("finished OK in {}", seconds(took)),
                    (false, Some(c)) => format!("failed with exit code {c} after {}", seconds(took)),
                    (false, None) => format!("ended after {}", seconds(took)),
                };
                let ok = !timed_out && code == Some(0);
                let mut reply = format!("`{command}` in {shown} {how}.\n{}", summarise_output(output));
                // What it built, ready to download (server folders).
                let mut more = Vec::new();
                if ok && folder.kind != "pc" {
                    let root = self.server_root(folder);
                    if let Ok(dir) = within(&root, inner) {
                        let built = tokio::task::spawn_blocking(move || built_files(&root, &dir, since)).await.unwrap_or_default();
                        if !built.is_empty() {
                            let names: Vec<String> = built.iter().map(|p| join(&folder.name, Path::new(p))).collect();
                            reply.push_str(&format!(
                                "\n\nBuilt (the user can download these from the chat): {}",
                                names.join(", ")
                            ));
                            more = names
                                .into_iter()
                                .map(|path| FileAction { op: "download".into(), path, note: Some("built".into()), change_id: None, failed: false })
                                .collect();
                        }
                    }
                }
                Outcome {
                    reply,
                    action: FileAction { failed: !ok, ..note(format!("`{command}` {}", if ok { "passed".to_string() } else { how })) },
                    more,
                }
            }
        }
    }

    /// The long-running process tools: start one (asks like a command), read its output,
    /// type into it, stop it. On a PC they run there; for a server folder, here.
    async fn process(&self, op: &str, folder: &ChatFolder, inner: &Path, shown: &str, args: &Value) -> Outcome {
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
        let command = s("command");
        let name = {
            let n = s("name");
            let n = if n.is_empty() { program_of(&command).unwrap_or_else(|| "process".into()) } else { n };
            n.chars().filter(|c| c.is_alphanumeric() || "-_ .".contains(*c)).take(40).collect::<String>()
        };
        let key = format!("{}:{name}", folder.id);
        let action = |n: String, failed: bool| FileAction { op: op.into(), path: shown.to_string(), note: Some(n), change_id: None, failed };
        let result: Result<Value, String> = match op {
            "start" => {
                if command.is_empty() {
                    return fail(op, shown, "Say which command to start.".into());
                }
                if let Err(o) = self.permit(folder, shown, op, &command, "Start").await {
                    return o;
                }
                if folder.kind == "pc" {
                    self.pc_within(folder, "start", inner, json!({ "command": command, "name": name }), Duration::from_secs(60)).await
                } else {
                    match self.server_spec(folder, inner, &command).await {
                        Ok(spec) => crate::runner::start(&key, &spec).await,
                        Err(e) => Err(e),
                    }
                }
            }
            "output" => {
                let lines = args.get("lines").and_then(Value::as_u64).unwrap_or(60) as usize;
                if folder.kind == "pc" {
                    self.pc(folder, "output", inner, json!({ "name": name, "lines": lines })).await
                } else {
                    crate::runner::output(&key, lines, None)
                }
            }
            "input" => {
                let text = s("text");
                if let Some(why) = refused(&text) {
                    return fail(op, shown, format!("That isn't allowed ({why})."));
                }
                let sent = if folder.kind == "pc" {
                    self.pc(folder, "input", inner, json!({ "name": name, "text": text })).await
                } else {
                    crate::runner::input(&key, &text).await
                };
                match sent {
                    Ok(_) => {
                        // What it said back.
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        if folder.kind == "pc" {
                            self.pc(folder, "output", inner, json!({ "name": name, "lines": 20 })).await
                        } else {
                            crate::runner::output(&key, 20, None)
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            _ => {
                let stopped = if folder.kind == "pc" { self.pc(folder, "stop", inner, json!({ "name": name })).await } else { crate::runner::stop(&key) };
                match stopped {
                    Ok(_) => {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        Ok(json!({ "running": false, "output": "", "stopped": true }))
                    }
                    Err(e) => Err(e),
                }
            }
        };
        match result {
            Err(e) => fail(op, shown, e),
            Ok(v) => {
                let running = v.get("running").and_then(Value::as_bool) == Some(true);
                let out = v.get("output").and_then(Value::as_str).unwrap_or_default();
                let state = if op == "stop" {
                    format!("\"{name}\" was stopped.")
                } else if running {
                    format!("\"{name}\" is running.")
                } else {
                    match v.get("exit_code").and_then(Value::as_i64) {
                        Some(c) => format!("\"{name}\" has ended (exit code {c})."),
                        None => format!("\"{name}\" has ended."),
                    }
                };
                let reply = if out.trim().is_empty() || op == "stop" {
                    state.clone()
                } else {
                    format!("{state} Its latest output:\n{}", summarise_output(out))
                };
                let note = match op {
                    "start" => format!("started \"{name}\": `{command}`"),
                    "output" => format!("read \"{name}\""),
                    "input" => format!("typed `{}` into \"{name}\"", s("text")),
                    _ => format!("stopped \"{name}\""),
                };
                Outcome { reply, action: action(note, op == "start" && !running), more: Vec::new() }
            }
        }
    }
}

/// The command is a Java build (or the folder is a Java project): it gets Outpost's Java.
fn wants_java(dir: &Path, command: &str) -> bool {
    let program = program_of(command).unwrap_or_default();
    ["java", "javac", "jar", "gradle", "gradlew", "mvn", "mvnw"].contains(&program.as_str())
        || ["gradlew", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "pom.xml", "src"]
            .iter()
            .any(|f| dir.join(f).exists())
}

/// Files a build just made, worth downloading: `.jar`s (not sources or javadoc ones) in
/// `build/libs` or `target`, under `dir`, changed since `since`. Paths are inside `root`.
pub fn built_files(root: &Path, dir: &Path, since: std::time::SystemTime) -> Vec<String> {
    let mut found = Vec::new();
    let mut look = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = look.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if depth < 4 && !matches!(name.as_str(), ".git" | ".gradle" | "node_modules" | "src") {
                    look.push((p, depth + 1));
                }
                continue;
            }
            let parent = d.to_string_lossy().replace('\\', "/");
            let in_output = parent.ends_with("build/libs") || parent.ends_with("/target") || parent == "target";
            let wanted = name.ends_with(".jar") && !name.ends_with("-sources.jar") && !name.ends_with("-javadoc.jar") && !name.ends_with("-plain.jar");
            let fresh = e.metadata().and_then(|m| m.modified()).is_ok_and(|m| m >= since);
            if in_output && wanted && fresh {
                if let Ok(rel) = p.strip_prefix(root) {
                    found.push(slash(rel));
                }
            }
        }
    }
    found.sort();
    found.truncate(10);
    found
}

fn seconds(s: u64) -> String {
    if s < 90 {
        format!("{s} s")
    } else {
        format!("{} min {} s", s / 60, s % 60)
    }
}

/// Which commands run without asking, per folder (kept on the server, so it's the same from
/// every device): the programs the person said "Always allow" to.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CommandRules {
    #[serde(default)]
    pub allow: Vec<String>,
}

impl CommandRules {
    fn key(folder_id: &str) -> String {
        format!("commands.{folder_id}")
    }

    pub async fn load(state: &AppState, folder_id: &str) -> Self {
        outpost_core::settings::get(&state.pool, &Self::key(folder_id))
            .await
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }

    pub async fn save(&self, state: &AppState, folder_id: &str) -> Result<(), String> {
        let v = serde_json::to_string(self).map_err(|e| e.to_string())?;
        outpost_core::settings::set(&state.pool, &Self::key(folder_id), &v).await.map_err(|e| e.to_string())
    }

    fn with(mut self, program: &str) -> Self {
        if !self.allow.iter().any(|p| p == program) {
            self.allow.push(program.to_string());
        }
        self
    }

    /// Runs without asking: one plain command whose program is allowed.
    pub fn allows(&self, command: &str) -> bool {
        !chained(command) && program_of(command).is_some_and(|p| self.allow.iter().any(|a| a == &p))
    }
}

/// The program a command runs, as "Always allow" remembers it: `./gradlew build` → `gradlew`.
pub fn program_of(command: &str) -> Option<String> {
    let first = command.split_whitespace().next()?;
    let name = first.trim_matches(['"', '\'']).rsplit(['/', '\\']).next()?.to_lowercase();
    let name = name
        .strip_suffix(".bat")
        .or_else(|| name.strip_suffix(".cmd"))
        .or_else(|| name.strip_suffix(".exe"))
        .unwrap_or(&name)
        .to_string();
    (!name.is_empty()).then_some(name)
}

/// More than one command, or output sent somewhere: always asks.
fn chained(command: &str) -> bool {
    ["&&", "||", ";", "|", ">", "<", "`", "$(", "&", "\n"].iter().any(|c| command.contains(c))
}

/// Programs that can do anything: never "always allowed".
const NEVER_ALWAYS: [&str; 16] = [
    "cmd", "powershell", "pwsh", "bash", "sh", "zsh", "start", "del", "erase", "rm", "rmdir", "rd", "format", "curl", "wget", "reg",
];

/// What "Always allow" would allow for this command (`None`: it always asks).
pub fn always_label(command: &str) -> Option<String> {
    if chained(command) {
        return None;
    }
    program_of(command).filter(|p| !NEVER_ALWAYS.contains(&p.as_str()))
}

/// Commands never run, whatever the person says: ones that wipe disks or the system.
pub fn refused(command: &str) -> Option<&'static str> {
    let c = command.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    let bad: [(&str, &str); 11] = [
        ("format ", "formats a drive"),
        ("diskpart", "changes disks"),
        ("rm -rf /", "deletes everything"),
        ("rm -rf ~", "deletes everything"),
        ("del /s", "deletes folders of files"),
        ("rd /s", "deletes folders"),
        ("rmdir /s", "deletes folders"),
        ("shutdown", "turns the PC off"),
        ("reg delete", "changes the registry"),
        ("cipher /w", "wipes a drive"),
        ("mkfs", "formats a drive"),
    ];
    bad.iter().find(|(p, _)| c.contains(p)).map(|(_, why)| *why)
}

/// A command's output for the model: all of it when short; else the lines that look like
/// problems, then the end.
pub fn summarise_output(output: &str) -> String {
    let output = output.trim();
    if output.is_empty() {
        return "(No output.)".into();
    }
    if output.chars().count() <= OUTPUT_LIMIT {
        return output.to_string();
    }
    let lines: Vec<&str> = output.lines().collect();
    let problem = |l: &&&str| {
        let l = l.to_lowercase();
        l.contains("error") || l.contains("exception") || l.contains("failed") || l.contains("warning:") || l.starts_with("e: ")
            || l.contains("cannot find symbol") || l.contains("caused by")
    };
    let problems: Vec<&str> = lines.iter().filter(problem).take(60).copied().collect();
    let tail: Vec<&str> = lines.iter().rev().take(40).rev().copied().collect();
    let mut out = format!("(Long output, {} lines: the problem lines, then the end.)\n", lines.len());
    if !problems.is_empty() {
        out.push_str(&problems.join("\n"));
        out.push_str("\n...\n");
    }
    out.push_str(&tail.join("\n"));
    if out.chars().count() > OUTPUT_LIMIT {
        let keep: String = out.chars().rev().take(OUTPUT_LIMIT).collect::<Vec<_>>().into_iter().rev().collect();
        out = format!("(trimmed)\n{keep}");
    }
    out
}

pub(crate) fn fail(op: &str, path: &str, why: String) -> Outcome {
    Outcome {
        reply: format!("That didn't work: {why}"),
        action: FileAction { op: op.into(), path: path.into(), note: Some(why), change_id: None, failed: true },
        more: Vec::new(),
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
    if file.is_dir() {
        return Err(format!("{} is a folder, so it can't be written as a file (skip this change).", slash(inner)));
    }
    if let Some(dir) = file.parent() {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            return Err(if e.kind() == std::io::ErrorKind::AlreadyExists || e.raw_os_error() == Some(20) {
                format!("Part of {} is a file, not a folder, so it can't go there (skip this change).", slash(inner))
            } else {
                e.to_string()
            });
        }
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
    #[tokio::test]
    async fn a_file_cant_go_where_a_folder_is() {
        let root = std::env::temp_dir().join(format!("shape-{}", uuid::Uuid::new_v4()));
        super::write_server(&root, std::path::Path::new("Hello/src/App.java"), "class App {}").await.unwrap();
        let e = super::write_server(&root, std::path::Path::new("Hello"), "oops").await.unwrap_err();
        assert!(e.contains("is a folder"), "{e}");
        let e = super::write_server(&root, std::path::Path::new("Hello/src/App.java/x.txt"), "oops").await.unwrap_err();
        assert!(e.contains("is a file"), "{e}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_builds_jars_are_offered() {
        let root = std::env::temp_dir().join(format!("built-{}", uuid::Uuid::new_v4()));
        let before = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
        for f in ["build/libs/Shop-1.0.jar", "build/libs/Shop-1.0-sources.jar", "api/target/api.jar", "lib/old.jar", "build/libs/notes.txt"] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        let got = super::built_files(&root, &root, before);
        assert_eq!(got, vec!["api/target/api.jar".to_string(), "build/libs/Shop-1.0.jar".to_string()]);
        // Older than the build: not offered.
        assert!(super::built_files(&root, &root, std::time::SystemTime::now() + std::time::Duration::from_secs(60)).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn commands_ask_unless_allowed_and_some_never_run() {
        assert_eq!(program_of("./gradlew build").as_deref(), Some("gradlew"));
        assert_eq!(program_of("gradlew.bat build --info").as_deref(), Some("gradlew"));
        assert_eq!(program_of(".\\gradlew clean build").as_deref(), Some("gradlew"));
        let rules = CommandRules { allow: vec!["gradlew".into()] };
        assert!(rules.allows("gradlew build"));
        assert!(rules.allows("./gradlew test --info"));
        assert!(!rules.allows("mvn package"));
        // Chained or redirected: always asks, and can't be "always allowed".
        assert!(!rules.allows("gradlew build && del x"));
        assert_eq!(always_label("gradlew build > log.txt"), None);
        assert_eq!(always_label("powershell -c x"), None);
        assert_eq!(always_label("npm test").as_deref(), Some("npm"));
        assert!(refused("format C: /q").is_some());
        assert!(refused("rd  /S /Q build").is_some());
        assert!(refused("gradlew build").is_none());
    }

    #[test]
    fn long_output_keeps_the_problems_and_the_end() {
        let mut log = String::new();
        for i in 0..3000 {
            log.push_str(&format!("line {i} compiling\n"));
            if i == 1500 {
                log.push_str("Main.java:12: error: cannot find symbol\n");
            }
        }
        log.push_str("BUILD FAILED in 4s\n");
        let s = summarise_output(&log);
        assert!(s.contains("cannot find symbol"));
        assert!(s.ends_with("BUILD FAILED in 4s"));
        assert!(s.chars().count() <= OUTPUT_LIMIT + 20);
        assert_eq!(summarise_output("ok"), "ok");
    }

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
