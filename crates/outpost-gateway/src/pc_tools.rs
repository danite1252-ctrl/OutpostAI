//! Tools on the user's PC: MCP servers the Outpost desktop app runs there (web search,
//! GitHub, a browser...). The app sends the tools of the servers this chat uses with each
//! message; each call the model makes goes back to that app (a `file_request` event with
//! `op: "mcp"`, like PC folders), which runs it and answers. Phones don't send any.

use std::time::Duration;

use axum::response::sse::Event;
use outpost_core::chat_files::FileAction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use utoipa::ToSchema;

use crate::workspace::{fail, Outcome, Relay};

/// Most tool servers one message may bring, and most tools in all (each costs context).
pub const MAX_SERVERS: usize = 8;
pub const MAX_TOOLS: usize = 80;
/// Longest a tool on the PC may take (a browser loading pages, a search).
const CALL_WITHIN: Duration = Duration::from_secs(180);
/// Longest to wait for a yes or no.
/// How long the person has to OK a routine.
const ROUTINE_WITHIN: Duration = Duration::from_secs(5 * 60);
const CONFIRM_WITHIN: Duration = Duration::from_secs(60);
/// How long a typed chat waits for Allow or Skip.
const ASK_WITHIN: Duration = Duration::from_secs(5 * 60);
/// Most of a tool's answer the model reads.
const ANSWER_LIMIT: usize = 16_000;

/// One MCP server on the PC, with its tools (as MCP's `tools/list` gives them).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct PcToolServer {
    /// The app's id for it.
    pub id: String,
    /// Its name, e.g. "GitHub".
    pub name: String,
    #[serde(default)]
    pub tools: Vec<PcTool>,
    /// A shared tool's actions this person may not use (its owner's choice): the model is
    /// told, so it says so rather than trying.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub withheld: Vec<String>,
    /// Whose choice that was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withheld_by: Option<String>,
    /// Actions its owner set to ask the person first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ask_first: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PcTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// JSON Schema for its arguments.
    #[serde(default, rename = "inputSchema", alias = "input_schema")]
    #[schema(value_type = Object)]
    pub input_schema: Value,
    /// It only reads (changes nothing), as its server says.
    #[serde(default)]
    pub read_only: Option<bool>,
}

/// Where a tool server runs: on the app streaming the reply (desktop), on your PC that offers
/// it for chats from elsewhere (see [`crate::device_tools`]), or on this server (see
/// [`crate::server_tools`]).
#[derive(Clone)]
pub enum Via {
    App,
    Device { tools: crate::device_tools::DeviceTools, session_id: String },
    /// On this server, for this person (what they may use of it is checked on each call).
    Server { state: crate::AppState, who: crate::server_tools::Who },
    /// Outpost's own tools (making routines), for this person.
    Outpost(crate::outpost_tools::Context),
}

struct Entry {
    /// The name the model calls it by: `<server>__<tool>`.
    call: String,
    server: usize,
    tool: String,
}

/// A message's PC tools.
pub struct PcTools {
    relay: Relay,
    chat_id: String,
    events: mpsc::Sender<Event>,
    servers: Vec<PcToolServer>,
    /// Where each of `servers` runs.
    routes: Vec<Via>,
    entries: Vec<Entry>,
    /// Ask the user before each call (hands-free: speech can be misheard).
    confirm: bool,
    /// A routine's run: nobody is there to ask, so what must ask first isn't done.
    unattended: bool,
}

/// A tool's name as words: `HassTurnOff` → "turn off", `search_files` → "search files".
pub fn tool_words(name: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let boundary = c == '_' || c == '-' || c == '.' || c == ' ';
        let upper_start = c.is_uppercase() && i > 0 && (chars[i - 1].is_lowercase() || chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
        if boundary || upper_start {
            if !cur.is_empty() {
                words.push(cur.to_lowercase());
                cur.clear();
            }
            if boundary {
                continue;
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur.to_lowercase());
    }
    // Home Assistant's tools all start "Hass".
    if words.first().is_some_and(|w| w == "hass") && words.len() > 1 {
        words.remove(0);
    }
    words.join(" ")
}

/// The arguments, briefly, for saying aloud ("kitchen lamp, 50").
pub fn args_words(args: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(o) = args.as_object() {
        for v in o.values() {
            match v {
                Value::String(s) if !s.trim().is_empty() => parts.push(s.trim().chars().take(60).collect()),
                Value::Number(n) => parts.push(n.to_string()),
                Value::Bool(true) => {}
                Value::Array(a) => parts.extend(a.iter().filter_map(Value::as_str).take(3).map(str::to_string)),
                _ => {}
            }
        }
    }
    let s = parts.join(", ");
    if s.chars().count() > 140 {
        format!("{}…", s.chars().take(140).collect::<String>())
    } else {
        s
    }
}

/// A call that only looks something up (its server says so, or its name does).
pub fn reads(tool: &str, read_only: Option<bool>) -> bool {
    let words = tool_words(tool);
    read_only == Some(true)
        || ["get", "list", "search", "read", "find", "fetch", "query", "lookup", "events"].iter().any(|w| words.split(' ').next() == Some(*w) || words.ends_with(w))
}

/// A call on something that always asks first, everywhere: a lock, an alarm, a door, gate or
/// garage, or a cover (blinds, shutters). Its kind, or `None`.
pub fn sensitive(tool: &str, args: &Value) -> Option<&'static str> {
    let text = format!("{} {}", tool_words(tool), args).to_lowercase();
    let words: Vec<&str> = text.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    if has(&["lock", "locks", "unlock", "unlocks", "deadbolt"]) {
        Some("A lock")
    } else if has(&["alarm", "alarms", "alarm_control_panel", "disarm", "arm"]) {
        Some("An alarm")
    } else if has(&["door", "doors", "gate", "gates", "garage"]) {
        Some("A door or gate")
    } else if has(&["cover", "covers", "shutter", "shutters", "blind", "blinds"]) {
        Some("A cover or blind")
    } else {
        None
    }
}

/// The question asked before a call: "Would you like me to check Google Calendar?".
pub fn confirm_question(server: &str, tool: &str, args: &Value, read_only: Option<bool>) -> String {
    let what = args_words(args);
    let words = tool_words(tool);
    if reads(tool, read_only) {
        if what.is_empty() {
            format!("Would you like me to check {server}?")
        } else {
            format!("Would you like me to check {server} for {what}?")
        }
    } else if what.is_empty() {
        format!("Would you like me to {words} with {server}?")
    } else {
        format!("Would you like me to {words}: {what}, with {server}?")
    }
}

/// Letters, digits, `_` and `-` only (what models and Ollama accept in a tool's name).
fn slug(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' })
        .collect();
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "tool".into()
    } else {
        out
    }
}

/// Smart-home calls go wrong when a model guesses a name or sends only a room (which acts on
/// everything in it: the lamp *and* the air conditioner).
const HOME_ASSISTANT_NOTE: &str = "\n\nHome Assistant: to switch or change a device, pass its exact name as Home \
    Assistant lists it (call GetLiveContext first to see the devices, their names and areas, unless this chat already \
    did). Pass the device's name, not just a room or area: a room alone acts on every device in it. Only send an area \
    (with no name) when the user asked for everything in that room. If two devices could match (\"kitchen light\" and \
    \"kitchen lamp\"), ask which.";

impl PcTools {
    /// `None` when there are no tools to offer.
    pub fn new(relay: Relay, chat_id: &str, events: mpsc::Sender<Event>, servers: Vec<(PcToolServer, Via)>, confirm: bool) -> Option<Self> {
        let (servers, routes): (Vec<PcToolServer>, Vec<Via>) =
            servers.into_iter().filter(|(s, _)| !s.tools.is_empty()).take(MAX_SERVERS).unzip();
        let mut entries: Vec<Entry> = Vec::new();
        for (i, s) in servers.iter().enumerate() {
            let prefix: String = slug(&s.name).chars().take(20).collect();
            for t in &s.tools {
                if entries.len() >= MAX_TOOLS {
                    break;
                }
                let base: String = format!("{prefix}__{}", slug(&t.name)).chars().take(60).collect();
                let mut call = base.clone();
                let mut n = 2;
                while entries.iter().any(|e| e.call == call) {
                    call = format!("{base}{n}");
                    n += 1;
                }
                entries.push(Entry { call, server: i, tool: t.name.clone() });
            }
        }
        (!entries.is_empty()).then(|| Self { relay, chat_id: chat_id.into(), events, servers, routes, entries, confirm, unattended: false })
    }

    /// The system prompt's part about them.
    pub fn instructions(&self) -> String {
        let outpost = self.servers.iter().any(|s| s.id == crate::outpost_tools::SERVER_ID);
        let names: Vec<&str> = self.servers.iter().filter(|s| s.id != crate::outpost_tools::SERVER_ID).map(|s| s.name.as_str()).collect();
        // Only Outpost's own: just how to use them.
        if names.is_empty() && outpost {
            return format!("## Tools{}", crate::outpost_tools::NOTE.trim_end());
        }
        format!(
            "## Tools\nYou can use tools from: {}. Use them when they help answer (to look things up, check facts or \
             act for the user), and say what you found or did. Don't make up a tool's results; if one fails, say so. \
             Each time the user asks for something to be done (even the same thing again, or undoing it: \"turn it back \
             on\"), call the tool again in this reply: earlier calls don't carry over, and nothing happens unless you call \
             a tool now. Never say something was done unless a tool call in this reply did it. When acting on \
             something (a device, a file, a message), name exactly that one thing; if you aren't sure which one the user \
             means, look it up with a tool or ask, rather than guessing.{}",
            names.join(", "),
            if self.has_home_assistant() { HOME_ASSISTANT_NOTE } else { "" }
        ) + &self.withheld_note()
            + if outpost { crate::outpost_tools::NOTE } else { "" }
    }

    /// The actions of shared tools this person may not use, so the model says so.
    fn withheld_note(&self) -> String {
        let lines: Vec<String> = self
            .servers
            .iter()
            .filter(|s| !s.withheld.is_empty())
            .map(|s| {
                let words: Vec<String> = s.withheld.iter().take(30).map(|w| tool_words(w)).collect();
                format!(
                    "- {}: not allowed for this user: {} ({} chose this). If they ask for one of these, say it isn't \
                     allowed for them and they can ask {}; don't try another way.",
                    s.name,
                    words.join(", "),
                    s.withheld_by.as_deref().unwrap_or("its owner"),
                    s.withheld_by.as_deref().unwrap_or("its owner"),
                )
            })
            .collect();
        if lines.is_empty() {
            String::new()
        } else {
            format!("\n\nSome actions are switched off for this user:\n{}", lines.join("\n"))
        }
    }

    /// Home Assistant's tools are here (their names start "Hass", plus GetLiveContext).
    fn has_home_assistant(&self) -> bool {
        self.servers.iter().any(|s| s.tools.iter().any(|t| t.name.starts_with("Hass") || t.name == "GetLiveContext"))
    }

    /// The tool definitions for Ollama.
    pub fn tools(&self) -> Vec<Value> {
        self.entries
            .iter()
            .map(|e| {
                let s = &self.servers[e.server];
                let t = s.tools.iter().find(|t| t.name == e.tool).expect("entry's tool");
                let mut params = t.input_schema.clone();
                if !params.is_object() {
                    params = json!({ "type": "object", "properties": {} });
                }
                if params.get("type").is_none() {
                    params["type"] = json!("object");
                }
                let description: String = format!("({}) {}", s.name, t.description.trim()).chars().take(1200).collect();
                json!({ "type": "function", "function": { "name": e.call, "description": description, "parameters": params } })
            })
            .collect()
    }

    /// For a routine's run (nobody watching).
    pub fn unattended(mut self, on: bool) -> Self {
        self.unattended = on;
        self
    }

    pub fn has(&self, call: &str) -> bool {
        self.entries.iter().any(|e| e.call == call)
    }

    /// Run one call on the PC.
    pub async fn run(&self, call: &str, args: &Value) -> Outcome {
        let Some(e) = self.entries.iter().find(|e| e.call == call) else {
            return fail("mcp", call, format!("There's no tool called {call}."));
        };
        let server = &self.servers[e.server];
        let shown = format!("{} › {}", server.name, e.tool);
        let args = if args.is_object() { args.clone() } else { json!({}) };
        // Ask first: hands-free (speech can be misheard), what its owner set to ask, and anything
        // sensitive (locks, alarms, doors, covers), everywhere. Look-ups don't ask (unless hands-free).
        let read_only = server.tools.iter().find(|t| t.name == e.tool).and_then(|t| t.read_only);
        let outpost_own = matches!(self.routes[e.server], Via::Outpost(_));
        let why = if reads(&e.tool, read_only) || outpost_own {
            None
        } else if let Some(kind) = sensitive(&e.tool, &args) {
            Some(format!("{kind} always asks first"))
        } else if server.ask_first.iter().any(|a| a == &e.tool) {
            Some("set to ask first".to_string())
        } else {
            None
        };
        if why.is_some() || self.confirm {
            if self.unattended {
                let why = why.unwrap_or_default();
                return Outcome {
                    reply: format!(
                        "Not done: this needs the user's OK first ({why}), and a routine runs with nobody there to give it. \
                         Say in your answer what you'd have done, so they can do it themselves."
                    ),
                    action: FileAction { op: "mcp".into(), path: shown, note: Some("needs an OK (not in routines)".into()), change_id: None, failed: true },
                    more: Vec::new(),
                };
            }
            let question = match &self.routes[e.server] {
                // A routine: say what it is and when.
                Via::Outpost(ctx) if e.tool == "make_routine" => match crate::outpost_tools::wanted(ctx, &args) {
                    Ok(w) => format!("Shall I make the routine {}?", w.summary),
                    Err(_) => confirm_question(&server.name, &e.tool, &args, read_only),
                },
                _ => confirm_question(&server.name, &e.tool, &args, read_only),
            };
            let asked = json!({
                "op": "confirm", "kind": "tool", "question": question, "server": server.name, "tool": e.tool,
                "arguments": args, "why": why,
            });
            let within = if self.confirm { CONFIRM_WITHIN } else { ASK_WITHIN };
            let answer = self.relay.ask_within(&self.events, &self.chat_id, asked, within, "confirm").await;
            let ok = answer.as_ref().is_ok_and(|v| v.get("ok").and_then(Value::as_bool) == Some(true));
            if !ok {
                let cannot = answer.as_ref().is_ok_and(|v| v.get("cannot_ask").and_then(Value::as_bool) == Some(true));
                let (reply, note) = if cannot {
                    ("Not done: this needs the user's OK, and the app they're using can't ask for it (it may need updating). Tell them.", "couldn't ask")
                } else if self.confirm {
                    ("The user said no to this (or didn't answer), so it wasn't done. Speech can be misheard: briefly ask what they'd like instead.", "Not confirmed")
                } else if answer.is_err() {
                    ("Not done: nobody answered the request to OK it. Tell the user it needs their OK.", "no answer")
                } else {
                    ("The user chose Skip, so it wasn't done. Don't try it again unless they ask.", "skipped")
                };
                return Outcome {
                    reply: reply.into(),
                    action: FileAction { op: "mcp".into(), path: shown, note: Some(note.into()), change_id: None, failed: true },
                    more: Vec::new(),
                };
            }
        }
        if let Via::Outpost(ctx) = &self.routes[e.server] {
            return self.outpost(ctx, &e.tool, &args, shown).await;
        }
        let request = json!({ "op": "mcp", "server": server.id, "tool": e.tool, "arguments": args.clone() });
        let answer = match &self.routes[e.server] {
            Via::App => self.relay.ask_within(&self.events, &self.chat_id, request, CALL_WITHIN, "file_request").await,
            Via::Device { tools, session_id } => tools.call(session_id, request, CALL_WITHIN).await,
            Via::Outpost(_) => unreachable!("handled above"),
            Via::Server { state, who } => match tokio::time::timeout(CALL_WITHIN, crate::server_tools::call(state, &server.id, &e.tool, args, who)).await {
                Ok(Ok(r)) => Ok(json!({ "text": r.text, "is_error": r.is_error })),
                Ok(Err(err)) => Err(err.trim_start_matches("needs sign-in: ").to_string()),
                Err(_) => Err("The tool took too long.".into()),
            },
        };
        match answer {
            Ok(v) => {
                let text = v.get("text").and_then(Value::as_str).unwrap_or_default();
                let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let mut reply: String = text.chars().take(ANSWER_LIMIT).collect();
                if text.chars().count() > ANSWER_LIMIT {
                    reply.push_str("\n[... the rest was cut off]");
                }
                if reply.trim().is_empty() {
                    reply = "(no output)".into();
                }
                let note = if is_error { Some(text.chars().take(200).collect::<String>()) } else { None };
                Outcome {
                    reply: if is_error { format!("The tool reported an error: {reply}") } else { reply },
                    action: FileAction { op: "mcp".into(), path: shown, note, change_id: None, failed: is_error },
                    more: Vec::new(),
                }
            }
            Err(why) => fail("mcp", &shown, why),
        }
    }
}

impl PcTools {
    /// Outpost's own tools: a routine is made only once the person says yes (in the chat, or
    /// aloud when hands-free already asked).
    async fn outpost(&self, ctx: &crate::outpost_tools::Context, tool: &str, args: &Value, shown: String) -> Outcome {
        let done = |reply: String, note: Option<String>, failed: bool| Outcome {
            reply,
            action: FileAction { op: "mcp".into(), path: shown.clone(), note, change_id: None, failed },
            more: Vec::new(),
        };
        match tool {
            "list_routines" => match crate::outpost_tools::list(ctx).await {
                Ok(text) => done(text, None, false),
                Err(why) => done(format!("Couldn't list the routines: {why}"), Some(why), true),
            },
            "make_routine" => {
                let w = match crate::outpost_tools::wanted(ctx, args) {
                    Ok(w) => w,
                    Err(why) => return done(format!("The routine wasn't made: {why} Fix that and call make_routine again."), Some(why), true),
                };
                if !self.confirm {
                    let instructions = w.input.instructions.clone().unwrap_or_default();
                    let asked = json!({
                        "kind": "routine",
                        "question": format!("Make the routine {}?", w.summary),
                        "summary": w.summary,
                        "instructions": instructions,
                    });
                    let answer = self.relay.ask_within(&self.events, &self.chat_id, asked, ROUTINE_WITHIN, "confirm").await;
                    let ok = answer.as_ref().is_ok_and(|v| v.get("ok").and_then(Value::as_bool) == Some(true));
                    if !ok {
                        let cannot = answer.as_ref().is_ok_and(|v| v.get("cannot_ask").and_then(Value::as_bool) == Some(true));
                        let (reply, note) = if cannot {
                            ("The routine wasn't made: this app can't ask the user to OK it (it may need updating). Tell the user they can make it in Routines.", "couldn't ask")
                        } else if answer.is_err() {
                            ("The routine wasn't made: the user didn't answer. Tell them it's waiting for their OK if they ask again.", "no answer")
                        } else {
                            ("The user chose not to make this routine. Don't make it again unless they ask.", "not made")
                        };
                        return done(reply.into(), Some(note.into()), true);
                    }
                }
                match crate::outpost_tools::make(ctx, w).await {
                    Ok(text) => done(text, Some("made".into()), false),
                    Err(why) => done(format!("The routine wasn't made: {why}"), Some(why), true),
                }
            }
            _ => done(format!("There's no Outpost tool called {tool}."), None, true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, tools: &[&str]) -> PcToolServer {
        PcToolServer {
            id: format!("id-{name}"),
            name: name.into(),
            tools: tools
                .iter()
                .map(|t| PcTool { name: (*t).into(), description: format!("Does {t}"), input_schema: Value::Null, read_only: None })
                .collect(),
            withheld: Vec::new(),
            withheld_by: None,
            ask_first: Vec::new(),
        }
    }

    #[test]
    fn pc_tools_get_safe_unique_names_and_valid_schemas() {
        let (tx, _rx) = mpsc::channel(4);
        let none = PcTools::new(Relay::default(), "c", tx.clone(), vec![(server("Empty", &[]), Via::App)], false);
        assert!(none.is_none());
        let t = PcTools::new(
            Relay::default(),
            "c",
            tx,
            vec![(server("Brave Search", &["brave_web_search", "brave.local search"]), Via::App), (server("GitHub", &["search", "search"]), Via::App)],
            false,
        )
        .unwrap();
        let tools = t.tools();
        let names: Vec<&str> = tools.iter().map(|v| v["function"]["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["brave_search__brave_web_search", "brave_search__brave_local_search", "github__search", "github__search2"]);
        assert!(t.has("github__search2"));
        assert_eq!(tools[0]["function"]["parameters"]["type"], "object");
        assert!(tools[2]["function"]["description"].as_str().unwrap().starts_with("(GitHub) "));
        assert!(t.instructions().contains("Brave Search, GitHub"));
        assert!(!t.instructions().contains("GetLiveContext"));
        // Home Assistant gets told to use exact device names, not just a room.
        let (tx, _rx) = mpsc::channel(1);
        let ha = PcTools::new(Relay::default(), "c", tx, vec![(server("Home Assistant", &["HassTurnOff", "GetLiveContext"]), Via::App)], false).unwrap();
        assert!(ha.instructions().contains("call GetLiveContext first"));
        // Actions its owner didn't allow this person: the model is told.
        let (tx, _rx) = mpsc::channel(1);
        let mut limited = server("Home Assistant", &["HassTurnOn"]);
        limited.withheld = vec!["HassUnlock".into()];
        limited.withheld_by = Some("Dan".into());
        let t = PcTools::new(Relay::default(), "c", tx, vec![(limited, Via::App)], false).unwrap();
        assert!(t.instructions().contains("Home Assistant: not allowed for this user: unlock (Dan chose this)"), "{}", t.instructions());
        assert_eq!(t.tools().len(), 1);
    }

    #[tokio::test]
    async fn a_call_goes_to_the_app_and_its_answer_back_to_the_model() {
        let (tx, mut rx) = mpsc::channel(4);
        let relay = Relay::default();
        let t = PcTools::new(relay.clone(), "chat1", tx, vec![(server("Fetch", &["fetch"]), Via::App)], false).unwrap();
        let app = tokio::spawn(async move {
            // The app sees the request (as an SSE event) and answers it.
            let _event = rx.recv().await.unwrap();
            for _ in 0..50 {
                if relay.pending_ids().len() == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let id = relay.pending_ids().pop().unwrap();
            assert!(!relay.answer("someone-else", &id, json!({ "text": "x" })));
            assert!(relay.answer("chat1", &id, json!({ "text": "Page text", "is_error": false })));
        });
        let out = t.run("fetch__fetch", &json!({ "url": "https://example.com" })).await;
        app.await.unwrap();
        assert_eq!(out.reply, "Page text");
        assert_eq!(out.action.path, "Fetch › fetch");
        assert!(!out.action.failed);
        let missing = t.run("nope", &json!({})).await;
        assert!(missing.action.failed);
    }

    #[test]
    fn locks_alarms_doors_and_covers_are_sensitive() {
        assert_eq!(sensitive("HassTurnOff", &json!({ "name": "front door", "domain": ["lock"] })), Some("A lock"));
        assert_eq!(sensitive("HassTurnOn", &json!({ "name": "Garage" })), Some("A door or gate"));
        assert_eq!(sensitive("HassTurnOn", &json!({ "domain": ["alarm_control_panel"] })), Some("An alarm"));
        assert_eq!(sensitive("HassSetPosition", &json!({ "name": "lounge blinds", "position": 50 })), Some("A cover or blind"));
        assert_eq!(sensitive("HassTurnOff", &json!({ "name": "kitchen lamp" })), None);
        assert_eq!(sensitive("HassTurnOn", &json!({ "name": "warm white strip" })), None, "whole words only");
        assert!(reads("GetLiveContext", None));
        assert!(!reads("HassTurnOn", None));
    }

    #[tokio::test]
    async fn sensitive_and_ask_first_calls_wait_for_an_ok_and_routines_dont_do_them() {
        // Typed chat: unlocking asks; Skip means it isn't done.
        let (tx, mut rx) = mpsc::channel(8);
        let relay = Relay::default();
        let t = PcTools::new(relay.clone(), "chat1", tx, vec![(server("Home Assistant", &["HassTurnOff", "GetLiveContext"]), Via::App)], false).unwrap();
        let app = {
            let relay = relay.clone();
            tokio::spawn(async move {
                let ev = format!("{:?}", rx.recv().await.unwrap());
                assert!(ev.contains("confirm") && ev.contains("A lock always asks first"), "{ev}");
                for _ in 0..50 {
                    if relay.pending_ids().len() == 1 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let id = relay.pending_ids().pop().unwrap();
                assert!(relay.answer("chat1", &id, json!({ "ok": false })));
            })
        };
        let out = t.run("home_assistant__hassturnoff", &json!({ "name": "front door", "domain": ["lock"] })).await;
        app.await.unwrap();
        assert!(out.action.failed && out.reply.contains("Skip"), "{}", out.reply);

        // A routine: never done, nobody asked.
        let (tx, _rx) = mpsc::channel(8);
        let mut ha = server("Home Assistant", &["HassTurnOn"]);
        ha.ask_first = vec!["HassTurnOn".into()];
        let t = PcTools::new(Relay::default(), "chat2", tx, vec![(ha, Via::App)], false).unwrap().unattended(true);
        let out = t.run("home_assistant__hassturnon", &json!({ "name": "heater" })).await;
        assert!(out.action.failed && out.reply.contains("set to ask first") && out.reply.contains("routine"), "{}", out.reply);
    }

    #[test]
    fn questions_read_naturally() {
        assert_eq!(tool_words("HassTurnOff"), "turn off");
        assert_eq!(tool_words("calendar_events"), "calendar events");
        assert_eq!(tool_words("GetHTMLPage"), "get html page");
        assert_eq!(confirm_question("Google", "calendar_events", &json!({}), None), "Would you like me to check Google?");
        assert_eq!(
            confirm_question("Home Assistant", "HassTurnOff", &json!({ "name": "kitchen lamp" }), None),
            "Would you like me to turn off: kitchen lamp, with Home Assistant?"
        );
        assert_eq!(
            confirm_question("DuckDuckGo Search", "search", &json!({ "query": "weather Sydney" }), Some(true)),
            "Would you like me to check DuckDuckGo Search for weather Sydney?"
        );
    }

    #[tokio::test]
    async fn hands_free_asks_first_and_a_no_stops_it() {
        let (tx, mut rx) = mpsc::channel(4);
        let relay = Relay::default();
        let t = PcTools::new(relay.clone(), "chat1", tx, vec![(server("Home Assistant", &["HassTurnOff"]), Via::App)], true).unwrap();
        let app = tokio::spawn(async move {
            // The question comes first; the user says no.
            let ev = rx.recv().await.unwrap();
            let _ = ev;
            for _ in 0..50 {
                if relay.pending_ids().len() == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let id = relay.pending_ids().pop().unwrap();
            assert!(relay.answer("chat1", &id, json!({ "ok": false })));
            // Every device is told it's settled ("answered"), and nothing else is asked.
            assert!(tokio::time::timeout(Duration::from_millis(500), rx.recv()).await.is_ok());
            assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv()).await.is_err());
        });
        let out = t.run("home_assistant__hassturnoff", &json!({ "name": "lamp" })).await;
        app.await.unwrap();
        assert!(out.action.failed);
        assert_eq!(out.action.note.as_deref(), Some("Not confirmed"));
        assert!(out.reply.contains("said no"));
    }
}
